//! Provider status, chat turns, and advisor events sent to the frontend.

use serde::{Deserialize, Serialize};

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
