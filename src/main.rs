//! Pincers — a segmented, multi-connection download manager in Rust,
//! modelled on the download engine of Internet Download Manager.

mod bridge;
mod chunks;
mod engine;
mod gui;
mod i18n;
mod limiter;
mod probe;
mod state;

use anyhow::{Result, bail};
use clap::{CommandFactory, FromArgMatches, Parser};
use engine::{Config, Progress};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::watch;

// The help texts come from the translations (see `parse_args`).
#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    // Explicit positions: `mut_arg` (used to translate the help) re-inserts
    // the argument last, which would otherwise make `url` the second positional.
    #[arg(index = 1)]
    url: Option<String>,
    /// Extra URLs for the window (`pincers gui <URL>...`).
    #[arg(index = 2, hide = true)]
    more: Vec<String>,
    #[arg(short, long, default_value_t = 8)]
    connections: usize,
    #[arg(short, long, default_value = ".")]
    output: PathBuf,
    #[arg(short, long)]
    limit: Option<String>,
    #[arg(long, default_value_t = chunks::MIN_SEGMENT)]
    min_segment: u64,
}

/// Parses the command line with `--help` in the user's language.
fn parse_args() -> Args {
    Args::from_arg_matches(&command().get_matches()).unwrap_or_else(|e| e.exit())
}

fn command() -> clap::Command {
    Args::command()
        .about(tr!("cli.about"))
        .mut_arg("url", |a| a.help(tr!("cli.url")))
        .mut_arg("connections", |a| a.help(tr!("cli.connections")))
        .mut_arg("output", |a| a.help(tr!("cli.output")))
        .mut_arg("limit", |a| a.help(tr!("cli.limit")))
        .mut_arg("min_segment", |a| a.help(tr!("cli.min_segment")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(argv: &[&str]) -> Args {
        Args::from_arg_matches(&command().try_get_matches_from(argv).unwrap()).unwrap()
    }

    /// Translating the help must not move the positionals around.
    #[test]
    fn gui_with_urls_opens_the_window() {
        let a = parse(&["pincers", "gui", "https://a/x.zip", "https://b/y.iso"]);
        assert_eq!(a.url.as_deref(), Some("gui"));
        assert_eq!(a.more, ["https://a/x.zip", "https://b/y.iso"]);
        let b = parse(&["pincers", "https://a/x.zip", "-c", "4"]);
        assert_eq!(b.url.as_deref(), Some("https://a/x.zip"));
        assert_eq!(b.connections, 4);
    }
}

fn main() -> Result<()> {
    // Started by the browser extension: speak native messaging on stdio.
    let raw: Vec<String> = std::env::args().collect();
    if bridge::is_native_host_invocation(&raw) {
        i18n::set_language(gui::saved_language().as_deref());
        return bridge::run_native_host();
    }

    // Same language as the window: the choice saved in Options, or the system's.
    i18n::set_language(gui::saved_language().as_deref());
    let args = parse_args();
    match args.url.as_deref() {
        #[cfg(windows)]
        Some("register") => {
            for key in bridge::register(bridge::CHROME_EXTENSION_ID, bridge::FIREFOX_ADDON_ID)? {
                println!("✔ {key}");
            }
            println!("{}", tr!("cli.registered"));
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
                "{}",
                tr!(
                    "cli.done",
                    path = path.display(),
                    size = human(size as f64),
                    secs = format!("{secs:.1}"),
                    speed = human(size as f64 / secs.max(0.001)),
                )
            );
            Ok(())
        }
        Err(e) if cancel.load(Ordering::Relaxed) => {
            println!("{}", tr!("cli.paused", error = e));
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
        bar.set_prefix(tr!("cli.progress", speed = human(p.speed), eta = eta, n = p.connections));
        bar.set_message(format!("{}  [{}]\n{}", p.file_name, p.status.text(), segment_map(&p, 60)));
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
        _ => bail!(tr!("cli.bad_limit", value = s)),
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
