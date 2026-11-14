//! The Claude CLI's arguments, and how the prompts reach it.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ChildStdin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// How the prompts are handed to the CLI.
pub(super) enum PromptMode<'a> {
    /// Both prompts are arguments.
    Argv,
    /// The system prompt is read from `system_file` and the conversation from stdin, which keeps
    /// the command line short. Windows caps it far below what a prompt needs.
    Stdin { system_file: &'a Path },
}

pub(super) struct Invocation {
    pub(super) args: Vec<OsString>,
    pub(super) stdin: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn claude_invocation(
    mode: PromptMode,
    system: &str,
    conversation: &str,
    home_dir: &str,
    work_dir: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> Invocation {
    let mut args: Vec<OsString> = [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--no-session-persistence",
        "--tools",
        "Read,Glob,Grep",
        "--allowedTools",
        "Read,Glob,Grep",
        "--permission-prompts",
        "none",
    ]
    .map(OsString::from)
    .into();
    match &mode {
        PromptMode::Argv => args.extend(["--system-prompt".into(), system.into()]),
        PromptMode::Stdin { system_file } => {
            args.extend(["--system-prompt-file".into(), (*system_file).into()]);
        }
    }
    args.extend(["--add-dir".into(), home_dir.into()]);
    if work_dir != home_dir {
        args.extend(["--add-dir".into(), work_dir.into()]);
    }
    if let Some(model) = model {
        args.extend(["--model".into(), model.into()]);
    }
    if let Some(effort) = effort {
        args.extend(["--effort".into(), effort.into()]);
    }
    let stdin = match mode {
        PromptMode::Argv => {
            args.extend(["--".into(), conversation.into()]);
            None
        }
        PromptMode::Stdin { .. } => Some(conversation.to_owned()),
    };
    Invocation { args, stdin }
}

/// Write `text` to the child's stdin on a thread, then close it. The child may exit without
/// reading everything, so a failed write is not an error.
pub(super) fn feed_stdin(stdin: ChildStdin, text: String) {
    std::thread::spawn(move || {
        let mut stdin = stdin;
        let _ = stdin.write_all(text.as_bytes());
    });
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temp file holding the system prompt, removed when dropped.
pub(super) struct TempPromptFile {
    path: PathBuf,
}

impl TempPromptFile {
    pub(super) fn create(contents: &str) -> std::io::Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let name = format!(
            "storage-saver-prompt-{}-{}-{nanos}.txt",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let guard = Self { path };
        file.write_all(contents.as_bytes())?;
        Ok(guard)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempPromptFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[OsString]) -> Vec<&str> {
        args.iter().map(|a| a.to_str().unwrap()).collect()
    }

    #[test]
    fn argv_mode_keeps_the_prompts_as_arguments() {
        let inv = claude_invocation(
            PromptMode::Argv,
            "SYS\nprompt",
            "question",
            "/Users/me",
            "/Users/me/work",
            Some("opus"),
            Some("high"),
        );
        assert_eq!(inv.stdin, None);
        assert_eq!(
            strings(&inv.args),
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--no-session-persistence",
                "--tools",
                "Read,Glob,Grep",
                "--allowedTools",
                "Read,Glob,Grep",
                "--permission-prompts",
                "none",
                "--system-prompt",
                "SYS\nprompt",
                "--add-dir",
                "/Users/me",
                "--add-dir",
                "/Users/me/work",
                "--model",
                "opus",
                "--effort",
                "high",
                "--",
                "question",
            ]
        );
    }

    #[test]
    fn argv_mode_skips_a_repeated_work_dir_and_unset_options() {
        let inv = claude_invocation(PromptMode::Argv, "s", "q", "/h", "/h", None, None);
        assert_eq!(
            &strings(&inv.args)[12..],
            ["--system-prompt", "s", "--add-dir", "/h", "--", "q"]
        );
    }

    #[test]
    fn stdin_mode_keeps_the_command_line_short_and_single_line() {
        let big = "line of prompt text\n".repeat(3000);
        assert!(big.len() > 50_000);
        let file = Path::new("C:\\Users\\me\\AppData\\Local\\Temp\\prompt.txt");
        let inv = claude_invocation(
            PromptMode::Stdin { system_file: file },
            &big,
            &big,
            "C:\\Users\\me",
            "C:\\Users\\me\\work",
            Some("opus"),
            Some("high"),
        );
        for arg in &inv.args {
            let arg = arg.to_str().unwrap();
            assert!(!arg.contains('\n') && !arg.contains('\r'));
            assert!(arg.len() <= 1024);
        }
        let args = strings(&inv.args);
        assert!(args.contains(&"--system-prompt-file"));
        assert!(!args.contains(&"--system-prompt"));
        assert!(!args.contains(&"--"));
        assert_eq!(inv.stdin.as_deref(), Some(big.as_str()));
    }

    #[test]
    fn temp_prompt_file_is_removed_on_drop() {
        let file = TempPromptFile::create("hello").unwrap();
        let path = file.path().to_path_buf();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
        drop(file);
        assert!(!path.exists());
    }

    #[test]
    fn temp_prompt_files_get_distinct_paths() {
        let a = TempPromptFile::create("a").unwrap();
        let b = TempPromptFile::create("b").unwrap();
        assert_ne!(a.path(), b.path());
    }
}
