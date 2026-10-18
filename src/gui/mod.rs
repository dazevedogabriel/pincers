//! Pincers window, implemented from the Claude Design mock-up
//! (`Pincers.dc.html`): custom title bar, filter sidebar, download list with
//! per-part bars and a detail panel with the file map, speed history and the
//! parts table. The engine runs on a tokio runtime; the UI only reads `watch`
//! snapshots of each download.

mod theme;
mod views;

use crate::bridge::{self, Command, DownloadRequest};
use crate::engine::{self, Config, Progress, SegmentView};
use crate::{tr, tr_n};
use crate::state::SavedState;
use anyhow::Result;
use eframe::egui::{self, Align2, Color32, CursorIcon, Rect, ResizeDirection, Sense, ViewportCommand, pos2, vec2};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use theme::*;
use tokio::runtime::Runtime;
use tokio::sync::watch;

pub const LOGO_SVG: &[u8] = include_bytes!("../../assets/logo.svg");
const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));
/// The mock-up samples speed every 400 ms and keeps 48 (item) / 32 (total) bars.
const TICK: Duration = Duration::from_millis(400);

pub fn run(initial_urls: Vec<String>) -> Result<()> {
    // Single instance: a window that is already open takes the URLs (or just
    // comes to the front) and this process ends.
    let requests: Vec<DownloadRequest> =
        initial_urls.into_iter().map(|url| DownloadRequest { url, ..Default::default() }).collect();
    let forward = if requests.is_empty() { Command::Show } else { Command::Add { downloads: requests.clone() } };
    if bridge::send(forward).is_ok() {
        return Ok(());
    }
    let listener = bridge::Listener::bind().ok();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Pincers")
        .with_inner_size([1400.0, 820.0])
        .with_min_inner_size([760.0, 520.0])
        .with_decorations(false);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    // Always start centred on the screen (no window position is persisted).
    let options = eframe::NativeOptions { viewport, centered: true, ..Default::default() };
    eframe::run_native(
        "Pincers",
        options,
        Box::new(move |cc| Ok(Box::new(PincersApp::new(cc, requests, listener)))),
    )
    .map_err(|e| anyhow::anyhow!(tr!("window.open_failed", error = e)))
}

/// Rasterises the logo into a square 256 px RGBA icon for the taskbar.
fn window_icon() -> Option<egui::IconData> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(LOGO_SVG, &usvg::Options::default()).ok()?;
    let side = 256u32;
    let mut pix = tiny_skia::Pixmap::new(side, side)?;
    let sz = tree.size();
    let scale = side as f32 / sz.width().max(sz.height());
    let dy = (side as f32 - sz.height() * scale) / 2.0;
    let dx = (side as f32 - sz.width() * scale) / 2.0;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy), &mut pix.as_mut());
    // tiny-skia stores premultiplied alpha; the icon wants straight alpha.
    let rgba = pix
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Some(egui::IconData { rgba, width: side, height: side })
}

// ---------------------------------------------------------------- settings

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum MapView {
    #[default]
    Grade,
    Barras,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub connections: usize,
    pub out_dir: PathBuf,
    pub limit_enabled: bool,
    /// KB/s
    pub limit_kbps: u64,
    pub map_view: MapView,
    /// Language code chosen in Options; `None` follows the system.
    pub language: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        let out_dir = std::env::var_os("USERPROFILE")
            .map(|h| PathBuf::from(h).join("Downloads"))
            .unwrap_or_else(|| PathBuf::from("."));
        Self { connections: 8, out_dir, limit_enabled: false, limit_kbps: 1024, map_view: MapView::Grade, language: None }
    }
}

impl Settings {
    fn config(&self, out_dir: &Path) -> Config {
        Config {
            connections: self.connections.clamp(1, 32),
            speed_limit: self.limit_enabled.then_some(self.limit_kbps.max(1) * 1024),
            out_dir: out_dir.to_owned(),
            ..Config::default()
        }
    }
}

// ---------------------------------------------------------------- downloads

#[derive(Clone, PartialEq)]
pub enum Status {
    Running,
    Paused,
    Done(PathBuf),
    Failed(String),
}

impl Status {
    pub fn label(&self) -> String {
        tr!(match self {
            Status::Running => "status.running",
            Status::Paused => "status.paused",
            Status::Done(_) => "status.done",
            Status::Failed(_) => "status.failed",
        })
    }
    pub fn color(&self) -> Color32 {
        match self {
            Status::Running => ACCENT,
            Status::Paused => AMBER,
            Status::Done(_) => GREEN,
            Status::Failed(_) => RED,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Filter {
    All,
    Running,
    Paused,
    Done,
    Failed,
}

impl Filter {
    pub const ALL: [Filter; 5] = [Filter::All, Filter::Running, Filter::Paused, Filter::Done, Filter::Failed];
    /// Stable identifier (also used as the widget id).
    pub fn key(self) -> &'static str {
        match self {
            Filter::All => "filter.all",
            Filter::Running => "filter.running",
            Filter::Paused => "filter.paused",
            Filter::Done => "filter.done",
            Filter::Failed => "filter.failed",
        }
    }
    pub fn label(self) -> String {
        tr!(self.key())
    }
    pub fn dot(self) -> Color32 {
        match self {
            Filter::All => DIM,
            Filter::Running => ACCENT,
            Filter::Paused => AMBER,
            Filter::Done => GREEN,
            Filter::Failed => RED,
        }
    }
    pub fn matches(self, s: &Status) -> bool {
        match self {
            Filter::All => true,
            Filter::Running => *s == Status::Running,
            Filter::Paused => *s == Status::Paused,
            Filter::Done => matches!(s, Status::Done(_)),
            Filter::Failed => matches!(s, Status::Failed(_)),
        }
    }
}

type Outcome = Arc<Mutex<Option<Result<PathBuf, String>>>>;

pub struct Item {
    pub id: u64,
    pub url: String,
    pub out_dir: PathBuf,
    pub status: Status,
    pub last: Progress,
    rx: Option<watch::Receiver<Progress>>,
    cancel: Arc<AtomicBool>,
    outcome: Outcome,
    pub added_ts: i64,
    pub hist: VecDeque<f64>,
    /// When `status` last changed; see [`Item::settled`].
    pub changed_at: Instant,
    /// Cookies / referrer / user agent captured by the browser extension.
    pub headers: Vec<(String, String)>,
    /// File name suggested by the browser.
    pub suggested_name: Option<String>,
}

impl Item {
    pub fn name(&self) -> String {
        if !self.last.file_name.is_empty() {
            return self.last.file_name.clone();
        }
        let path = self.url.split(['?', '#']).next().unwrap_or(&self.url);
        path.rsplit('/').find(|s| !s.is_empty()).unwrap_or(&self.url).to_owned()
    }
    /// Buttons that change meaning with the status ("Pausar" becomes "Abrir
    /// arquivo" in the same place when a download finishes) ignore clicks for
    /// a moment, so a click aimed at the old button cannot open the file.
    pub fn settled(&self) -> bool {
        self.changed_at.elapsed() >= Duration::from_secs(1)
    }

    pub fn ext(&self) -> String {
        let name = self.name();
        match name.rsplit_once('.') {
            Some((_, e)) if !e.is_empty() => e.chars().take(4).collect::<String>().to_uppercase(),
            _ => "BIN".into(),
        }
    }
    pub fn fraction(&self) -> f64 {
        match (&self.status, self.last.size) {
            (Status::Done(_), _) => 1.0,
            (_, Some(s)) if s > 0 => self.last.downloaded as f64 / s as f64,
            _ => 0.0,
        }
    }
    pub fn active_parts(&self) -> usize {
        self.last.segments.iter().filter(|s| s.active && s.pos < s.end).count()
    }
    pub fn parts_label(&self) -> String {
        let n = self.last.segments.len().max(1);
        if self.status == Status::Running {
            tr_n!("item.parts_active", n, active = self.active_parts())
        } else {
            tr_n!("item.parts", n)
        }
    }
    pub fn size_label(&self) -> String {
        let total = self.last.size.map(|s| size(s as f64)).unwrap_or_else(|| "?".into());
        match self.status {
            Status::Done(_) => total,
            _ => format!("{} / {total}", size(self.last.downloaded as f64)),
        }
    }
    pub fn speed_label(&self) -> String {
        if self.status == Status::Running { format!("{}/s", size(self.last.speed)) } else { "—".into() }
    }
    pub fn eta_label(&self) -> String {
        match (self.status == Status::Running, self.last.size) {
            (true, Some(s)) if self.last.speed > 0.0 => {
                eta(s.saturating_sub(self.last.downloaded) as f64 / self.last.speed)
            }
            _ => "—".into(),
        }
    }
    /// Colour of one part, as in the mock-up's `pcol()`.
    pub fn part_color(&self, s: &SegmentView) -> Color32 {
        if s.failed {
            RED
        } else if s.pos >= s.end || matches!(self.status, Status::Done(_)) {
            GREEN
        } else {
            match self.status {
                Status::Running => ACCENT,
                Status::Paused => AMBER,
                _ => MUTED,
            }
        }
    }
    /// Parts to draw; a finished download without stored parts shows one.
    pub fn parts(&self) -> Vec<SegmentView> {
        if self.last.segments.is_empty() {
            let size = self.last.size.unwrap_or(1);
            let pos = if matches!(self.status, Status::Done(_)) { size } else { self.last.downloaded };
            return vec![SegmentView { start: 0, pos, end: size, active: false, failed: false }];
        }
        self.last.segments.clone()
    }
}

/// What survives a restart of the app; the engine's `.pincers` file keeps
/// the authoritative segment state for unfinished downloads.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Saved {
    settings: Settings,
    items: Vec<SavedItem>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct SavedItem {
    url: String,
    out_dir: PathBuf,
    file_name: String,
    size: Option<u64>,
    downloaded: u64,
    done_path: Option<PathBuf>,
    error: Option<String>,
    added_ts: i64,
    /// (start, pos, end) of each part, for drawing finished downloads.
    parts: Vec<(u64, u64, u64)>,
    /// Needed again on resume: sites behind a login check the cookies.
    headers: Vec<(String, String)>,
    suggested_name: Option<String>,
}

/// Language chosen in Options, read straight from the saved settings (the
/// command line and the native host use it too, without opening the window).
pub fn saved_language() -> Option<String> {
    let data = std::fs::read(saved_path()).ok()?;
    let saved: Saved = serde_json::from_slice(&data).ok()?;
    saved.settings.language
}

fn saved_path() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Pincers")
        .join("pincers.json")
}

fn now_ts() -> i64 {
    chrono::Local::now().timestamp()
}

// ---------------------------------------------------------------- app

pub struct PincersApp {
    rt: Runtime,
    pub settings: Settings,
    pub items: Vec<Item>,
    next_id: u64,
    pub selected: Option<u64>,
    pub filter: Filter,
    pub url_input: String,
    pub show_settings: bool,
    pub total_hist: VecDeque<f64>,
    dirty: bool,
    last_save: Instant,
    last_tick: Instant,
    /// Requests from the browser extension or a second `pincers.exe`.
    inbox: std::sync::mpsc::Receiver<Command>,
}

impl PincersApp {
    fn new(cc: &eframe::CreationContext<'_>, initial: Vec<DownloadRequest>, listener: Option<bridge::Listener>) -> Self {
        let (tx, inbox) = std::sync::mpsc::channel();
        if let Some(listener) = listener {
            let ctx = cc.egui_ctx.clone();
            listener.spawn(move |cmd| {
                let _ = tx.send(cmd);
                ctx.request_repaint();
            });
        }
        theme::install(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let saved: Saved = std::fs::read(saved_path())
            .ok()
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default();
        let mut app = Self {
            rt: tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio"),
            settings: saved.settings,
            items: Vec::new(),
            next_id: 0,
            selected: None,
            filter: Filter::All,
            url_input: String::new(),
            show_settings: false,
            total_hist: VecDeque::from(vec![0.0; 32]),
            dirty: false,
            last_save: Instant::now(),
            last_tick: Instant::now(),
            inbox,
        };
        for s in saved.items {
            let status = match (s.done_path, s.error) {
                (Some(p), _) => Status::Done(p),
                (None, Some(e)) => Status::Failed(e),
                // Anything that was running when the app closed comes back paused.
                (None, None) => Status::Paused,
            };
            let mut item = app.new_item(s.url, s.out_dir, s.added_ts);
            item.last.file_name = s.file_name;
            item.headers = s.headers;
            item.suggested_name = s.suggested_name;
            item.last.size = s.size;
            item.last.downloaded = s.downloaded;
            item.last.segments = s
                .parts
                .into_iter()
                .map(|(start, pos, end)| SegmentView { start, pos, end, active: false, failed: false })
                .collect();
            // Unfinished: the engine's state file has the freshest parts.
            if !matches!(status, Status::Done(_))
                && !item.last.file_name.is_empty()
                && let Some(st) = SavedState::load(&item.out_dir.join(&item.last.file_name))
            {
                item.last.segments = st
                    .segments
                    .iter()
                    .map(|g| SegmentView { start: g.start, pos: g.pos, end: g.end, active: false, failed: false })
                    .collect();
                item.last.downloaded = st.segments.iter().map(|g| g.pos - g.start).sum();
            }
            item.status = status;
            app.items.push(item);
        }
        app.selected = app.items.first().map(|i| i.id);
        for req in initial {
            app.add_request(req, &cc.egui_ctx);
        }
        app
    }

    fn new_item(&mut self, url: String, out_dir: PathBuf, added_ts: i64) -> Item {
        self.next_id += 1;
        Item {
            id: self.next_id,
            url,
            out_dir,
            status: Status::Paused,
            last: Progress::default(),
            rx: None,
            cancel: Arc::new(AtomicBool::new(false)),
            outcome: Arc::new(Mutex::new(None)),
            added_ts,
            hist: VecDeque::new(),
            changed_at: Instant::now(),
            headers: Vec::new(),
            suggested_name: None,
        }
    }

    /// Adds every URL in `text`: the extension copies several links one per
    /// line, which the single-line field receives separated by spaces.
    pub fn add_url(&mut self, text: String, ctx: &egui::Context) {
        for url in split_urls(&text) {
            self.add_request(DownloadRequest { url, ..Default::default() }, ctx);
        }
    }

    /// Adds and starts a download, with the browser context when it came
    /// from the extension.
    pub fn add_request(&mut self, req: DownloadRequest, ctx: &egui::Context) {
        let url = req.url.trim().to_owned();
        if url.is_empty() {
            return;
        }
        let headers = req.headers();
        // A blank destination (hand-edited settings) falls back to Downloads.
        let out_dir = if self.settings.out_dir.as_os_str().is_empty() {
            Settings::default().out_dir
        } else {
            self.settings.out_dir.clone()
        };
        let mut item = self.new_item(url, out_dir, now_ts());
        item.headers = headers;
        item.suggested_name = req.file_name.filter(|n| !n.trim().is_empty());
        let id = item.id;
        // Newest first, like the mock-up.
        self.items.insert(0, item);
        self.selected = Some(id);
        self.filter = Filter::All;
        self.start(id, ctx);
    }

    /// Starts or resumes a download; the engine picks up its `.pincers` state.
    pub fn start(&mut self, id: u64, ctx: &egui::Context) {
        let settings = self.settings.clone();
        let Some(item) = self.items.iter_mut().find(|i| i.id == id) else { return };
        if item.status == Status::Running || matches!(item.status, Status::Done(_)) {
            return;
        }
        let (tx, rx) = watch::channel(Progress {
            file_name: item.last.file_name.clone(),
            size: item.last.size,
            downloaded: item.last.downloaded,
            segments: item.last.segments.clone(),
            ..Progress::default()
        });
        item.cancel = Arc::new(AtomicBool::new(false));
        item.outcome = Arc::new(Mutex::new(None));
        item.rx = Some(rx);
        item.status = Status::Running;
        item.changed_at = Instant::now();

        let url = item.url.clone();
        let mut cfg = settings.config(&item.out_dir);
        cfg.headers = item.headers.clone();
        cfg.file_name = item.suggested_name.clone();
        let (cancel, outcome, ctx) = (item.cancel.clone(), item.outcome.clone(), ctx.clone());
        self.rt.spawn(async move {
            let r = engine::download(&url, cfg, tx, cancel).await;
            *outcome.lock().unwrap() = Some(r.map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
        self.dirty = true;
    }

    pub fn pause(&mut self, id: u64) {
        if let Some(item) = self.items.iter().find(|i| i.id == id && i.status == Status::Running) {
            item.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Row/primary-button action: pause, resume, retry or open.
    pub fn toggle(&mut self, id: u64, ctx: &egui::Context) {
        let Some(item) = self.items.iter().find(|i| i.id == id) else { return };
        if !item.settled() {
            return;
        }
        match &item.status {
            Status::Running => self.pause(id),
            Status::Paused | Status::Failed(_) => self.start(id, ctx),
            Status::Done(p) => open_file(p),
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.pause(id);
        let idx = self.items.iter().position(|i| i.id == id);
        self.items.retain(|i| i.id != id);
        if self.selected == Some(id) {
            // Select the neighbour, like the mock-up's onRemove.
            self.selected = idx.and_then(|i| self.items.get(i.min(self.items.len().saturating_sub(1)))).map(|i| i.id);
        }
        self.dirty = true;
    }

    pub fn pause_all(&mut self) {
        let ids: Vec<_> = self.items.iter().map(|i| i.id).collect();
        ids.into_iter().for_each(|id| self.pause(id));
    }

    pub fn resume_all(&mut self, ctx: &egui::Context) {
        let ids: Vec<_> = self
            .items
            .iter()
            .filter(|i| matches!(i.status, Status::Paused | Status::Failed(_)))
            .map(|i| i.id)
            .collect();
        ids.into_iter().for_each(|id| self.start(id, ctx));
    }

    /// Pulls fresh progress snapshots and finished results from the engine.
    fn poll(&mut self) {
        for item in &mut self.items {
            if let Some(rx) = &item.rx {
                item.last = rx.borrow().clone();
            }
            let Some(result) = item.outcome.lock().unwrap().take() else { continue };
            item.changed_at = Instant::now();
            item.status = match result {
                Ok(path) => Status::Done(path),
                Err(_) if item.cancel.load(Ordering::Relaxed) => Status::Paused,
                Err(e) => Status::Failed(e),
            };
            if let Some(rx) = item.rx.take() {
                item.last = rx.borrow().clone();
            }
            item.last.speed = 0.0;
            item.last.connections = 0;
            for s in &mut item.last.segments {
                s.active = false;
            }
            self.dirty = true;
        }
    }

    /// Speed history sampling (mock-up: every 400 ms).
    fn tick(&mut self) {
        if self.last_tick.elapsed() < TICK {
            return;
        }
        self.last_tick = Instant::now();
        let mut total = 0.0;
        for item in &mut self.items {
            let v = if item.status == Status::Running { item.last.speed } else { 0.0 };
            if v > 0.0 || item.hist.back().is_some_and(|&b| b > 0.0) {
                item.hist.push_back(v);
                while item.hist.len() > 48 {
                    item.hist.pop_front();
                }
            }
            total += v;
        }
        self.total_hist.push_back(total);
        while self.total_hist.len() > 32 {
            self.total_hist.pop_front();
        }
    }

    pub fn total_speed(&self) -> f64 {
        self.items.iter().filter(|i| i.status == Status::Running).map(|i| i.last.speed).sum()
    }

    pub fn active_connections(&self) -> usize {
        self.items.iter().filter(|i| i.status == Status::Running).map(Item::active_parts).sum()
    }

    fn persist(&self) {
        let saved = Saved {
            settings: self.settings.clone(),
            items: self
                .items
                .iter()
                .map(|i| SavedItem {
                    url: i.url.clone(),
                    out_dir: i.out_dir.clone(),
                    file_name: i.last.file_name.clone(),
                    size: i.last.size,
                    downloaded: i.last.downloaded,
                    done_path: match &i.status {
                        Status::Done(p) => Some(p.clone()),
                        _ => None,
                    },
                    error: match &i.status {
                        Status::Failed(e) => Some(e.clone()),
                        _ => None,
                    },
                    added_ts: i.added_ts,
                    parts: i.last.segments.iter().map(|s| (s.start, s.pos, s.end)).collect(),
                    headers: i.headers.clone(),
                    suggested_name: i.suggested_name.clone(),
                })
                .collect(),
        };
        let path = saved_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(data) = serde_json::to_vec_pretty(&saved) {
            let _ = std::fs::write(path, data);
        }
    }

    /// Handles what the extension or a second `pincers.exe` sent.
    fn drain_inbox(&mut self, ctx: &egui::Context) {
        let mut show = false;
        while let Ok(cmd) = self.inbox.try_recv() {
            show = true;
            if let Command::Add { downloads } = cmd {
                for req in downloads {
                    self.add_request(req, ctx);
                }
            }
        }
        if show {
            // Restore and raise the window. Windows may only flash it in the
            // taskbar when another app has the focus.
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }
}

/// Opens a file, folder or program through the Windows shell.
///
/// `ShellExecuteW` is used instead of spawning `cmd /C start` or `explorer`:
/// when Pincers is started by a double-click it drops its console, and from
/// then on `Command::spawn` fails with OS error 50 because the child cannot
/// inherit the (now invalid) standard handles.
#[cfg(windows)]
fn shell_open(target: &std::ffi::OsStr, params: Option<&str>) -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &OsStr| s.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (op, file) = (wide(OsStr::new("open")), wide(target));
    let params = params.map(|p| wide(OsStr::new(p)));
    let params_ptr = params.as_ref().map_or(std::ptr::null(), |p| p.as_ptr());
    let r = unsafe {
        ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), params_ptr, std::ptr::null(), SW_SHOWNORMAL)
    };
    // Per the API, values above 32 mean success.
    r as isize > 32
}

#[cfg(not(windows))]
fn shell_open(target: &std::ffi::OsStr, _params: Option<&str>) -> bool {
    std::process::Command::new("xdg-open").arg(target).spawn().is_ok()
}

/// Splits pasted text into URLs. egui turns pasted line breaks into spaces
/// in single-line fields, so whitespace separates the links.
fn split_urls(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|t| t.trim_matches(|c: char| matches!(c, '"' | '\'' | '<' | '>')))
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::split_urls;

    #[test]
    fn splits_pasted_links() {
        assert_eq!(split_urls("https://a.com/x.zip"), ["https://a.com/x.zip"]);
        assert_eq!(split_urls(" https://a.com/x.zip https://b.com/y.iso  "), ["https://a.com/x.zip", "https://b.com/y.iso"]);
        // A link that carries another URL in its query stays whole.
        assert_eq!(split_urls("<https://a.com/go?u=https://b.com/f.zip>"), ["https://a.com/go?u=https://b.com/f.zip"]);
    }
}

pub fn open_file(path: &Path) {
    // A file moved or deleted since: show its folder instead.
    if !path.exists() || !shell_open(path.as_os_str(), None) {
        open_dir(path.parent().unwrap_or(path));
    }
}

fn open_dir(dir: &Path) {
    shell_open(dir.as_os_str(), None);
}

pub fn open_folder(item: &Item) {
    match &item.status {
        // Opens the folder with the file selected.
        #[cfg(windows)]
        Status::Done(p) if p.exists() => {
            shell_open(std::ffi::OsStr::new("explorer.exe"), Some(&format!("/select,\"{}\"", p.display())));
        }
        Status::Done(p) => open_dir(p.parent().unwrap_or(&item.out_dir)),
        _ => open_dir(&item.out_dir),
    }
}

// ------------------------------------------------------------ window chrome

/// 40 px title bar: logo, name, version, drag area and caption buttons.
fn title_bar(ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let rect = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(rect, 0, CHROME);
    p.hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0, LINE));

    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let btn_w = 46.0;
    let buttons = Rect::from_min_max(pos2(rect.right() - 3.0 * btn_w, rect.top()), rect.right_bottom());

    // Drag / double-click area (everything left of the caption buttons).
    let drag = ui.interact(
        Rect::from_min_max(rect.min, pos2(buttons.left(), rect.bottom())),
        ui.scope_id().with("drag"),
        Sense::click_and_drag(),
    );
    if drag.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    } else if drag.drag_started() {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }

    // Logo + name + version.
    let logo = Rect::from_center_size(pos2(rect.left() + 14.0 + 12.0, rect.center().y), vec2(24.0, 20.0));
    egui::Image::from_bytes("bytes://logo.svg", LOGO_SVG).paint_at(ui, logo);
    let name = text(&p, pos2(logo.right() + 10.0, rect.center().y), Align2::LEFT_CENTER, "Pincers", sans(13.0, W::Semi), TEXT, 200.0);
    text(&p, pos2(name.right() + 10.0, rect.center().y + 0.5), Align2::LEFT_CENTER, VERSION, mono(11.0, W::Regular), DIM, 100.0);

    for (i, kind) in ["min", "max", "close"].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(buttons.left() + i as f32 * btn_w, rect.top()), vec2(btn_w, rect.height() - 1.0));
        let resp = ui.interact(r, ui.scope_id().with(kind), Sense::click());
        let hovered = resp.hovered();
        let (bg, fg) = match (kind, hovered) {
            ("close", true) => (CLOSE_RED, Color32::WHITE),
            (_, true) => (RAISED, MUTED),
            _ => (Color32::TRANSPARENT, MUTED),
        };
        p.rect_filled(r, 0, bg);
        match kind {
            "min" => icon_min(&p, r, fg),
            "max" => icon_max(&p, r, fg, maximized),
            _ => icon_close(&p, r, fg),
        }
        if resp.clicked() {
            ctx.send_viewport_cmd(match kind {
                "min" => ViewportCommand::Minimized(true),
                "max" => ViewportCommand::Maximized(!maximized),
                _ => ViewportCommand::Close,
            });
        }
    }
}

/// Without native decorations the edges must start OS resizes themselves.
fn resize_edges(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let r = ctx.content_rect();
    let m = 5.0;
    let (l, rt, t, b) = (pos.x - r.left() < m, r.right() - pos.x < m, pos.y - r.top() < m, r.bottom() - pos.y < m);
    let dir = match (l, rt, t, b) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::West,
        (_, true, ..) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (.., true) => ResizeDirection::South,
        _ => return,
    };
    ctx.set_cursor_icon(match dir {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
        _ => CursorIcon::ResizeNeSw,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
}

impl eframe::App for PincersApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        self.tick();
        self.drain_inbox(ui.ctx());
        let ctx = ui.ctx().clone();
        let width = ctx.content_rect().width();

        egui::Panel::top("titlebar")
            .exact_size(40.0)
            .frame(egui::Frame::NONE)
            .show_separator_line(false)
            .show(ui, title_bar);

        // Same breakpoints as the mock-up: sidebar from 1180 px, detail panel
        // clamp(290px, 27vw, 360px), stats columns when the list has 700 px.
        let sidebar = width >= 1180.0;
        let has_sel = self.selected.is_some_and(|id| self.items.iter().any(|i| i.id == id));
        let detail_w = (width * 0.27).clamp(290.0, 360.0);
        if sidebar {
            egui::Panel::left("sidebar")
                .exact_size(208.0)
                .frame(egui::Frame::NONE.fill(CHROME))
                .show_separator_line(false)
                .resizable(false)
                .show(ui, |ui| views::sidebar(self, ui));
        }
        if has_sel {
            egui::Panel::right("detail")
                .exact_size(detail_w)
                .frame(egui::Frame::NONE.fill(PANEL))
                .show_separator_line(false)
                .resizable(false)
                .show(ui, |ui| views::detail(self, ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
            .show(ui, |ui| views::main(self, ui, !sidebar));

        views::settings_window(self, &ctx);
        resize_edges(&ctx);
        // 1 px window outline, since the native frame is gone. winit keeps the
        // drop shadow of borderless windows by moving the client area 1 px
        // down, so its last row is off-window: draw the bottom edge above it.
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if !maximized {
            let mut r = ctx.content_rect();
            r.max.y -= 1.0;
            ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("outline")))
                .rect_stroke(r, 0, egui::Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
        }

        if self.items.iter().any(|i| i.status == Status::Running) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.dirty && self.last_save.elapsed() > Duration::from_secs(1) {
            self.persist();
            self.dirty = false;
            self.last_save = Instant::now();
        }
    }

    fn on_exit(&mut self) {
        bridge::unregister();
        // Pause everything so each engine writes its `.pincers` state.
        for item in &self.items {
            item.cancel.store(true, Ordering::Relaxed);
        }
        std::thread::sleep(Duration::from_millis(400));
        self.poll();
        self.persist();
    }
}
