//! How the browser extension (and a second `pincers.exe`) reach the running
//! Pincers window — the counterpart of IDM's `IDMMsgHost.exe`.
//!
//! * The window listens on `127.0.0.1:<random port>` and writes the port plus
//!   a random token to `%APPDATA%\Pincers\instance.json`. Requests without the
//!   token are dropped, so web pages cannot push downloads into Pincers.
//! * The browser starts `pincers.exe` as a *native messaging host* (stdin /
//!   stdout, 32-bit length + JSON). The host forwards each request to the
//!   window, starting Pincers first when it is not open.

use crate::tr;
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Name used in the browsers' native messaging registrations.
pub const HOST_NAME: &str = "com.pincers.host";
/// ID of the Chrome/Opera extension, fixed by the `key` in its manifest.
pub const CHROME_EXTENSION_ID: &str = "oogdnhbjgdjaaolpjobohbmhkbjiljla";
/// `browser_specific_settings.gecko.id` of the Firefox add-on.
pub const FIREFOX_ADDON_ID: &str = "pincers@pincers.app";

/// One download handed over by the browser.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DownloadRequest {
    pub url: String,
    pub referrer: Option<String>,
    /// `Cookie` header value for `url`.
    pub cookies: Option<String>,
    pub user_agent: Option<String>,
    /// Name the browser picked (from `Content-Disposition` or the URL).
    pub file_name: Option<String>,
}

impl DownloadRequest {
    /// Extra HTTP headers the engine must send so the server treats Pincers
    /// like the browser session that started the download.
    pub fn headers(&self) -> Vec<(String, String)> {
        let mut h = Vec::new();
        if let Some(c) = self.cookies.as_ref().filter(|c| !c.is_empty()) {
            h.push(("Cookie".into(), c.clone()));
        }
        if let Some(r) = self.referrer.as_ref().filter(|r| !r.is_empty()) {
            h.push(("Referer".into(), r.clone()));
        }
        if let Some(u) = self.user_agent.as_ref().filter(|u| !u.is_empty()) {
            h.push(("User-Agent".into(), u.clone()));
        }
        h
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Bring the window to the front.
    Show,
    Add { downloads: Vec<DownloadRequest> },
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    token: String,
    #[serde(flatten)]
    command: Command,
}

#[derive(Serialize, Deserialize)]
struct InstanceInfo {
    port: u16,
    token: String,
    pid: u32,
}

fn instance_path() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Pincers")
        .join("instance.json")
}

fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    // RandomState is seeded from the OS RNG; two of them give 128 bits.
    (0..2)
        .map(|_| {
            let mut h = RandomState::new().build_hasher();
            h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
            format!("{:016x}", h.finish())
        })
        .collect()
}

// ------------------------------------------------------------- client side

/// Sends `command` to the running window. Errors when none is listening.
pub fn send(command: Command) -> Result<()> {
    let data = std::fs::read(instance_path()).with_context(|| tr!("bridge.not_open"))?;
    let info: InstanceInfo = serde_json::from_slice(&data)?;
    let addr = SocketAddr::from(([127, 0, 0, 1], info.port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(800)).with_context(|| tr!("bridge.not_open"))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut line = serde_json::to_vec(&Envelope { token: info.token, command })?;
    line.push(b'\n');
    stream.write_all(&line)?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    if reply.trim() == "ok" { Ok(()) } else { bail!(tr!("bridge.bad_reply", reply = format!("{reply:?}"))) }
}

/// Like [`send`], but starts Pincers first if it is not open.
pub fn send_or_launch(command: Command) -> Result<()> {
    if send(command.clone()).is_ok() {
        return Ok(());
    }
    launch_detached()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        std::thread::sleep(Duration::from_millis(250));
        match send(command.clone()) {
            Ok(()) => return Ok(()),
            Err(e) if Instant::now() > deadline => return Err(e.context(tr!("bridge.no_start"))),
            Err(_) => {}
        }
    }
}

/// Starts `pincers.exe gui` independent of the caller. A browser runs its
/// native host inside a job object that is killed with the browser tab, so the
/// window must break away from it.
fn launch_detached() -> Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("gui")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        if cmd.spawn().is_ok() {
            return Ok(());
        }
        // The job may forbid breaking away; start it inside the job then.
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().map(|_| ()).with_context(|| tr!("bridge.cannot_start", path = exe.display()))
}

// ------------------------------------------------------------- server side

/// Keeps `instance.json` while the window is open and removes it after.
pub struct Listener {
    token: String,
    listener: TcpListener,
}

impl Listener {
    pub fn bind() -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let token = random_token();
        let info = InstanceInfo { port: listener.local_addr()?.port(), token: token.clone(), pid: std::process::id() };
        let path = instance_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, serde_json::to_vec(&info)?)?;
        Ok(Self { token, listener })
    }

    /// Accepts requests forever on a background thread, calling `on_command`
    /// for each authenticated one.
    pub fn spawn(self, on_command: impl Fn(Command) + Send + 'static) {
        std::thread::spawn(move || {
            for stream in self.listener.incoming().flatten() {
                let _ = handle(stream, &self.token, &on_command);
            }
        });
    }
}

fn handle(stream: TcpStream, token: &str, on_command: &impl Fn(Command)) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut line = String::new();
    // Requests are small; cap them so a stray client cannot exhaust memory.
    BufReader::new(stream.try_clone()?.take(4 << 20)).read_line(&mut line)?;
    let env: Envelope = serde_json::from_str(&line)?;
    if env.token != token {
        return Err(anyhow!(tr!("bridge.bad_token")));
    }
    on_command(env.command);
    (&stream).write_all(b"ok\n")?;
    Ok(())
}

/// Removes `instance.json` if it still belongs to this process.
pub fn unregister() {
    let path = instance_path();
    let mine = std::fs::read(&path)
        .ok()
        .and_then(|d| serde_json::from_slice::<InstanceInfo>(&d).ok())
        .is_some_and(|i| i.pid == std::process::id());
    if mine {
        let _ = std::fs::remove_file(path);
    }
}

// ----------------------------------------------------- native messaging host

/// Whether the browser started us as its native messaging host: Chrome and
/// Opera pass the extension origin, Firefox the manifest path and add-on id.
pub fn is_native_host_invocation(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| a.starts_with("chrome-extension://"))
        || args.get(1).is_some_and(|a| a.to_ascii_lowercase().ends_with(".json"))
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HostMessage {
    /// The extension checks that Pincers is installed.
    Ping,
    Download { downloads: Vec<DownloadRequest> },
    Show,
}

/// Native messaging loop: one JSON message in, one JSON reply out.
pub fn run_native_host() -> Result<()> {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    loop {
        let mut len = [0u8; 4];
        if stdin.read_exact(&mut len).is_err() {
            return Ok(()); // the browser closed the port
        }
        let len = u32::from_ne_bytes(len) as usize;
        if len > 64 << 20 {
            bail!(tr!("bridge.too_big", len = len));
        }
        let mut buf = vec![0u8; len];
        stdin.read_exact(&mut buf)?;
        let reply = match serde_json::from_slice::<HostMessage>(&buf) {
            Ok(HostMessage::Ping) => serde_json::json!({
                "ok": true,
                "version": env!("CARGO_PKG_VERSION"),
                "running": instance_alive(),
            }),
            Ok(HostMessage::Show) => result_json(send_or_launch(Command::Show)),
            Ok(HostMessage::Download { downloads }) => result_json(send_or_launch(Command::Add { downloads })),
            Err(e) => serde_json::json!({ "ok": false, "error": tr!("bridge.bad_message", error = e) }),
        };
        let out = serde_json::to_vec(&reply)?;
        stdout.write_all(&(out.len() as u32).to_ne_bytes())?;
        stdout.write_all(&out)?;
        stdout.flush()?;
    }
}

fn result_json(r: Result<()>) -> serde_json::Value {
    match r {
        Ok(()) => serde_json::json!({ "ok": true }),
        Err(e) => serde_json::json!({ "ok": false, "error": format!("{e:#}") }),
    }
}

/// Whether a window is listening, without asking it to do anything.
fn instance_alive() -> bool {
    std::fs::read(instance_path())
        .ok()
        .and_then(|d| serde_json::from_slice::<InstanceInfo>(&d).ok())
        .is_some_and(|i| {
            TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], i.port)), Duration::from_millis(300)).is_ok()
        })
}

// ----------------------------------------------------------- registration

/// Registers the native host for Firefox, Chrome, Chromium and Opera (Opera
/// reads Chrome's key) under HKCU, pointing at this executable.
#[cfg(windows)]
pub fn register(chrome_extension_id: &str, firefox_addon_id: &str) -> Result<Vec<String>> {
    let exe = std::env::current_exe()?;
    let dir = instance_path().parent().unwrap().to_owned();
    std::fs::create_dir_all(&dir)?;
    let chrome_manifest = dir.join("native-host-chrome.json");
    let firefox_manifest = dir.join("native-host-firefox.json");
    std::fs::write(
        &chrome_manifest,
        serde_json::to_vec_pretty(&serde_json::json!({
            "name": HOST_NAME,
            "description": "Pincers download manager",
            "path": exe,
            "type": "stdio",
            "allowed_origins": [format!("chrome-extension://{chrome_extension_id}/")],
        }))?,
    )?;
    std::fs::write(
        &firefox_manifest,
        serde_json::to_vec_pretty(&serde_json::json!({
            "name": HOST_NAME,
            "description": "Pincers download manager",
            "path": exe,
            "type": "stdio",
            "allowed_extensions": [firefox_addon_id],
        }))?,
    )?;
    let mut done = Vec::new();
    for (key, manifest) in [
        (r"Software\Google\Chrome\NativeMessagingHosts", &chrome_manifest),
        (r"Software\Chromium\NativeMessagingHosts", &chrome_manifest),
        (r"Software\Mozilla\NativeMessagingHosts", &firefox_manifest),
    ] {
        let full = format!(r"HKCU\{key}\{HOST_NAME}");
        let status = std::process::Command::new("reg")
            .args(["add", &full, "/ve", "/t", "REG_SZ", "/d"])
            .arg(manifest)
            .arg("/f")
            .stdout(std::process::Stdio::null())
            .status()?;
        if !status.success() {
            bail!(tr!("bridge.reg_failed", key = full));
        }
        done.push(full);
    }
    Ok(done)
}
