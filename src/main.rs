//! Pincers — a segmented, multi-connection download manager in Rust,
//! modelled on the download engine of Internet Download Manager.

mod bridge;
mod chunks;
mod engine;
mod gui;
mod limiter;
mod probe;
mod state;

use anyhow::{Result, bail};
use clap::Parser;
use engine::{Config, Progress};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::watch;

/// 🦀 Pincers: baixa arquivos em vários segmentos paralelos, como o IDM.
/// Sem URL (ou com `gui`), abre a interface gráfica.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// URL do arquivo (omita, ou use `gui`, para abrir a janela;
    /// `register` liga o Pincers às extensões do navegador)
    url: Option<String>,
    /// URLs extras para adicionar à janela (`pincers gui <URL>...`)
    #[arg(hide = true)]
    more: Vec<String>,
    /// Número máximo de conexões simultâneas (IDM usa 8 por padrão)
    #[arg(short, long, default_value_t = 8)]
    connections: usize,
    /// Pasta de destino
    #[arg(short, long, default_value = ".")]
    output: PathBuf,
    /// Limite de velocidade, ex.: 500K, 2M
    #[arg(short, long)]
    limit: Option<String>,
    /// Tamanho mínimo de segmento em bytes (IDM: 11680)
    #[arg(long, default_value_t = chunks::MIN_SEGMENT)]
    min_segment: u64,
}

fn main() -> Result<()> {
    // Started by the browser extension: speak native messaging on stdio.
    let raw: Vec<String> = std::env::args().collect();
    if bridge::is_native_host_invocation(&raw) {
        return bridge::run_native_host();
    }

    let args = Args::parse();
    match args.url.as_deref() {
        #[cfg(windows)]
        Some("register") => {
            for key in bridge::register(bridge::CHROME_EXTENSION_ID, bridge::FIREFOX_ADDON_ID)? {
                println!("✔ {key}");
            }
            println!("Pronto: as extensões do Chrome/Opera/Firefox já conseguem falar com o Pincers.");
            Ok(())
        }
        None => {
            hide_own_console();
            gui::run(Vec::new())
        }
        Some("gui") => {
            hide_own_console();
            gui::run(args.more)
        }
        Some(url) => {
            let url = url.to_owned();
            tokio::runtime::Runtime::new()?.block_on(cli(url, args))
        }
    }
}

/// When started by a double-click, Windows gives us a console of our own;
/// drop it so only the window shows. From a terminal the console is shared
/// with the shell and stays.
fn hide_own_console() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
        let mut pids = [0u32; 2];
        if GetConsoleProcessList(pids.as_mut_ptr(), 2) == 1 {
            FreeConsole();
        }
    }
}

async fn cli(url: String, args: Args) -> Result<()> {
    let cfg = Config {
        connections: args.connections.clamp(1, 32),
        min_segment: args.min_segment.max(1024),
        speed_limit: args.limit.as_deref().map(parse_size).transpose()?,
        out_dir: args.output,
        ..Config::default()
    };

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                cancel.store(true, Ordering::Relaxed);
            }
        });
    }

    let (tx, rx) = watch::channel(Progress::default());
    let ui = tokio::spawn(render(rx));
    let started = std::time::Instant::now();
    let result = engine::download(&url, cfg, tx, cancel.clone()).await;
    let _ = ui.await;

    match result {
        Ok(path) => {
            let secs = started.elapsed().as_secs_f64();
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            println!(
                "✔ {}  ({} em {:.1}s, média {}/s)",
                path.display(),
                human(size as f64),
                secs,
                human(size as f64 / secs.max(0.001))
            );
            Ok(())
        }
        Err(e) if cancel.load(Ordering::Relaxed) => {
            println!("⏸ Pausado. Rode o mesmo comando para retomar. ({e})");
            Ok(())
        }
        Err(e) => Err(e),
    }
}

async fn render(mut rx: watch::Receiver<Progress>) {
    let bar = ProgressBar::new(0);
    bar.set_style(
        ProgressStyle::with_template("{msg}\n{wide_bar:.cyan/blue} {bytes}/{total_bytes}  {prefix}")
            .unwrap()
            .progress_chars("█▓░"),
    );
    loop {
        let changed = tokio::time::timeout(Duration::from_millis(200), rx.changed()).await;
        let p = rx.borrow().clone();
        if let Some(size) = p.size {
            bar.set_length(size);
        }
        bar.set_position(p.downloaded);
        let eta = match p.size {
            Some(s) if p.speed > 0.0 => format_eta((s.saturating_sub(p.downloaded)) as f64 / p.speed),
            _ => "--".into(),
        };
        bar.set_prefix(format!("{}/s  ETA {eta}  {} conexões", human(p.speed), p.connections));
        bar.set_message(format!("{}  [{}]\n{}", p.file_name, p.status, segment_map(&p, 60)));
        if p.finished || matches!(changed, Ok(Err(_))) {
            bar.finish_and_clear();
            return;
        }
    }
}

/// One character per slice of the file, like IDM's "Show download progress" map.
fn segment_map(p: &Progress, width: usize) -> String {
    let Some(size) = p.size.filter(|&s| s > 0) else { return String::new() };
    (0..width)
        .map(|i| {
            let a = size * i as u64 / width as u64;
            let b = size * (i as u64 + 1) / width as u64;
            let mut got = 0;
            let mut active = false;
            for s in &p.segments {
                let lo = a.max(s.start);
                let hi = b.min(s.pos);
                if hi > lo {
                    got += hi - lo;
                }
                active |= s.active && s.pos >= a && s.pos < b;
            }
            if active {
                '▶'
            } else if got >= b - a {
                '█'
            } else if got > 0 {
                '▒'
            } else {
                '·'
            }
        })
        .collect()
}

pub(crate) fn parse_size(s: &str) -> Result<u64> {
    let s = s.trim().to_ascii_uppercase();
    let (num, mult) = match s.chars().last() {
        Some('K') => (&s[..s.len() - 1], 1024.0),
        Some('M') => (&s[..s.len() - 1], 1024.0 * 1024.0),
        Some('G') => (&s[..s.len() - 1], 1024.0 * 1024.0 * 1024.0),
        _ => (&s[..], 1.0),
    };
    match num.parse::<f64>() {
        Ok(n) if n > 0.0 => Ok((n * mult) as u64),
        _ => bail!("limite inválido: {s}"),
    }
}

pub(crate) fn human(b: f64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", U[i])
}

pub(crate) fn format_eta(secs: f64) -> String {
    let s = secs as u64;
    if s >= 3600 { format!("{}h{:02}m", s / 3600, s / 60 % 60) } else { format!("{}m{:02}s", s / 60, s % 60) }
}
