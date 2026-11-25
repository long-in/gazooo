use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, AppResult};
use crate::services::storage::{MoveReport, StorageService};

/// 画像の保存先を変更する(既存画像を移動するかは `move_existing` で選ぶ)。
#[tauri::command]
pub async fn storage_change_images_dir(
    app: AppHandle,
    storage: State<'_, Arc<StorageService>>,
    new_dir: Option<String>,
    move_existing: bool,
) -> AppResult<MoveReport> {
    let (root, report) = storage.change_images_dir(new_dir, move_existing).await?;
    // 新しい保存先の画像を WebView から読めるようにする(それ以外のフォルダは読めないまま)
    app.asset_protocol_scope()
        .allow_directory(&root, true)
        .map_err(|e| AppError::Storage(e.to_string()))?;
    Ok(report)
}
