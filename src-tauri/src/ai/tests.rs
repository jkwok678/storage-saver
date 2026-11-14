use super::provider::{extract_json_object, is_model_id, parse_cursor_models};
use super::recommend::{RawRecommendation, extract_recommendations, resolve_recommendations};
use super::stream::{JsonStream, visible_streaming_prefix};
use super::types::AiModel;

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

#[cfg(target_os = "macos")]
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
