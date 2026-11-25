use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::error::AppResult;
use crate::services::presets::{Preset, PresetService};

#[tauri::command]
pub async fn preset_list(presets: State<'_, Arc<PresetService>>) -> AppResult<Vec<Preset>> {
    presets.list().await
}

#[tauri::command]
pub async fn preset_save(
    presets: State<'_, Arc<PresetService>>,
    id: Option<String>,
    name: String,
    params: Value,
) -> AppResult<Preset> {
    presets.save(id, name, params).await
}

#[tauri::command]
pub async fn preset_delete(presets: State<'_, Arc<PresetService>>, id: String) -> AppResult<()> {
    presets.delete(&id).await
}
