//! One advisor question: stream the reply, then check the paths it recommends.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tauri::ipc::Channel;

use super::cli::{command_with_env, find_binary};
use super::prompt::{build_conversation_prompt, build_system_instructions};
use super::provider::{CLAUDE_EFFORTS, is_model_id};
use super::recommend::{extract_recommendations, resolve_recommendations};
use super::stream::{JsonStream, visible_streaming_prefix};
use super::types::{AiQueryEvent, ChatTurn};
use crate::{platform, scan};

static AI_GENERATION: AtomicU64 = AtomicU64::new(0);
static RUNNING_CHILD: Mutex<Option<platform::ChildHandle>> = Mutex::new(None);

pub(super) async fn query_ai(
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

pub(super) fn stop_ai_query() {
    AI_GENERATION.fetch_add(1, Ordering::AcqRel);
    kill_running_child();
}

fn kill_running_child() {
    if let Ok(mut guard) = RUNNING_CHILD.lock()
        && let Some(handle) = guard.take()
    {
        platform::terminate(&handle);
    }
}

/// Forget `handle` unless a newer query has already registered its own child.
fn clear_running_child(handle: &platform::ChildHandle) {
    if let Ok(mut guard) = RUNNING_CHILD.lock()
        && guard.as_ref() == Some(handle)
    {
        *guard = None;
    }
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

    let home_dir = platform::home_dir()
        .map(|home| home.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/Users".into());
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

    let handle = platform::track_child(&child);
    if let Ok(mut guard) = RUNNING_CHILD.lock() {
        *guard = Some(handle.clone());
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
    clear_running_child(&handle);
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
