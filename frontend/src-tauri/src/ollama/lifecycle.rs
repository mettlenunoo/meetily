//! Ties a local `ollama serve` to Hearken's lifetime.
//!
//! On launch, if nothing is listening on 127.0.0.1:11434, Hearken starts `ollama serve`.
//! On exit, it stops that server again. An Ollama that was already running (started by
//! the user, Homebrew services or Ollama.app) is never touched.
//!
//! The PID of the server we start is written to a file so that, if Hearken crashes and
//! leaves it running, the next launch takes ownership again instead of leaving it orphaned.
//!
//! Everything here is best-effort: recording and transcription never depend on it.

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const OLLAMA_ADDR: &str = "127.0.0.1:11434";

/// PID of the `ollama serve` Hearken owns (spawned now, or adopted from a previous crash).
static OWNED: Mutex<Option<Owned>> = Mutex::new(None);

struct Owned {
    pid: u32,
    child: Option<Child>,
    pid_file: PathBuf,
}

fn is_listening() -> bool {
    let addr: SocketAddr = OLLAMA_ADDR.parse().expect("valid address");
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// Apps launched from Finder get a minimal PATH, so check the usual install locations too.
fn find_ollama() -> Option<PathBuf> {
    if let Ok(path) = which::which("ollama") {
        return Some(path);
    }
    [
        "/opt/homebrew/bin/ollama",
        "/usr/local/bin/ollama",
        "/Applications/Ollama.app/Contents/Resources/ollama",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

fn process_is_ollama(pid: u32) -> bool {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().ends_with("ollama"))
        .unwrap_or(false)
}

fn read_pid_file(pid_file: &Path) -> Option<u32> {
    std::fs::read_to_string(pid_file).ok()?.trim().parse().ok()
}

/// Start `ollama serve` unless one is already running. Blocks for up to ~10 s while the
/// server comes up, so call it off the main thread.
pub fn start_if_needed(pid_file: PathBuf) {
    if is_listening() {
        // Adopt a server we started before a crash; leave anyone else's alone.
        match read_pid_file(&pid_file) {
            Some(pid) if process_is_ollama(pid) => {
                log::info!("Ollama: adopting server left running by a previous session (pid {})", pid);
                *OWNED.lock().unwrap() = Some(Owned { pid, child: None, pid_file });
            }
            _ => {
                let _ = std::fs::remove_file(&pid_file);
                log::info!("Ollama: already running, not managed by Hearken");
            }
        }
        return;
    }

    let Some(binary) = find_ollama() else {
        log::info!("Ollama: not installed, skipping auto-start");
        return;
    };

    let child = match Command::new(&binary)
        .arg("serve")
        .env("OLLAMA_HOST", OLLAMA_ADDR) // local only
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            log::warn!("Ollama: failed to start {}: {}", binary.display(), e);
            return;
        }
    };

    let pid = child.id();
    if let Some(dir) = pid_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&pid_file, pid.to_string()) {
        log::warn!("Ollama: could not write pid file {}: {}", pid_file.display(), e);
    }
    *OWNED.lock().unwrap() = Some(Owned { pid, child: Some(child), pid_file });

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if is_listening() {
            log::info!("Ollama: started {} serve (pid {})", binary.display(), pid);
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    log::warn!("Ollama: started (pid {}) but not listening on {} after 10 s", pid, OLLAMA_ADDR);
}

/// Stop the server Hearken started. Sends SIGTERM so Ollama can unload models cleanly,
/// then SIGKILL if it hasn't exited after 3 s.
pub fn stop_if_owned() {
    let Some(mut owned) = OWNED.lock().unwrap().take() else {
        return;
    };
    log::info!("Ollama: stopping server started by Hearken (pid {})", owned.pid);

    let _ = Command::new("kill").args(["-TERM", &owned.pid.to_string()]).status();

    let deadline = Instant::now() + Duration::from_secs(3);
    let exited = loop {
        let gone = match owned.child.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(Some(_))),
            None => !process_is_ollama(owned.pid),
        };
        if gone || Instant::now() >= deadline {
            break gone;
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    if !exited {
        log::warn!("Ollama: did not exit after SIGTERM, killing");
        match owned.child.as_mut() {
            Some(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            None => {
                let _ = Command::new("kill").args(["-KILL", &owned.pid.to_string()]).status();
            }
        }
    }
    let _ = std::fs::remove_file(&owned.pid_file);
}
