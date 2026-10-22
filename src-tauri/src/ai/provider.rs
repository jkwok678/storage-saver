//! Claude Code and Cursor: whether they are signed in, and which models they offer.

use std::process::Stdio;
use std::time::Duration;

use super::cli::{command_with_env, find_binary, output_with_timeout};
use super::types::{AiModel, AiProviderStatus};

/// `auth status` should answer at once; a hung CLI must not leave the panel checking forever.
const STATUS_TIMEOUT: Duration = Duration::from_secs(20);
/// Long enough to finish signing in through the browser.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// Claude Code resolves these aliases to the latest model of each family.
const CLAUDE_MODELS: &[(&str, &str)] = &[
    ("fable", "Fable"),
    ("opus", "Opus"),
    ("sonnet", "Sonnet"),
    ("haiku", "Haiku"),
];

/// Levels `claude --effort` accepts. Cursor puts the effort in the model id instead.
pub(super) const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

pub(super) async fn ai_providers_status() -> Result<Vec<AiProviderStatus>, String> {
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

pub(super) async fn connect_ai_provider(provider: String) -> Result<AiProviderStatus, String> {
    match tauri::async_runtime::spawn_blocking(move || run_connect_provider(&provider)).await {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
}

pub(super) async fn disconnect_ai_provider(provider: String) -> Result<AiProviderStatus, String> {
    match tauri::async_runtime::spawn_blocking(move || run_disconnect_provider(&provider)).await {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
}

pub(super) async fn ai_models(provider: String) -> Result<Vec<AiModel>, String> {
    match tauri::async_runtime::spawn_blocking(move || list_ai_models(&provider)).await {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
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
pub(super) fn parse_cursor_models(text: &str) -> Vec<AiModel> {
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
pub(super) fn is_model_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-[]=,:".contains(c))
}

pub(super) fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    (start <= end).then_some(&s[start..=end])
}
