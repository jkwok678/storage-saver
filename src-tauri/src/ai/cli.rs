//! Finding the Claude and Cursor CLIs and running them with a timeout.

use std::env;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use crate::platform;

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

fn augmented_path() -> OsString {
    let mut dirs = platform::cli_search_dirs();
    if let Some(existing) = env::var_os("PATH") {
        for part in env::split_paths(&existing) {
            if !part.as_os_str().is_empty() && !dirs.contains(&part) {
                dirs.push(part);
            }
        }
    }
    env::join_paths(dirs).unwrap_or_default()
}

pub(super) fn find_binary(names: &[&str]) -> Option<PathBuf> {
    let path_var = augmented_path();
    for name in names {
        for dir in env::split_paths(&path_var) {
            for extension in platform::CLI_EXTENSIONS {
                let candidate = dir.join(format!("{name}{extension}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

pub(super) fn command_with_env(binary: &Path) -> Command {
    let mut cmd = Command::new(binary);
    platform::prepare_command(&mut cmd);
    cmd.env("PATH", augmented_path());
    cmd.env("NO_COLOR", "1");
    cmd.env("TERM", "dumb");
    cmd
}
