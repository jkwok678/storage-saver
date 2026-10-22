use crate::scan::{self, StorageSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;

/// `auth status` should answer at once; a hung CLI must not leave the panel checking forever.
const STATUS_TIMEOUT: Duration = Duration::from_secs(20);
/// Long enough to finish signing in through the browser.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

static AI_GENERATION: AtomicU64 = AtomicU64::new(0);
static RUNNING_PID: Mutex<Option<u32>> = Mutex::new(None);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderStatus {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub connected: bool,
    pub account: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AiModel {
    pub id: String,
    pub label: String,
}

/// Claude Code resolves these aliases to the latest model of each family.
const CLAUDE_MODELS: &[(&str, &str)] = &[
    ("fable", "Fable"),
    ("opus", "Opus"),
    ("sonnet", "Sonnet"),
    ("haiku", "Haiku"),
];

/// Levels `claude --effort` accepts. Cursor puts the effort in the model id instead.
const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurn {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRecommendation {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub bytes: Option<u64>,
    pub safety: String,
    pub reason: String,
    pub exists: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct RawRecommendation {
    path: String,
    #[serde(default = "default_safety")]
    safety: String,
    #[serde(default)]
    reason: String,
}

fn default_safety() -> String {
    "caution".into()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
pub enum AiQueryEvent {
    Status {
        message: String,
    },
    Chunk {
        text: String,
    },
    Done {
        answer: String,
        recommendations: Vec<AiRecommendation>,
    },
}

#[tauri::command]
pub async fn ai_providers_status() -> Result<Vec<AiProviderStatus>, String> {
    match tauri::async_runtime::spawn_blocking(|| {
        let (claude, cursor) = std::thread::scope(|scope| {
            let claude_job = scope.spawn(check_claude_status);
            let cursor_job = scope.spawn(check_cursor_status);
            (
                claude_job.join().unwrap_or_else(|_| AiProviderStatus {
                    id: "claude".into(),
                    name: "Claude Code".into(),
                    installed: false,
                    connected: false,
                    account: None,
                    detail: Some("Failed to check status".into()),
                }),
                cursor_job.join().unwrap_or_else(|_| AiProviderStatus {
                    id: "cursor".into(),
                    name: "Cursor".into(),
                    installed: false,
                    connected: false,
                    account: None,
                    detail: Some("Failed to check status".into()),
                }),
            )
        });
        Ok(vec![claude, cursor])
    })
    .await
    {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
}

#[tauri::command]
pub async fn connect_ai_provider(provider: String) -> Result<AiProviderStatus, String> {
    match tauri::async_runtime::spawn_blocking(move || run_connect_provider(&provider)).await {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
}

#[tauri::command]
pub async fn disconnect_ai_provider(provider: String) -> Result<AiProviderStatus, String> {
    match tauri::async_runtime::spawn_blocking(move || run_disconnect_provider(&provider)).await {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
}

#[tauri::command]
pub async fn ai_models(provider: String) -> Result<Vec<AiModel>, String> {
    match tauri::async_runtime::spawn_blocking(move || list_ai_models(&provider)).await {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
}

#[tauri::command]
pub async fn query_ai(
    provider: String,
    prompt: String,
    current_path: Option<String>,
    history: Vec<ChatTurn>,
    model: Option<String>,
    effort: Option<String>,
    on_event: Channel<AiQueryEvent>,
) -> Result<(), String> {
    match tauri::async_runtime::spawn_blocking(move || {
        run_ai_query(
            &provider,
            &prompt,
            current_path.as_deref(),
            &history,
            model.as_deref(),
            effort.as_deref(),
            on_event,
        )
    })
    .await
    {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
}

#[tauri::command]
pub fn stop_ai_query() {
    AI_GENERATION.fetch_add(1, Ordering::AcqRel);
    kill_running_child();
}

fn kill_running_child() {
    if let Ok(mut guard) = RUNNING_PID.lock()
        && let Some(pid) = guard.take()
    {
        // SAFETY: Sending SIGTERM to the tracked child process ID. The id is cleared before the
        // child is reaped, so it still names our child here.
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }
}

/// Forget `pid` unless a newer query has already registered its own child.
fn clear_running_pid(pid: u32) {
    if let Ok(mut guard) = RUNNING_PID.lock()
        && *guard == Some(pid)
    {
        *guard = None;
    }
}

/// Run `cmd` to completion like `Command::output`, killing it if it outlives `timeout`.
fn output_with_timeout(cmd: &mut Command, timeout: Duration) -> std::io::Result<Output> {
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

fn find_binary(names: &[&str]) -> Option<PathBuf> {
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

fn command_with_env(binary: &Path) -> Command {
    let mut cmd = Command::new(binary);
    cmd.env("PATH", augmented_path());
    cmd.env("NO_COLOR", "1");
    cmd.env("TERM", "dumb");
    cmd
}

fn check_claude_status() -> AiProviderStatus {
    let Some(binary) = find_binary(&["claude"]) else {
        return AiProviderStatus {
            id: "claude".into(),
            name: "Claude Code".into(),
            installed: false,
            connected: false,
            account: None,
            detail: Some("Claude Code CLI (`claude`) not found".into()),
        };
    };

    let output = output_with_timeout(
        command_with_env(&binary)
            .args(["auth", "status", "--json"])
            .stdin(Stdio::null()),
        STATUS_TIMEOUT,
    );

    let Ok(output) = output else {
        return AiProviderStatus {
            id: "claude".into(),
            name: "Claude Code".into(),
            installed: true,
            connected: false,
            account: None,
            detail: Some("Could not run `claude auth status`".into()),
        };
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(json) = extract_json_object(&stdout)
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(json)
    {
        let logged_in = val
            .get("loggedIn")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let email = val
            .get("email")
            .or_else(|| val.get("accountEmail"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned);
        let sub = val
            .get("subscriptionType")
            .or_else(|| val.get("authMethod"))
            .and_then(|v| v.as_str())
            .filter(|s| *s != "none")
            .map(ToOwned::to_owned);
        let account = email.or(sub.clone());
        let detail = if logged_in {
            Some(
                sub.map(|s| format!("Connected ({s})"))
                    .unwrap_or_else(|| "Connected".into()),
            )
        } else {
            Some("Not signed in".into())
        };
        return AiProviderStatus {
            id: "claude".into(),
            name: "Claude Code".into(),
            installed: true,
            connected: logged_in,
            account,
            detail,
        };
    }

    let connected = output.status.success();
    AiProviderStatus {
        id: "claude".into(),
        name: "Claude Code".into(),
        installed: true,
        connected,
        account: None,
        detail: Some(if connected {
            "Connected".into()
        } else {
            "Not signed in".into()
        }),
    }
}

fn check_cursor_status() -> AiProviderStatus {
    let Some(binary) = find_binary(&["cursor-agent", "agent"]) else {
        return AiProviderStatus {
            id: "cursor".into(),
            name: "Cursor".into(),
            installed: false,
            connected: false,
            account: None,
            detail: Some("Cursor CLI (`cursor-agent`) not found".into()),
        };
    };

    let output = output_with_timeout(
        command_with_env(&binary)
            .args(["status", "--format", "json"])
            .stdin(Stdio::null()),
        STATUS_TIMEOUT,
    );

    let Ok(output) = output else {
        return AiProviderStatus {
            id: "cursor".into(),
            name: "Cursor".into(),
            installed: true,
            connected: false,
            account: None,
            detail: Some("Could not run `cursor-agent status`".into()),
        };
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(json) = extract_json_object(&stdout)
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(json)
    {
        let authenticated = val
            .get("isAuthenticated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let account = val
            .get("email")
            .or_else(|| val.get("username"))
            .or_else(|| val.get("user"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned);
        let detail = val
            .get("message")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
            .or_else(|| {
                Some(if authenticated {
                    "Connected".into()
                } else {
                    "Not signed in".into()
                })
            });
        return AiProviderStatus {
            id: "cursor".into(),
            name: "Cursor".into(),
            installed: true,
            connected: authenticated,
            account,
            detail,
        };
    }

    let connected = output.status.success() && !stdout.to_lowercase().contains("not logged in");
    AiProviderStatus {
        id: "cursor".into(),
        name: "Cursor".into(),
        installed: true,
        connected,
        account: None,
        detail: Some(if connected {
            "Connected".into()
        } else {
            "Not signed in".into()
        }),
    }
}

fn run_connect_provider(provider: &str) -> Result<AiProviderStatus, String> {
    match provider {
        "claude" => {
            let binary = find_binary(&["claude"]).ok_or_else(|| {
                "Claude Code CLI (`claude`) is not installed on this Mac.".to_string()
            })?;
            let output = output_with_timeout(
                command_with_env(&binary)
                    .args(["auth", "login", "--claudeai"])
                    .stdin(Stdio::null()),
                LOGIN_TIMEOUT,
            )
            .map_err(|err| format!("Claude login did not finish: {err}"))?;
            let status = check_claude_status();
            if !status.connected && !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let msg = if !stderr.is_empty() { stderr } else { stdout };
                if !msg.is_empty() {
                    return Err(msg);
                }
            }
            Ok(status)
        }
        "cursor" => {
            let binary = find_binary(&["cursor-agent", "agent"]).ok_or_else(|| {
                "Cursor CLI (`cursor-agent`) is not installed on this Mac.".to_string()
            })?;
            let output = output_with_timeout(
                command_with_env(&binary)
                    .args(["login"])
                    .stdin(Stdio::null()),
                LOGIN_TIMEOUT,
            )
            .map_err(|err| format!("Cursor login did not finish: {err}"))?;
            let status = check_cursor_status();
            if !status.connected && !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let msg = if !stderr.is_empty() { stderr } else { stdout };
                if !msg.is_empty() {
                    return Err(msg);
                }
            }
            Ok(status)
        }
        other => Err(format!("Unknown AI provider: {other}")),
    }
}

fn run_disconnect_provider(provider: &str) -> Result<AiProviderStatus, String> {
    match provider {
        "claude" => {
            if let Some(binary) = find_binary(&["claude"]) {
                let _ = output_with_timeout(
                    command_with_env(&binary)
                        .args(["auth", "logout"])
                        .stdin(Stdio::null()),
                    STATUS_TIMEOUT,
                );
            }
            Ok(check_claude_status())
        }
        "cursor" => {
            if let Some(binary) = find_binary(&["cursor-agent", "agent"]) {
                let _ = output_with_timeout(
                    command_with_env(&binary)
                        .args(["logout"])
                        .stdin(Stdio::null()),
                    STATUS_TIMEOUT,
                );
            }
            Ok(check_cursor_status())
        }
        other => Err(format!("Unknown AI provider: {other}")),
    }
}

fn list_ai_models(provider: &str) -> Result<Vec<AiModel>, String> {
    match provider {
        "claude" => Ok(CLAUDE_MODELS
            .iter()
            .map(|(id, label)| AiModel {
                id: (*id).into(),
                label: (*label).into(),
            })
            .collect()),
        "cursor" => {
            let binary = find_binary(&["cursor-agent", "agent"]).ok_or_else(|| {
                "Cursor CLI (`cursor-agent`) was not found. Install Cursor CLI first.".to_string()
            })?;
            let output = output_with_timeout(
                command_with_env(&binary)
                    .arg("--list-models")
                    .stdin(Stdio::null()),
                STATUS_TIMEOUT,
            )
            .map_err(|err| format!("Could not list Cursor models: {err}"))?;
            let models = parse_cursor_models(&String::from_utf8_lossy(&output.stdout));
            if models.is_empty() {
                return Err("Cursor did not list any models. Check that you are signed in.".into());
            }
            Ok(models)
        }
        other => Err(format!("Unknown AI provider: {other}")),
    }
}

/// Reads `cursor-agent --list-models`, one `id - Label` per line.
fn parse_cursor_models(text: &str) -> Vec<AiModel> {
    text.lines()
        .filter_map(|line| {
            // Some labels end in zero-width spaces.
            let clean: String = line
                .chars()
                .filter(|c| !c.is_control() && !matches!(c, '\u{200b}'..='\u{200d}' | '\u{feff}'))
                .collect();
            let (id, label) = clean.split_once(" - ")?;
            let id = id.trim();
            if !is_model_id(id) {
                return None;
            }
            Some(AiModel {
                id: id.into(),
                label: label.split_whitespace().collect::<Vec<_>>().join(" "),
            })
        })
        .collect()
}

/// A model id goes on the command line, so it must not read as a flag.
fn is_model_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-[]=,:".contains(c))
}

fn run_ai_query(
    provider: &str,
    user_prompt: &str,
    current_path: Option<&str>,
    history: &[ChatTurn],
    model: Option<&str>,
    effort: Option<&str>,
    on_event: Channel<AiQueryEvent>,
) -> Result<(), String> {
    let model = model.filter(|m| !m.is_empty());
    if let Some(model) = model
        && !is_model_id(model)
    {
        return Err(format!("Unknown model: {model}"));
    }
    let effort = effort.filter(|e| !e.is_empty());
    if let Some(effort) = effort
        && !CLAUDE_EFFORTS.contains(&effort)
    {
        return Err(format!("Unknown effort level: {effort}"));
    }

    let generation = AI_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    kill_running_child();

    let cancelled = || AI_GENERATION.load(Ordering::Acquire) != generation;

    let _ = on_event.send(AiQueryEvent::Status {
        message: "Gathering scanned storage data…".into(),
    });

    let snapshot = scan::storage_snapshot(current_path);
    if cancelled() {
        return Ok(());
    }

    let system_context = build_system_instructions(&snapshot);
    let conversation_prompt = build_conversation_prompt(user_prompt, history);

    let home_dir = env::var("HOME").unwrap_or_else(|_| "/Users".into());
    let work_dir = current_path
        .filter(|p| Path::new(p).is_dir())
        .unwrap_or(&home_dir);

    let mut cmd = match provider {
        "claude" => {
            let binary = find_binary(&["claude"]).ok_or_else(|| {
                "Claude Code CLI (`claude`) was not found. Install Claude Code first.".to_string()
            })?;
            let _ = on_event.send(AiQueryEvent::Status {
                message: "Querying Claude Code…".into(),
            });
            let mut c = command_with_env(&binary);
            c.current_dir(work_dir);
            // Text output arrives only once the whole answer is done, so stream JSON events instead.
            c.args([
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
                &system_context,
                "--add-dir",
                &home_dir,
            ]);
            if work_dir != home_dir {
                c.args(["--add-dir", work_dir]);
            }
            if let Some(model) = model {
                c.args(["--model", model]);
            }
            if let Some(effort) = effort {
                c.args(["--effort", effort]);
            }
            c.arg("--").arg(&conversation_prompt);
            c
        }
        "cursor" => {
            let binary = find_binary(&["cursor-agent", "agent"]).ok_or_else(|| {
                "Cursor CLI (`cursor-agent`) was not found. Install Cursor CLI first.".to_string()
            })?;
            let _ = on_event.send(AiQueryEvent::Status {
                message: "Querying Cursor Agent…".into(),
            });
            let combined = format!("{system_context}\n\n---\n\n{conversation_prompt}");
            let mut c = command_with_env(&binary);
            c.current_dir(work_dir);
            c.args([
                "-p",
                "--output-format",
                "stream-json",
                "--stream-partial-output",
                "--mode",
                "ask",
                "--trust",
                "--workspace",
                work_dir,
            ]);
            if work_dir != home_dir {
                c.args(["--add-dir", &home_dir]);
            }
            if let Some(model) = model {
                c.args(["--model", model]);
            }
            c.arg("--").arg(combined);
            c
        }
        other => return Err(format!("Unsupported AI provider: {other}")),
    };

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|err| format!("Failed to start {provider}: {err}"))?;

    let pid = child.id();
    if let Ok(mut guard) = RUNNING_PID.lock() {
        *guard = Some(pid);
    }
    // A stop that landed before the id was registered had no child to kill.
    if cancelled() {
        let _ = child.kill();
    }

    let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
    let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;

    let stderr_thread = std::thread::spawn(move || {
        let mut err_text = String::new();
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            if line.starts_with("Tip: You can start the Cursor CLI") {
                continue;
            }
            if !err_text.is_empty() {
                err_text.push('\n');
            }
            err_text.push_str(&line);
        }
        err_text
    });

    let mut stream = JsonStream::new(provider == "cursor");
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        if cancelled() {
            let _ = child.kill();
            break;
        }
        let Ok(line) = line else { break };
        if !stream.feed(&line) {
            continue;
        }
        let visible = visible_streaming_prefix(&stream.text);
        let _ = on_event.send(AiQueryEvent::Chunk {
            text: visible.to_string(),
        });
    }

    // Cleared before reaping, so a stop never signals a process id the system has reused.
    clear_running_pid(pid);
    let status = child.wait().map_err(|err| err.to_string())?;
    let stderr_output = stderr_thread.join().unwrap_or_default();

    if cancelled() {
        return Ok(());
    }

    if stream.is_error || !status.success() {
        let msg = [
            stream.result.as_deref(),
            Some(stderr_output.as_str()),
            Some(&stream.text),
        ]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|text| !text.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            format!(
                "{provider} exited with status {status}. Check that your subscription is signed in."
            )
        });
        return Err(msg);
    }

    // The result holds the final reply alone, without the notes written between tool calls.
    let final_output = match stream.result {
        Some(result) if !result.trim().is_empty() => result,
        _ => stream.text,
    };

    let _ = on_event.send(AiQueryEvent::Status {
        message: "Verifying recommended paths on disk…".into(),
    });

    let (answer, raw_recs) = extract_recommendations(&final_output);
    let recommendations = resolve_recommendations(raw_recs, &cancelled);
    if cancelled() {
        return Ok(());
    }

    let _ = on_event.send(AiQueryEvent::Done {
        answer,
        recommendations,
    });

    Ok(())
}

fn build_system_instructions(snapshot: &StorageSnapshot) -> String {
    let mut ctx = String::with_capacity(4096);
    ctx.push_str(
        "You are the Storage Saver AI Advisor for macOS. Your job is to help the user understand what is taking up disk space on their Mac and identify what they can and should delete.\n\n\
         Rules:\n\
         1. Base your analysis on the scanned storage data below, and use your read-only file/directory tools if you need to inspect specific directories further.\n\
         2. Never attempt to delete, modify, or move files yourself. The user will review and move items to Trash using Storage Saver's UI.\n\
         3. Clearly distinguish between:\n\
            - `safe`: Safe to delete anytime without losing personal data (e.g. ~/Library/Caches/*, Xcode DerivedData, npm/cargo/homebrew caches, old logs, ~/.Trash).\n\
            - `caution`: Worth reviewing before deleting (e.g. large files in ~/Downloads, old .dmg/.zip installers, Docker/VM images, old project node_modules/target build folders, large videos).\n\
            - System/protected paths (/System, /Library, active app bundles) that should NOT be deleted.\n\
         4. Mention concrete file/folder paths and their human-readable sizes in your explanation.\n\
         5. IMPORTANT: Whenever you recommend specific files or folders that the user can delete or review for deletion, include a fenced `recommendations` JSON block at the VERY END of your response in this exact format:\n\
         ```recommendations\n\
         [\n\
           {\n\
             \"path\": \"/Users/example/Library/Caches/com.example.app\",\n\
             \"safety\": \"safe\",\n\
             \"reason\": \"App cache that will be regenerated automatically if needed.\"\n\
           }\n\
         ]\n\
         ```\n\
         Only include real, absolute paths on this Mac in the `recommendations` block.\n\n\
         === SCANNED STORAGE DATA ===\n",
    );

    if let Some(disk) = &snapshot.disk {
        let pct = if disk.total > 0 {
            (disk.used as f64 / disk.total as f64) * 100.0
        } else {
            0.0
        };
        ctx.push_str(&format!(
            "Boot Disk: {:.1}% full — {} used of {} total ({} available)\n\n",
            pct,
            human_bytes(disk.used),
            human_bytes(disk.total),
            human_bytes(disk.available)
        ));
    }

    if let Some(curr) = &snapshot.current_path {
        ctx.push_str(&format!("Currently Viewed Folder: {curr}\n"));
        if !snapshot.current_items.is_empty() {
            ctx.push_str("Items in Current Folder (sorted by size):\n");
            for item in &snapshot.current_items {
                let kind = if item.is_dir { "dir" } else { "file" };
                ctx.push_str(&format!(
                    "  - {} ({}, {}): {}\n",
                    item.path,
                    kind,
                    human_bytes(item.bytes),
                    item.name
                ));
            }
            ctx.push('\n');
        }
    }

    if !snapshot.notable_locations.is_empty() {
        ctx.push_str("Known User Cache & Cleanup Locations:\n");
        for item in &snapshot.notable_locations {
            ctx.push_str(&format!(
                "  - {} ({}): {}\n",
                item.path,
                human_bytes(item.bytes),
                item.name
            ));
        }
        ctx.push('\n');
    }

    if !snapshot.top_folders.is_empty() {
        ctx.push_str("Largest Scanned Folders (total size including subfolders):\n");
        for item in &snapshot.top_folders {
            ctx.push_str(&format!(
                "  - {} (total: {}, direct files: {})\n",
                item.path,
                human_bytes(item.bytes),
                human_bytes(item.own_bytes)
            ));
        }
        ctx.push('\n');
    }

    if !snapshot.dense_folders.is_empty() {
        ctx.push_str("Folders with Largest Direct File Usage (own bytes):\n");
        for item in &snapshot.dense_folders {
            ctx.push_str(&format!(
                "  - {} (direct files: {}, total: {})\n",
                item.path,
                human_bytes(item.own_bytes),
                human_bytes(item.bytes)
            ));
        }
        ctx.push('\n');
    }

    if !snapshot.largest_files.is_empty() {
        ctx.push_str("Largest Individual Files Found:\n");
        for file in &snapshot.largest_files {
            ctx.push_str(&format!(
                "  - {} ({})\n",
                file.path,
                human_bytes(file.bytes)
            ));
        }
        ctx.push('\n');
    }

    ctx
}

fn build_conversation_prompt(user_prompt: &str, history: &[ChatTurn]) -> String {
    if history.is_empty() {
        return user_prompt.to_string();
    }
    let mut out = String::from("Conversation so far:\n");
    for turn in history.iter().rev().take(8).rev() {
        let speaker = if turn.role == "assistant" {
            "Assistant"
        } else {
            "User"
        };
        out.push_str(&format!("{speaker}: {}\n\n", turn.content));
    }
    out.push_str(&format!("User's new question:\n{user_prompt}"));
    out
}

/// Reply text and final result read from Claude's or Cursor's `stream-json` output, one line at
/// a time. Claude streams text as `stream_event` deltas; Cursor sends each piece of text as an
/// `assistant` event of its own and marks tool use with `tool_call` events.
#[derive(Default)]
struct JsonStream {
    /// Whether `assistant` events are pieces of text (Cursor) rather than whole messages (Claude).
    assistant_pieces: bool,
    /// Everything written so far, turns separated by a blank line.
    text: String,
    /// Where the current turn starts in `text`.
    turn_start: usize,
    /// Whether the current turn's text already arrived in pieces.
    turn_streamed: bool,
    result: Option<String>,
    is_error: bool,
}

impl JsonStream {
    fn new(assistant_pieces: bool) -> Self {
        Self {
            assistant_pieces,
            ..Self::default()
        }
    }

    /// Read one line of output. Returns true when `text` changed.
    fn feed(&mut self, line: &str) -> bool {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            return false;
        };
        match val.get("type").and_then(|v| v.as_str()) {
            Some("stream_event") => {
                let event = &val["event"];
                match event["type"].as_str() {
                    Some("message_start") => {
                        self.turn_streamed = false;
                        false
                    }
                    Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
                        let Some(text) = event["delta"]["text"].as_str() else {
                            return false;
                        };
                        if !self.turn_streamed {
                            self.start_turn();
                            self.turn_streamed = true;
                        }
                        self.text.push_str(text);
                        true
                    }
                    _ => false,
                }
            }
            Some("assistant") if self.assistant_pieces => {
                let piece = message_text(&val);
                if piece.is_empty() {
                    return false;
                }
                if !self.turn_streamed {
                    self.start_turn();
                    self.turn_streamed = true;
                }
                // The whole message may come again once its pieces are done.
                if piece.len() > 1 && piece == self.text[self.turn_start..] {
                    return false;
                }
                self.text.push_str(&piece);
                true
            }
            // The whole message again. Only needed when its text did not stream.
            Some("assistant") if !self.turn_streamed => {
                let text = message_text(&val);
                if text.is_empty() {
                    return false;
                }
                self.start_turn();
                self.text.push_str(&text);
                true
            }
            Some("tool_call") => {
                self.turn_streamed = false;
                false
            }
            Some("result") => {
                self.is_error = val["is_error"].as_bool().unwrap_or(false);
                self.result = val["result"].as_str().map(ToOwned::to_owned);
                false
            }
            _ => false,
        }
    }

    fn start_turn(&mut self) {
        if !self.text.is_empty() {
            self.text.push_str("\n\n");
        }
        self.turn_start = self.text.len();
    }
}

fn message_text(event: &serde_json::Value) -> String {
    event["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect()
}

fn visible_streaming_prefix(text: &str) -> &str {
    if let Some(idx) = text.find("```recommendations") {
        text[..idx].trim_end()
    } else if let Some(idx) = text.find("```json-recommendations") {
        text[..idx].trim_end()
    } else {
        // The fence may have arrived only partway, e.g. "```recomm".
        let last_line = text.rfind('\n').map_or(0, |idx| idx + 1);
        let tail = &text[last_line..];
        if tail.starts_with("```")
            && ["```recommendations", "```json-recommendations"]
                .iter()
                .any(|tag| tag.starts_with(tail))
        {
            text[..last_line].trim_end()
        } else {
            text
        }
    }
}

fn extract_recommendations(raw: &str) -> (String, Vec<RawRecommendation>) {
    for tag in ["```recommendations", "```json-recommendations"] {
        if let Some(start) = raw.find(tag) {
            let before = raw[..start].trim();
            let after_tag = &raw[start + tag.len()..];
            let (json_block, after_fence) = match after_tag.find("```") {
                Some(end) => (&after_tag[..end], after_tag[end + 3..].trim()),
                None => (after_tag, ""),
            };
            let parsed = parse_recommendation_array(json_block);
            let mut clean = before.to_string();
            if !after_fence.is_empty() {
                if !clean.is_empty() {
                    clean.push_str("\n\n");
                }
                clean.push_str(after_fence);
            }
            return (clean, parsed);
        }
    }

    // Fallback: check if there's a trailing ```json block containing path/reason objects
    if let Some(start) = raw.rfind("```json") {
        let after_tag = &raw[start + 7..];
        if let Some(end) = after_tag.find("```") {
            let json_block = &after_tag[..end];
            let parsed = parse_recommendation_array(json_block);
            if !parsed.is_empty() {
                let before = raw[..start].trim().to_string();
                return (before, parsed);
            }
        }
    }

    (raw.trim().to_string(), Vec::new())
}

fn parse_recommendation_array(block: &str) -> Vec<RawRecommendation> {
    let trimmed = block.trim();
    if let Ok(list) = serde_json::from_str::<Vec<RawRecommendation>>(trimmed) {
        return list;
    }
    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']'))
        && start < end
        && let Ok(list) = serde_json::from_str::<Vec<RawRecommendation>>(&trimmed[start..=end])
    {
        return list;
    }
    Vec::new()
}

fn resolve_recommendations(
    raw_recs: Vec<RawRecommendation>,
    cancelled: &dyn Fn() -> bool,
) -> Vec<AiRecommendation> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let home = env::var("HOME").ok();

    for raw in raw_recs {
        if cancelled() {
            break;
        }
        let mut path_str = raw.path.trim().to_string();
        if path_str.starts_with("~/")
            && let Some(ref h) = home
        {
            path_str = format!("{h}/{}", &path_str[2..]);
        }
        if !path_str.starts_with('/') || path_str == "/" {
            continue;
        }
        while path_str.len() > 1 && path_str.ends_with('/') {
            path_str.pop();
        }
        let path = Path::new(&path_str);
        // `..` would point somewhere other than the path shown, and protected folders cannot be
        // trashed anyway, so neither is worth offering.
        if path.components().any(|c| c == Component::ParentDir) || scan::is_protected_path(path) {
            continue;
        }
        if !seen.insert(path_str.clone()) {
            continue;
        }
        let path = Path::new(&path_str);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path_str.clone());
        let inspected = scan::inspect_path_size(path);
        let (bytes, is_dir, exists) = match inspected {
            Some((b, d)) => (Some(b), d, true),
            None => (None, false, false),
        };
        let safety = if raw.safety.eq_ignore_ascii_case("safe") {
            "safe".into()
        } else {
            "caution".into()
        };
        out.push(AiRecommendation {
            path: path_str,
            name,
            is_dir,
            bytes,
            safety,
            reason: raw.reason.trim().to_string(),
            exists,
        });
    }
    // Sort existing items with largest byte sizes first
    out.sort_by(|a, b| {
        b.exists
            .cmp(&a.exists)
            .then_with(|| b.bytes.unwrap_or(0).cmp(&a.bytes.unwrap_or(0)))
    });
    out
}

fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    (start <= end).then_some(&s[start..=end])
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut val = bytes as f64;
    let mut unit = 0;
    while val >= 1024.0 && unit < UNITS.len() - 1 {
        val /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{val:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cursor_model_list() {
        let listing = "Available models\n\n\
            auto - Auto (default)\n\
            claude-opus-5-5-high - Claude Opus 5.5 1M High\n\
            grok-4.7-low-fast - Grok 4.7  Low Fast\u{200b}\u{200b}\n\
            --force - not a model\n";
        assert_eq!(
            parse_cursor_models(listing),
            vec![
                AiModel {
                    id: "auto".into(),
                    label: "Auto (default)".into()
                },
                AiModel {
                    id: "claude-opus-5-5-high".into(),
                    label: "Claude Opus 5.5 1M High".into(),
                },
                AiModel {
                    id: "grok-4.7-low-fast".into(),
                    label: "Grok 4.7 Low Fast".into()
                },
            ]
        );
    }

    #[test]
    fn model_ids_cannot_be_flags() {
        assert!(is_model_id("claude-opus-4-8[context=1m,effort=high]"));
        assert!(!is_model_id("--dangerously-skip-permissions"));
        assert!(!is_model_id("opus; rm -rf"));
        assert!(!is_model_id(""));
    }

    #[test]
    fn extracts_recommendations_fence_and_cleans_answer() {
        let sample = "Here are the best folders to clean up:\n\n\
            1. **Caches** (2.4 GB)\n\n\
            ```recommendations\n\
            [\n\
              {\n\
                \"path\": \"/tmp\",\n\
                \"safety\": \"safe\",\n\
                \"reason\": \"Temporary files\"\n\
              }\n\
            ]\n\
            ```";
        let (answer, recs) = extract_recommendations(sample);
        assert_eq!(
            answer,
            "Here are the best folders to clean up:\n\n1. **Caches** (2.4 GB)"
        );
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].path, "/tmp");
        assert_eq!(recs[0].safety, "safe");
        assert_eq!(recs[0].reason, "Temporary files");
    }

    #[test]
    fn extracts_json_object_ignoring_cli_tips() {
        let noisy =
            "Tip: You can start the Cursor CLI with `agent`.\n{\n  \"isAuthenticated\": false\n}\n";
        let extracted = extract_json_object(noisy).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(extracted).unwrap();
        assert_eq!(parsed["isAuthenticated"], false);
    }

    #[test]
    fn hides_recommendations_block_while_streaming() {
        let partial = "You can delete your caches.\n\n```recommendations\n[{\"path\":";
        assert_eq!(
            visible_streaming_prefix(partial),
            "You can delete your caches."
        );
        let half_fence = "You can delete your caches.\n\n```recomm";
        assert_eq!(
            visible_streaming_prefix(half_fence),
            "You can delete your caches."
        );
    }

    #[test]
    fn reads_claude_stream_json_deltas_and_result() {
        let mut stream = JsonStream::new(false);
        let lines = [
            r#"{"type":"system","subtype":"init"}"#,
            r#"{"type":"stream_event","event":{"type":"message_start"}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Let me "}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"look."}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Let me look."}]}}"#,
            r#"{"type":"stream_event","event":{"type":"message_start"}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{"}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Caches use 2 GB."}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Caches use 2 GB."}"#,
        ];
        let changed: Vec<bool> = lines.iter().map(|line| stream.feed(line)).collect();
        assert_eq!(
            changed,
            [false, false, true, true, false, false, false, true, false]
        );
        assert_eq!(stream.text, "Let me look.\n\nCaches use 2 GB.");
        assert_eq!(stream.result.as_deref(), Some("Caches use 2 GB."));
        assert!(!stream.is_error);
    }

    #[test]
    fn reads_cursor_stream_json_pieces_and_result() {
        let mut stream = JsonStream::new(true);
        let lines = [
            "Tip: You can start the Cursor CLI with `agent`.",
            r#"{"type":"system","subtype":"init"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Let me "}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"check."}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Let me check."}]}}"#,
            r#"{"type":"tool_call","subtype":"started"}"#,
            r#"{"type":"tool_call","subtype":"completed"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Caches use 2 GB."}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Let me check.

Caches use 2 GB."}"#,
        ];
        let changed: Vec<bool> = lines.iter().map(|line| stream.feed(line)).collect();
        assert_eq!(
            changed,
            [false, false, true, true, false, false, false, true, false]
        );
        assert_eq!(stream.text, "Let me check.\n\nCaches use 2 GB.");
        assert!(!stream.is_error);
    }

    #[test]
    fn reads_claude_stream_json_error_result() {
        let mut stream = JsonStream::new(false);
        stream.feed(
            r#"{"type":"result","is_error":true,"result":"Invalid API key · Please run /login"}"#,
        );
        assert!(stream.is_error);
        assert_eq!(
            stream.result.as_deref(),
            Some("Invalid API key · Please run /login")
        );
    }

    #[test]
    fn drops_protected_and_parent_dir_recommendations() {
        let raw = |path: &str| RawRecommendation {
            path: path.into(),
            safety: "safe".into(),
            reason: String::new(),
        };
        let recs = resolve_recommendations(
            vec![
                raw("/System"),
                raw("/tmp/../System/Library"),
                raw("~/Library"),
            ],
            &|| false,
        );
        assert!(recs.is_empty(), "{recs:?}");
    }
}
