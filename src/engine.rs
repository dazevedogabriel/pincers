//! Download engine: the scheduler (IDM's `Session`/`ChunksList` owner) and the
//! per-connection workers (IDM's `Connection`/`HTTPConnection`).
//!
//! Flow, as observed in IDMan.exe:
//! 1. The first connection asks for `Range: bytes=0-`; its answer tells the
//!    size and whether ranges work, and it keeps streaming as segment 0.
//! 2. While there are free connection slots, `NeedNewConnection` picks the
//!    segment with the most free space and halves it (dynamic segmentation).
//!    Each new connection asks for an open range `bytes=<start>-`.
//! 3. When a connection finishes, its slot is reused immediately to split the
//!    largest remaining segment again, so all connections stay busy until the
//!    very end.
//! 4. A connection reaching its end swallows the following segment if that one
//!    has not really started, otherwise it verifies a few overlapping bytes
//!    and stops. A mismatch means the server sends inconsistent data and the
//!    download restarts on a single connection.
//!
//! Design choice that differs from IDM: instead of one temp file per chunk
//! joined at the end (`ChunksList::JoinFile`), every connection writes in place
//! into one preallocated file, so there is no slow "assembling" phase.

use crate::chunks::{ChunksList, MIN_SEGMENT, SegId, VERIFY_OVERLAP};
use crate::limiter::SpeedLimiter;
use crate::probe::{RemoteInfo, probe};
use crate::state::{SavedSegment, SavedState, part_path};
use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use reqwest::{Client, Response, StatusCode, header};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

#[derive(Debug, Clone)]
pub struct Config {
    /// IDM default "MaxConnectionsNumber" is 8.
    pub connections: usize,
    pub min_segment: u64,
    pub speed_limit: Option<u64>,
    pub out_dir: PathBuf,
    /// Attempts per connection before it gives up (IDM: "Connection failed, restarting...").
    pub retries: u32,
    /// Silence on a connection longer than this restarts it.
    pub read_timeout: Duration,
    /// Sent on every request (cookies, referrer, user agent from the browser).
    pub headers: Vec<(String, String)>,
    /// Name to save as instead of the one the server suggests.
    pub file_name: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            connections: 8,
            min_segment: MIN_SEGMENT,
            speed_limit: None,
            out_dir: PathBuf::from("."),
            retries: 10,
            read_timeout: Duration::from_secs(30),
            headers: Vec::new(),
            file_name: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SegmentView {
    pub start: u64,
    pub pos: u64,
    pub end: u64,
    pub active: bool,
    pub failed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub file_name: String,
    pub size: Option<u64>,
    pub downloaded: u64,
    /// Bytes per second over the last couple of seconds.
    pub speed: f64,
    pub connections: usize,
    pub resumable: bool,
    pub segments: Vec<SegmentView>,
    pub status: String,
    pub finished: bool,
}

pub fn build_client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent(concat!("Pincers/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .pool_max_idle_per_host(32)
        // Ranges must address the raw bytes, never a transparently decoded body.
        .no_gzip()
        .build()?)
}

struct Shared {
    client: Client,
    url: String,
    info: RemoteInfo,
    file: File,
    chunks: Mutex<ChunksList>,
    limiter: SpeedLimiter,
    cfg: Config,
    /// Exponential moving average of how long a new connection takes to
    /// deliver its response headers (the `connect_time` of FindMaxFreeChunk).
    connect_time: Mutex<f64>,
    corrupted: AtomicBool,
    cancel: Arc<AtomicBool>,
}

enum WorkerEnd {
    Done,
    Failed(anyhow::Error),
    /// The server refused one more connection (429/503).
    Throttled,
}

/// HTTP 429/503 on a range request: the server caps connections per client.
#[derive(Debug)]
struct ServerBusy(StatusCode);

impl std::fmt::Display for ServerBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "o servidor recusou mais conexões ({})", self.0)
    }
}

impl std::error::Error for ServerBusy {}

/// Downloads `url`, publishing progress on `progress`. Set `cancel` to pause:
/// the state is saved and the next call with the same URL resumes.
pub async fn download(
    url: &str,
    cfg: Config,
    progress: watch::Sender<Progress>,
    cancel: Arc<AtomicBool>,
) -> Result<PathBuf> {
    let client = build_client()?;
    progress.send_modify(|p| p.status = "Conectando...".into());
    let (mut info, first) = probe(&client, url, &cfg.headers).await?;
    if let Some(name) = cfg.file_name.as_deref().map(crate::probe::sanitize).filter(|n| !n.is_empty()) {
        info.file_name = name;
    }
    let target = cfg.out_dir.join(&info.file_name);
    std::fs::create_dir_all(&cfg.out_dir).ok();

    progress.send_modify(|p| {
        p.file_name = info.file_name.clone();
        p.size = info.size;
        p.resumable = info.resumable;
    });

    let (Some(size), true) = (info.size, info.resumable) else {
        // IDM: "the server does not support resume" -> one connection, no split.
        return single_stream(first, info, target, &cfg, progress, cancel).await;
    };

    // Resume if a compatible state file exists.
    let saved = SavedState::load(&target).filter(|s| {
        s.size == size
            && part_path(&target).exists()
            && (s.etag.is_none() || s.etag == info.etag)
            && (s.last_modified.is_none() || s.last_modified == info.last_modified)
    });
    let part = part_path(&target);
    let (chunks, first) = match &saved {
        Some(s) => {
            let segs: Vec<_> = s.segments.iter().map(|g| (g.start, g.pos, g.end)).collect();
            // Reuse the probe stream only if segment 0 has not started yet.
            let reuse = segs.first().is_some_and(|&(st, p, _)| st == 0 && p == 0);
            (ChunksList::from_saved(&segs), reuse.then_some(first))
        }
        None => (ChunksList::new(size), Some(first)),
    };
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&part)
        .with_context(|| format!("não foi possível criar {}", part.display()))?;
    file.set_len(size)?;

    let shared = Arc::new(Shared {
        client,
        url: info.final_url.to_string(),
        info: info.clone(),
        file,
        chunks: Mutex::new(chunks),
        limiter: SpeedLimiter::new(cfg.speed_limit),
        cfg: cfg.clone(),
        connect_time: Mutex::new(0.5),
        corrupted: AtomicBool::new(false),
        cancel: cancel.clone(),
    });

    let result = schedule(shared.clone(), first, &target, &progress).await;
    save_state(&shared, &target);
    result?;

    drop(shared);
    let final_path = unique_path(&target);
    std::fs::rename(&part, &final_path)?;
    SavedState::remove(&target);
    progress.send_modify(|p| {
        p.finished = true;
        p.status = "Download concluído".into();
    });
    Ok(final_path)
}

async fn schedule(
    shared: Arc<Shared>,
    mut first: Option<Response>,
    target: &Path,
    progress: &watch::Sender<Progress>,
) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<(SegId, WorkerEnd)>();
    let mut max_conn = shared.cfg.connections.max(1);
    let mut last_save = Instant::now();
    let mut speed = SpeedMeter::new(shared.chunks.lock().unwrap().downloaded());
    let mut failures = 0u32;

    loop {
        if shared.cancel.load(Ordering::Relaxed) {
            progress.send_modify(|p| p.status = "Pausado".into());
            bail!("download pausado");
        }

        if shared.corrupted.swap(false, Ordering::Relaxed) {
            // IDM: "started to download the file using one connection because of
            // data corruption..." — drop everything and restart with 1 connection.
            max_conn = 1;
            let size = shared.info.size.unwrap_or(0);
            shared.chunks.lock().unwrap().reset(size);
            progress.send_modify(|p| p.status = "Dados inconsistentes: reiniciando com 1 conexão".into());
            // Running workers see their ids vanish and stop.
        }

        // Fill free slots: first orphaned segments, then dynamic splits.
        {
            let connect_time = *shared.connect_time.lock().unwrap();
            let mut chunks = shared.chunks.lock().unwrap();
            if chunks.all_done() {
                break;
            }
            while chunks.active_count() < max_conn {
                let id = if let Some(id) = chunks.orphan() {
                    id
                } else if let Some((victim, free)) =
                    chunks.need_new_connection(connect_time, shared.cfg.min_segment)
                {
                    match chunks.split(victim, free) {
                        Some(id) => id,
                        None => break,
                    }
                } else {
                    break;
                };
                let seg = chunks.get_mut(id).unwrap();
                seg.active = true;
                seg.failed = false;
                let initial = if id == 0 { first.take() } else { None };
                tokio::spawn(worker(shared.clone(), id, initial, tx.clone()));
            }
        }

        // Wait for a worker to finish or for the next tick.
        match tokio::time::timeout(Duration::from_millis(250), rx.recv()).await {
            Ok(Some((id, WorkerEnd::Failed(e)))) => {
                failures += 1;
                progress.send_modify(|p| p.status = format!("Conexão {id} falhou: {e:#}"));
                let idle = shared.chunks.lock().unwrap().active_count() == 0;
                if failures > shared.cfg.retries * max_conn as u32 && idle {
                    // Publish once more so frontends see which segments failed.
                    publish(&shared, &mut speed, progress, max_conn);
                    return Err(e.context("todas as conexões falharam"));
                }
            }
            Ok(Some((_, WorkerEnd::Throttled))) => {
                // IDM: "Some servers may have specific limitations for a number
                // of connections". Settle on what the server accepts.
                let accepted = shared.chunks.lock().unwrap().active_count().max(1);
                if accepted < max_conn {
                    max_conn = accepted;
                    progress.send_modify(|p| {
                        p.status = format!("O servidor limita conexões: usando {max_conn}")
                    });
                }
            }
            Ok(Some((_, WorkerEnd::Done))) | Ok(None) | Err(_) => {}
        }

        publish(&shared, &mut speed, progress, max_conn);
        if last_save.elapsed() > Duration::from_secs(1) {
            save_state(&shared, target);
            last_save = Instant::now();
        }
    }
    publish(&shared, &mut speed, progress, max_conn);
    Ok(())
}

async fn worker(
    shared: Arc<Shared>,
    id: SegId,
    mut initial: Option<Response>,
    tx: mpsc::UnboundedSender<(SegId, WorkerEnd)>,
) {
    let mut attempt = 0;
    let end = loop {
        match run_connection(&shared, id, initial.take()).await {
            Ok(()) => break WorkerEnd::Done,
            // Only extra connections are refused: hand the segment back so a
            // connection the server accepts can take it over later.
            Err(e) if e.is::<ServerBusy>() && id != 0 => break WorkerEnd::Throttled,
            Err(e) => {
                attempt += 1;
                let gone = shared.chunks.lock().unwrap().get(id).is_none_or(|s| s.done());
                if gone || attempt > shared.cfg.retries || shared.cancel.load(Ordering::Relaxed) {
                    break if gone { WorkerEnd::Done } else { WorkerEnd::Failed(e) };
                }
                // "Connection failed, restarting..." with a growing back-off.
                tokio::time::sleep(Duration::from_millis(500 * attempt.min(10) as u64)).await;
            }
        }
    };
    if let Some(s) = shared.chunks.lock().unwrap().get_mut(id) {
        s.active = false;
        s.speed = 0.0;
        s.failed = matches!(end, WorkerEnd::Failed(_));
    }
    let _ = tx.send((id, end));
}

/// One HTTP connection serving segment `id` until it is complete.
async fn run_connection(shared: &Shared, id: SegId, initial: Option<Response>) -> Result<()> {
    let Some(pos) = shared.chunks.lock().unwrap().get(id).map(|s| s.pos) else {
        return Ok(());
    };

    let resp = match initial {
        Some(r) => r,
        None => {
            let t0 = Instant::now();
            let mut req = shared.client.get(&shared.url).header(header::RANGE, format!("bytes={pos}-"));
            for (k, v) in &shared.cfg.headers {
                req = req.header(k.as_str(), v.as_str());
            }
            // If-Range: if the file changed on the server we get a 200 instead of a 206.
            if let Some(v) = shared.info.etag.as_ref().or(shared.info.last_modified.as_ref()) {
                req = req.header(header::IF_RANGE, v);
            }
            let r = req.send().await?;
            let mut ct = shared.connect_time.lock().unwrap();
            *ct = *ct * 0.7 + t0.elapsed().as_secs_f64() * 0.3;
            r
        }
    };
    if matches!(resp.status(), StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE) {
        return Err(ServerBusy(resp.status()).into());
    }
    if resp.status() != StatusCode::PARTIAL_CONTENT {
        bail!("o servidor respondeu {} a um pedido de intervalo", resp.status());
    }

    let mut stream = resp.bytes_stream();
    let mut write_pos = pos;
    let mut meter = (Instant::now(), 0u64);
    // Bytes collected past our end to compare with the neighbour's data.
    let mut overlap: Vec<u8> = Vec::new();
    let mut verify_at: Option<(u64, usize)> = None;

    loop {
        let item = tokio::time::timeout(shared.cfg.read_timeout, stream.next())
            .await
            .map_err(|_| anyhow!("tempo esgotado"))?;
        let Some(chunk) = item else { break };
        let chunk = chunk?;
        shared.limiter.consume(chunk.len()).await;
        let mut data = &chunk[..];

        while !data.is_empty() {
            if let Some((_, want)) = verify_at {
                let take = (want - overlap.len()).min(data.len());
                overlap.extend_from_slice(&data[..take]);
                data = &data[take..];
                if overlap.len() >= want {
                    break;
                }
                continue;
            }

            let n = {
                let mut chunks = shared.chunks.lock().unwrap();
                if chunks.is_cancelled(id) {
                    return Ok(());
                }
                let Some(seg) = chunks.get(id).cloned() else { return Ok(()) };
                if seg.pos != write_pos {
                    // Our segment was reset (corruption restart): stop.
                    return Ok(());
                }
                if seg.done() {
                    if chunks.try_absorb_next(id) {
                        continue;
                    }
                    // Neighbour is already downloading: check that both
                    // connections agree on the bytes around the boundary.
                    let end = seg.end;
                    let have = chunks
                        .segs
                        .iter()
                        .find(|s| s.start == end)
                        .map(|s| (s.pos - s.start).min(VERIFY_OVERLAP) as usize)
                        .unwrap_or(0);
                    if have == 0 {
                        return Ok(());
                    }
                    verify_at = Some((end, have));
                    continue;
                }
                (seg.end - seg.pos).min(data.len() as u64) as usize
            };

            write_at(&shared.file, &data[..n], write_pos)?;
            write_pos += n as u64;
            data = &data[n..];

            let mut chunks = shared.chunks.lock().unwrap();
            let Some(seg) = chunks.get_mut(id) else { return Ok(()) };
            // A concurrent split may have moved our end below what we just
            // wrote; those extra bytes are identical, the new owner rewrites them.
            seg.pos = write_pos.min(seg.end);
            write_pos = seg.pos;
            meter.1 += n as u64;
            let dt = meter.0.elapsed().as_secs_f64();
            if dt >= 0.5 {
                let inst = meter.1 as f64 / dt;
                seg.speed = if seg.speed == 0.0 { inst } else { seg.speed * 0.6 + inst * 0.4 };
                meter = (Instant::now(), 0);
            }
        }

        if let Some((at, want)) = verify_at
            && overlap.len() >= want
        {
            let mut disk = vec![0u8; want];
            read_at(&shared.file, &mut disk, at)?;
            if disk != overlap {
                shared.corrupted.store(true, Ordering::Relaxed);
            }
            return Ok(());
        }
    }

    let done = shared.chunks.lock().unwrap().get(id).is_none_or(|s| s.done());
    if done || verify_at.is_some() { Ok(()) } else { bail!("conexão encerrada pelo servidor") }
}

/// Fallback for servers without range support: one plain sequential stream.
async fn single_stream(
    resp: Response,
    info: RemoteInfo,
    target: PathBuf,
    cfg: &Config,
    progress: watch::Sender<Progress>,
    cancel: Arc<AtomicBool>,
) -> Result<PathBuf> {
    use std::io::Write;
    let part = part_path(&target);
    let mut file = std::io::BufWriter::with_capacity(1 << 20, File::create(&part)?);
    let limiter = SpeedLimiter::new(cfg.speed_limit);
    let mut stream = resp.bytes_stream();
    let mut done = 0u64;
    let mut meter = SpeedMeter::new(0);
    progress.send_modify(|p| {
        p.connections = 1;
        p.status = "O servidor não suporta segmentação: 1 conexão".into();
    });
    let mut last = Instant::now();
    while let Some(chunk) = tokio::time::timeout(cfg.read_timeout, stream.next())
        .await
        .map_err(|_| anyhow!("tempo esgotado"))?
    {
        if cancel.load(Ordering::Relaxed) {
            bail!("download cancelado (este servidor não permite retomar)");
        }
        let chunk = chunk?;
        limiter.consume(chunk.len()).await;
        file.write_all(&chunk)?;
        done += chunk.len() as u64;
        if last.elapsed() > Duration::from_millis(200) {
            let sp = meter.update(done);
            let size = info.size;
            progress.send_modify(|p| {
                p.downloaded = done;
                p.speed = sp;
                p.segments = vec![SegmentView { start: 0, pos: done, end: size.unwrap_or(done), active: true, failed: false }];
            });
            last = Instant::now();
        }
    }
    file.flush()?;
    drop(file);
    if let Some(size) = info.size
        && done != size
    {
        bail!("download incompleto: {done} de {size} bytes");
    }
    let final_path = unique_path(&target);
    std::fs::rename(&part, &final_path)?;
    progress.send_modify(|p| {
        p.downloaded = done;
        p.finished = true;
        p.status = "Download concluído".into();
    });
    Ok(final_path)
}

fn publish(shared: &Shared, speed: &mut SpeedMeter, progress: &watch::Sender<Progress>, max_conn: usize) {
    let chunks = shared.chunks.lock().unwrap();
    let downloaded = chunks.downloaded();
    let segments = chunks
        .segs
        .iter()
        .map(|s| SegmentView { start: s.start, pos: s.pos, end: s.end, active: s.active, failed: s.failed })
        .collect();
    let connections = chunks.active_count();
    drop(chunks);
    let sp = speed.update(downloaded);
    progress.send_modify(|p| {
        p.downloaded = downloaded;
        p.speed = sp;
        p.connections = connections;
        p.segments = segments;
        if p.status.is_empty() || p.status.starts_with("Conectando") || p.status.starts_with("Baixando") {
            p.status = format!("Baixando com até {max_conn} conexões");
        }
    });
}

fn save_state(shared: &Shared, target: &Path) {
    let chunks = shared.chunks.lock().unwrap();
    let state = SavedState {
        url: shared.url.clone(),
        size: shared.info.size.unwrap_or(0),
        etag: shared.info.etag.clone(),
        last_modified: shared.info.last_modified.clone(),
        segments: chunks
            .segs
            .iter()
            .map(|s| SavedSegment { start: s.start, pos: s.pos, end: s.end })
            .collect(),
    };
    drop(chunks);
    let _ = state.save(target);
}

/// Picks `name (1).ext`, `name (2).ext`… when the target already exists.
fn unique_path(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_owned();
    }
    let stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or("download");
    let ext = target.extension().and_then(|s| s.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    (1..)
        .map(|i| target.with_file_name(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

/// Sliding-window speed over roughly the last two seconds.
struct SpeedMeter {
    samples: std::collections::VecDeque<(Instant, u64)>,
}

impl SpeedMeter {
    fn new(start: u64) -> Self {
        Self { samples: [(Instant::now(), start)].into() }
    }
    fn update(&mut self, total: u64) -> f64 {
        let now = Instant::now();
        self.samples.push_back((now, total));
        while self.samples.len() > 2 && now.duration_since(self.samples[0].0) > Duration::from_secs(2) {
            self.samples.pop_front();
        }
        let (t0, b0) = self.samples[0];
        let dt = now.duration_since(t0).as_secs_f64();
        if dt > 0.0 { (total - b0) as f64 / dt } else { 0.0 }
    }
}

#[cfg(windows)]
fn write_at(f: &File, mut buf: &[u8], mut off: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = f.seek_write(buf, off)?;
        buf = &buf[n..];
        off += n as u64;
    }
    Ok(())
}

#[cfg(windows)]
fn read_at(f: &File, buf: &mut [u8], off: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buf.len() {
        let n = f.seek_read(&mut buf[done..], off + done as u64)?;
        if n == 0 {
            break;
        }
        done += n;
    }
    Ok(())
}

#[cfg(unix)]
fn write_at(f: &File, buf: &[u8], off: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(f, buf, off)
}

#[cfg(unix)]
fn read_at(f: &File, buf: &mut [u8], off: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(f, buf, off)
}
