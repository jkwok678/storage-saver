//! Claude and Cursor `stream-json` events, folded into the text shown while they answer.

/// Reply text and final result read from Claude's or Cursor's `stream-json` output, one line at
/// a time. Claude streams text as `stream_event` deltas; Cursor sends each piece of text as an
/// `assistant` event of its own and marks tool use with `tool_call` events.
#[derive(Default)]
pub(super) struct JsonStream {
    /// Whether `assistant` events are pieces of text (Cursor) rather than whole messages (Claude).
    assistant_pieces: bool,
    /// Everything written so far, turns separated by a blank line.
    pub(super) text: String,
    /// Where the current turn starts in `text`.
    turn_start: usize,
    /// Whether the current turn's text already arrived in pieces.
    turn_streamed: bool,
    pub(super) result: Option<String>,
    pub(super) is_error: bool,
}

impl JsonStream {
    pub(super) fn new(assistant_pieces: bool) -> Self {
        Self {
            assistant_pieces,
            ..Self::default()
        }
    }

    /// Read one line of output. Returns true when `text` changed.
    pub(super) fn feed(&mut self, line: &str) -> bool {
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

pub(super) fn visible_streaming_prefix(text: &str) -> &str {
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
