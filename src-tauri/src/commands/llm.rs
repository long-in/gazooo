use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::error::AppResult;
use crate::services::llm::{LlmService, LlmStatus};

type Svc<'a> = State<'a, Arc<LlmService>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallStarted {
    pub install_id: String,
}

#[tauri::command]
pub async fn llm_status(svc: Svc<'_>) -> AppResult<LlmStatus> {
    svc.status().await
}

#[tauri::command]
pub async fn llm_install(svc: Svc<'_>) -> AppResult<InstallStarted> {
    Ok(InstallStarted {
        install_id: svc.inner().install_start().await?,
    })
}

#[tauri::command]
pub fn llm_install_cancel(svc: Svc<'_>, install_id: String) {
    svc.install_cancel(&install_id);
}

#[tauri::command]
pub async fn llm_remove(svc: Svc<'_>) -> AppResult<LlmStatus> {
    svc.remove().await
}

#[tauri::command]
pub async fn llm_start(svc: Svc<'_>) -> AppResult<LlmStatus> {
    svc.inner().start().await
}

#[tauri::command]
pub async fn llm_stop(svc: Svc<'_>) -> AppResult<LlmStatus> {
    svc.stop().await
}

#[tauri::command]
pub fn llm_logs(svc: Svc<'_>) -> Vec<String> {
    svc.logs(300)
}

#[tauri::command]
pub async fn llm_update_settings(
    svc: Svc<'_>,
    idle_minutes: Option<u32>,
    release_before_generation: Option<bool>,
) -> AppResult<LlmStatus> {
    svc.update_settings(idle_minutes, release_before_generation)
        .await
}
