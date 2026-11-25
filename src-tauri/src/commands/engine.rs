use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::engine::{EngineModels, EngineStatus};
use crate::error::AppResult;
use crate::services::engine::EngineService;

#[tauri::command]
pub async fn engine_status(
    app: AppHandle,
    engine: State<'_, Arc<EngineService>>,
) -> AppResult<EngineStatus> {
    engine.refresh(&app, false).await
}

#[tauri::command]
pub async fn engine_list_models(engine: State<'_, Arc<EngineService>>) -> AppResult<EngineModels> {
    engine.list_models().await
}

#[tauri::command]
pub async fn engine_reconnect(
    app: AppHandle,
    engine: State<'_, Arc<EngineService>>,
) -> AppResult<EngineStatus> {
    engine.refresh(&app, true).await
}
