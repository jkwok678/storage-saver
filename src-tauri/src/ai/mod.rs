//! Advisor questions answered by the Claude Code or Cursor CLI.

mod cli;
mod prompt;
mod provider;
mod query;
mod recommend;
mod stream;
mod types;

use tauri::ipc::Channel;

pub use types::{AiModel, AiProviderStatus, AiQueryEvent, AiRecommendation, ChatTurn};

#[tauri::command]
pub async fn ai_providers_status() -> Result<Vec<AiProviderStatus>, String> {
    provider::ai_providers_status().await
}

#[tauri::command]
pub async fn connect_ai_provider(provider: String) -> Result<AiProviderStatus, String> {
    provider::connect_ai_provider(provider).await
}

#[tauri::command]
pub async fn disconnect_ai_provider(provider: String) -> Result<AiProviderStatus, String> {
    provider::disconnect_ai_provider(provider).await
}

#[tauri::command]
pub async fn ai_models(provider: String) -> Result<Vec<AiModel>, String> {
    provider::ai_models(provider).await
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
    query::query_ai(
        provider,
        prompt,
        current_path,
        history,
        model,
        effort,
        on_event,
    )
    .await
}

#[tauri::command]
pub fn stop_ai_query() {
    query::stop_ai_query();
}

#[cfg(test)]
mod tests;
