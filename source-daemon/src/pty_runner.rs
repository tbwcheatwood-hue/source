//! Persistent PTY sessions with real-time stdout/stderr streaming and exit-code handling.

use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use parking_lot::Mutex;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtySessionInfo {
    pub pty_id: String,
    pub shell: String,
    pub cwd: String,
    pub cols: u16,
    pub rows: u16,
    pub created_unix_ms: i64,
    pub exit_code: Option<i32>,
    pub alive: bool,
    pub buffered_bytes: usize,
}

struct PtySession {
    info: PtySessionInfo,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    buffer: Arc<Mutex<Vec<u8>>>,
}

#[derive(Clone, Default)]
pub struct PtyRunner {
    sessions: Arc<DashMap<String, Arc<PtySession>>>,
}

impl PtyRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(
        &self,
        cwd: Option<PathBuf>,
        shell: Option<String>,
        cols: Option<u16>,
        rows: Option<u16>,
        env: Option<HashMap<String, String>>,
    ) -> Result<PtySessionInfo> {
        let pty_system = native_pty_system();
        let cols = cols.unwrap_or(120);
        let rows = rows.unwrap_or(32);
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let shell = shell.unwrap_or_else(default_shell);
        let cwd = cwd.unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| {
                #[cfg(windows)]
                {
                    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| r"C:\".into()))
                }
                #[cfg(not(windows))]
                {
                    PathBuf::from("/")
                }
            })
        });
        let mut cmd = CommandBuilder::new(&shell);
        cmd.cwd(&cwd);
        if let Some(map) = env {
            for (k, v) in map {
                cmd.env(k, v);
            }
        }
        // Interactive login-ish shell (Unix). Windows PowerShell/cmd need no -i.
        let shell_base = std::path::Path::new(&shell)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(shell.as_str())
            .to_ascii_lowercase();
        if shell_base == "bash" || shell_base == "zsh" || shell_base == "bash.exe" {
            cmd.arg("-i");
        }

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .context("clone pty reader")?;
        let writer = pair.master.take_writer().context("take pty writer")?;
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let buffer_reader = buffer.clone();
        let pty_id = Uuid::new_v4().to_string();

        thread::Builder::new()
            .name(format!("source-pty-{pty_id}"))
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            buffer_reader.lock().extend_from_slice(&buf[..n]);
                        }
                        Err(_) => break,
                    }
                }
            })?;

        let info = PtySessionInfo {
            pty_id: pty_id.clone(),
            shell,
            cwd: cwd.display().to_string(),
            cols,
            rows,
            created_unix_ms: chrono::Utc::now().timestamp_millis(),
            exit_code: None,
            alive: true,
            buffered_bytes: 0,
        };

        let session = Arc::new(PtySession {
            info: info.clone(),
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            buffer,
        });
        self.sessions.insert(pty_id, session);
        Ok(info)
    }

    pub fn write(&self, pty_id: &str, data: &str) -> Result<()> {
        let session = self
            .sessions
            .get(pty_id)
            .ok_or_else(|| anyhow!("unknown pty {pty_id}"))?;
        session.writer.lock().write_all(data.as_bytes())?;
        session.writer.lock().flush()?;
        Ok(())
    }

    pub fn read(&self, pty_id: &str, max_bytes: Option<usize>, wait_ms: Option<u64>) -> Result<PtyReadResult> {
        let session = self
            .sessions
            .get(pty_id)
            .ok_or_else(|| anyhow!("unknown pty {pty_id}"))?
            .clone();

        let wait = Duration::from_millis(wait_ms.unwrap_or(50));
        let deadline = Instant::now() + wait;
        let max_bytes = max_bytes.unwrap_or(65_536);

        while Instant::now() < deadline {
            if !session.buffer.lock().is_empty() {
                break;
            }
            if let Some(code) = self.poll_exit(&session) {
                let output = drain_buffer(&session.buffer, max_bytes);
                return Ok(PtyReadResult {
                    pty_id: pty_id.to_string(),
                    data: output,
                    exit_code: Some(code),
                    alive: false,
                });
            }
            thread::sleep(Duration::from_millis(10));
        }

        let exit_code = self.poll_exit(&session);
        let data = drain_buffer(&session.buffer, max_bytes);
        Ok(PtyReadResult {
            pty_id: pty_id.to_string(),
            data,
            exit_code,
            alive: exit_code.is_none(),
        })
    }

    pub fn resize(&self, pty_id: &str, cols: u16, rows: u16) -> Result<PtySessionInfo> {
        let session = self
            .sessions
            .get(pty_id)
            .ok_or_else(|| anyhow!("unknown pty {pty_id}"))?;
        session.master.lock().resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut info = session.info.clone();
        info.cols = cols;
        info.rows = rows;
        info.buffered_bytes = session.buffer.lock().len();
        info.exit_code = self.poll_exit(&session);
        info.alive = info.exit_code.is_none();
        Ok(info)
    }

    pub fn close(&self, pty_id: &str) -> Result<PtySessionInfo> {
        let (_, session) = self
            .sessions
            .remove(pty_id)
            .ok_or_else(|| anyhow!("unknown pty {pty_id}"))?;
        let mut child = session.child.lock();
        let _ = child.kill();
        let code = child.wait().ok().map(|s| s.exit_code() as i32);
        let mut info = session.info.clone();
        info.exit_code = code;
        info.alive = false;
        info.buffered_bytes = session.buffer.lock().len();
        Ok(info)
    }

    pub fn list(&self) -> Vec<PtySessionInfo> {
        self.sessions
            .iter()
            .map(|e| {
                let s = e.value();
                let mut info = s.info.clone();
                info.exit_code = self.poll_exit(s);
                info.alive = info.exit_code.is_none();
                info.buffered_bytes = s.buffer.lock().len();
                info
            })
            .collect()
    }

    fn poll_exit(&self, session: &PtySession) -> Option<i32> {
        let mut child = session.child.lock();
        match child.try_wait() {
            Ok(Some(status)) => Some(status.exit_code() as i32),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyReadResult {
    pub pty_id: String,
    pub data: String,
    pub exit_code: Option<i32>,
    pub alive: bool,
}

fn drain_buffer(buffer: &Arc<Mutex<Vec<u8>>>, max_bytes: usize) -> String {
    let mut guard = buffer.lock();
    let n = guard.len().min(max_bytes);
    let chunk: Vec<u8> = guard.drain(..n).collect();
    String::from_utf8_lossy(&chunk).into_owned()
}

fn default_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        if !shell.is_empty() {
            return shell;
        }
    }
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| {
            // Prefer PowerShell when present; fall back to cmd.exe.
            which::which("pwsh")
                .or_else(|_| which::which("powershell"))
                .ok()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| r"C:\Windows\System32\cmd.exe".into())
        })
    }
    #[cfg(not(windows))]
    {
        "/bin/bash".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_shell() -> String {
        #[cfg(windows)]
        {
            default_shell()
        }
        #[cfg(not(windows))]
        {
            "/bin/bash".into()
        }
    }

    fn test_echo_command() -> &'static str {
        #[cfg(windows)]
        {
            "echo source-pty-ok\r\n"
        }
        #[cfg(not(windows))]
        {
            "echo source-pty-ok\n"
        }
    }

    #[test]
    fn open_echo_and_read() {
        let runner = PtyRunner::new();
        let info = runner
            .open(None, Some(test_shell()), Some(80), Some(24), None)
            .expect("open pty");
        runner
            .write(&info.pty_id, test_echo_command())
            .expect("write");
        let mut saw = String::new();
        for _ in 0..40 {
            let r = runner.read(&info.pty_id, Some(4096), Some(100)).unwrap();
            saw.push_str(&r.data);
            if saw.contains("source-pty-ok") {
                break;
            }
        }
        assert!(saw.contains("source-pty-ok"), "output was: {saw}");
        let closed = runner.close(&info.pty_id).unwrap();
        assert!(!closed.alive);
    }
}
