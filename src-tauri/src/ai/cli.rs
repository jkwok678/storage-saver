//! Finding the Claude and Cursor CLIs and running them with a timeout.

use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Run `cmd` to completion like `Command::output`, killing it if it outlives `timeout`.
pub(super) fn output_with_timeout(cmd: &mut Command, timeout: Duration) -> std::io::Result<Output> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Timed out waiting for the CLI to respond",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    })
}

fn augmented_path() -> String {
    let mut dirs = Vec::new();
    if let Ok(home) = env::var("HOME") {
        dirs.push(format!("{home}/.local/bin"));
        dirs.push(format!("{home}/.claude/local"));
        dirs.push(format!("{home}/.cursor/bin"));
    }
    dirs.push("/opt/homebrew/bin".into());
    dirs.push("/usr/local/bin".into());
    dirs.push("/usr/bin".into());
    dirs.push("/bin".into());
    dirs.push("/usr/sbin".into());
    dirs.push("/sbin".into());
    if let Ok(existing) = env::var("PATH") {
        for part in existing.split(':') {
            if !part.is_empty() && !dirs.iter().any(|d| d == part) {
                dirs.push(part.to_string());
            }
        }
    }
    dirs.join(":")
}

pub(super) fn find_binary(names: &[&str]) -> Option<PathBuf> {
    let path_var = augmented_path();
    for name in names {
        for dir in path_var.split(':') {
            let candidate = Path::new(dir).join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub(super) fn command_with_env(binary: &Path) -> Command {
    let mut cmd = Command::new(binary);
    cmd.env("PATH", augmented_path());
    cmd.env("NO_COLOR", "1");
    cmd.env("TERM", "dumb");
    cmd
}
