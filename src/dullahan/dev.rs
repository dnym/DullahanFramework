//! Closed-loop runtime dev bridge (Unix domain socket) + headless script runner.

use crate::dullahan::gfx::DebugOverlay;
use crate::dullahan::platform::{InputState, Key};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

pub const DEFAULT_SOCK_PATH: &str = "/tmp/dullahan.sock";
pub const DEFAULT_SNAP_PATH: &str = "/tmp/dullahan_state.snap";

/// Parsed command-line configuration for an `App`.
#[derive(Clone, Debug)]
pub struct AppConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub headless: bool,
    pub max_frames: Option<u64>,
    pub fixed_dt: Option<f32>,
    pub snap_path: Option<PathBuf>,
    pub sock_path: Option<PathBuf>,
    pub script_path: Option<PathBuf>,
    pub restore_path: Option<PathBuf>,
}

impl AppConfig {
    pub fn from_args(title: &str, default_width: u32, default_height: u32) -> Self {
        let mut cfg = Self {
            title: title.to_string(),
            width: default_width,
            height: default_height,
            headless: false,
            max_frames: None,
            fixed_dt: None,
            snap_path: None,
            sock_path: Some(PathBuf::from(DEFAULT_SOCK_PATH)),
            script_path: None,
            restore_path: None,
        };

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--headless" => {
                    cfg.headless = true;
                    if cfg.fixed_dt.is_none() {
                        cfg.fixed_dt = Some(1.0 / 60.0);
                    }
                }
                "--size" => {
                    if let Some(v) = args.next()
                        && let Some((w, h)) = parse_size(&v)
                    {
                        cfg.width = w;
                        cfg.height = h;
                    }
                }
                "--frames" => {
                    if let Some(v) = args.next()
                        && let Ok(n) = v.parse::<u64>()
                    {
                        cfg.max_frames = Some(n);
                    }
                }
                "--dt" => {
                    if let Some(v) = args.next()
                        && let Ok(dt) = v.parse::<f32>()
                    {
                        cfg.fixed_dt = Some(dt.max(1e-5));
                    }
                }
                "--snap" => {
                    if let Some(v) = args.next() {
                        cfg.snap_path = Some(PathBuf::from(v));
                    }
                }
                "--sock" => {
                    if let Some(v) = args.next() {
                        cfg.sock_path = Some(PathBuf::from(v));
                    }
                }
                "--no-sock" => {
                    cfg.sock_path = None;
                }
                "--script" => {
                    if let Some(v) = args.next() {
                        cfg.script_path = Some(PathBuf::from(v));
                    }
                }
                "--restore" => {
                    if let Some(v) = args.next() {
                        cfg.restore_path = Some(PathBuf::from(v));
                    }
                }
                _ => {}
            }
        }

        // In headless mode without an explicit --sock, default to no socket unless requested
        // (avoids colliding with an interactive instance if user runs quick headless tests).
        if cfg.headless && !std::env::args().any(|a| a == "--sock") {
            cfg.sock_path = None;
        }
        // In headless mode with --snap and no --frames, run 1 frame cleanly.
        if cfg.headless && cfg.max_frames.is_none() && cfg.snap_path.is_some() {
            cfg.max_frames = Some(1);
        }

        cfg
    }
}

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x').or_else(|| s.split_once('X'))?;
    Some((w.parse::<u32>().ok()?.max(16), h.parse::<u32>().ok()?.max(16)))
}

// ---------------------------------------------------------------------------
// Pending commands & scripted playback
// ---------------------------------------------------------------------------

pub struct PendingScreenshot {
    pub path: PathBuf,
    pub reply_stream: Option<UnixStream>,
}

pub struct ScriptEntry {
    pub frame: u64,
    pub command: String,
}

pub struct DevBridge {
    listener: Option<UnixListener>,
    sock_path: Option<PathBuf>,
    script: Vec<ScriptEntry>,
    pub paused: bool,
    pub step_budget: u32,
    pub pending_screenshot: Option<PendingScreenshot>,
}

/// Action returned from command dispatch so `App` / game can react cleanly.
pub enum DevAction {
    None,
    Quit,
    Reload { stream: Option<UnixStream> },
    SetConfig { key: String, val: String, stream: Option<UnixStream> },
    SnapshotSave { path: PathBuf, stream: Option<UnixStream> },
    SnapshotLoad { path: PathBuf, stream: Option<UnixStream> },
    SetOverlay(DebugOverlay),
    GameCmd { args: String, stream: Option<UnixStream> },
}

impl DevBridge {
    pub fn new(sock_path: Option<PathBuf>, script_path: Option<&Path>) -> Self {
        let listener = sock_path.as_ref().and_then(|p| {
            let _ = fs::remove_file(p);
            match UnixListener::bind(p) {
                Ok(l) => {
                    let _ = l.set_nonblocking(true);
                    Some(l)
                }
                Err(e) => {
                    eprintln!("[dev] warning: failed to bind {}: {e}", p.display());
                    None
                }
            }
        });

        let script = script_path
            .and_then(|p| load_script(p).ok())
            .unwrap_or_default();

        Self {
            listener,
            sock_path,
            script,
            paused: false,
            step_budget: 0,
            pending_screenshot: None,
        }
    }

    pub fn should_simulate(&mut self) -> bool {
        if !self.paused {
            return true;
        }
        if self.step_budget > 0 {
            self.step_budget -= 1;
            true
        } else {
            false
        }
    }

    /// Runs any script entries scheduled for `frame` and drains non-blocking socket connections.
    pub fn poll(
        &mut self,
        frame: u64,
        state_summary: &str,
        input: &mut InputState,
    ) -> Vec<DevAction> {
        let mut actions = Vec::new();

        // 1. Trigger scripted commands for this exact frame.
        let due: Vec<String> = self
            .script
            .iter()
            .filter(|e| e.frame == frame)
            .map(|e| e.command.clone())
            .collect();
        for cmd_line in due {
            let act = self.handle_line(&cmd_line, state_summary, input, None);
            if !matches!(act, DevAction::None) {
                actions.push(act);
            }
        }

        // 2. Drain any pending Unix socket connections.
        let mut accepted = Vec::new();
        if let Some(listener) = &self.listener {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => accepted.push(stream),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
        }

        for stream in accepted {
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(250)));
            let mut line = String::new();
            let read_ok = {
                let mut reader = BufReader::new(&stream);
                reader.read_line(&mut line).is_ok()
            };
            if !read_ok {
                continue;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let act = self.handle_line(trimmed, state_summary, input, Some(stream));
            if !matches!(act, DevAction::None) {
                actions.push(act);
            }
        }

        actions
    }

    fn handle_line(
        &mut self,
        line: &str,
        state_summary: &str,
        input: &mut InputState,
        mut stream: Option<UnixStream>,
    ) -> DevAction {
        let mut parts = line.split_whitespace();
        let Some(cmd) = parts.next() else {
            return DevAction::None;
        };

        match cmd {
            "ping" => {
                reply(&mut stream, "OK pong\n");
                DevAction::None
            }
            "state" => {
                reply(&mut stream, &format!("OK {state_summary}\n"));
                DevAction::None
            }
            "pause" => {
                self.paused = true;
                self.step_budget = 0;
                reply(&mut stream, "OK paused=1\n");
                DevAction::None
            }
            "resume" => {
                self.paused = false;
                self.step_budget = 0;
                reply(&mut stream, "OK paused=0\n");
                DevAction::None
            }
            "step" => {
                let n = parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                self.paused = true;
                self.step_budget = self.step_budget.saturating_add(n.max(1));
                reply(&mut stream, &format!("OK step={n}\n"));
                DevAction::None
            }
            "overlay" => {
                let arg = parts.next().unwrap_or("none");
                if let Some(ov) = DebugOverlay::from_str(arg) {
                    reply(&mut stream, &format!("OK overlay={}\n", ov.as_str()));
                    DevAction::SetOverlay(ov)
                } else {
                    reply(&mut stream, "ERR expected: none|grid|wireframe|colliders\n");
                    DevAction::None
                }
            }
            "screenshot" | "snap" => {
                let Some(path) = parts.next() else {
                    reply(&mut stream, "ERR usage: screenshot <path.png>\n");
                    return DevAction::None;
                };
                // Hold stream open until end_frame writes the PNG so dctl never races!
                self.pending_screenshot = Some(PendingScreenshot {
                    path: PathBuf::from(path),
                    reply_stream: stream,
                });
                DevAction::None
            }
            "input" => {
                let kind = parts.next().unwrap_or("");
                if kind == "key" {
                    let kname = parts.next().unwrap_or("");
                    let dur = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.1);
                    if let Some(k) = Key::from_name(kname) {
                        input.inject_key(k, dur);
                        reply(&mut stream, &format!("OK input key={kname} dur={dur:.3}\n"));
                    } else {
                        reply(&mut stream, &format!("ERR unknown key: {kname}\n"));
                    }
                } else {
                    reply(&mut stream, "ERR usage: input key <KEY> [seconds]\n");
                }
                DevAction::None
            }
            "reload" => DevAction::Reload { stream },
            "set" => {
                let Some(key) = parts.next() else {
                    reply(&mut stream, "ERR usage: set <key> <value>\n");
                    return DevAction::None;
                };
                let rest: Vec<&str> = parts.collect();
                if rest.is_empty() {
                    reply(&mut stream, "ERR usage: set <key> <value>\n");
                    return DevAction::None;
                }
                DevAction::SetConfig { key: key.to_string(), val: rest.join(" "), stream }
            }
            "snapshot" => {
                let sub = parts.next().unwrap_or("");
                let path = PathBuf::from(parts.next().unwrap_or(DEFAULT_SNAP_PATH));
                match sub {
                    "save" => DevAction::SnapshotSave { path, stream },
                    "load" => DevAction::SnapshotLoad { path, stream },
                    _ => {
                        reply(&mut stream, "ERR usage: snapshot <save|load> [path]\n");
                        DevAction::None
                    }
                }
            }
            "cmd" => {
                let rest = line.strip_prefix("cmd").unwrap_or("").trim().to_string();
                DevAction::GameCmd { args: rest, stream }
            }
            "quit" => {
                reply(&mut stream, "OK quitting\n");
                DevAction::Quit
            }
            other => {
                reply(&mut stream, &format!("ERR unknown command: {other}\n"));
                DevAction::None
            }
        }
    }
}

impl Drop for DevBridge {
    fn drop(&mut self) {
        if let Some(p) = &self.sock_path
            && self.listener.is_some()
        {
            let _ = fs::remove_file(p);
        }
    }
}

pub fn reply(stream: &mut Option<UnixStream>, msg: &str) {
    if let Some(s) = stream.as_mut() {
        let _ = s.write_all(msg.as_bytes());
        let _ = s.flush();
    }
}

fn load_script(path: &Path) -> Result<Vec<ScriptEntry>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Format: "at <frame> <command...>"
        let mut parts = line.splitn(3, char::is_whitespace);
        let kw = parts.next().unwrap_or("");
        let frame_s = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("").trim();
        if kw == "at"
            && let Ok(frame) = frame_s.parse::<u64>()
            && !rest.is_empty()
        {
            out.push(ScriptEntry { frame, command: rest.to_string() });
        }
    }
    Ok(out)
}
