//! 調査用の詳細ログ(docs/31 §6)。

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::services::debug_log::{self, DebugLog, DebugStatus};
use crate::services::settings;

type Svc<'a> = State<'a, Arc<DebugLog>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
}

async fn status_of(svc: &Arc<DebugLog>) -> AppResult<DebugStatus> {
    let svc = Arc::clone(svc);
    tokio::task::spawn_blocking(move || svc.status())
        .await
        .map_err(|e| AppError::internal(e.to_string()))
}

/// 環境の説明(API キーは含めない)。
pub async fn environment(app: &AppHandle, db: &Db, enabled: bool) -> AppResult<String> {
    let (url, managed) = db
        .call(|c| {
            Ok((
                settings::load(c)?.comfyui_url,
                settings::get_managed_engine(c)?.enabled,
            ))
        })
        .await?;
    Ok(debug_log::describe_environment(
        &app.package_info().version.to_string(),
        &url,
        managed,
        enabled,
    ))
}

#[tauri::command]
pub async fn debug_status(svc: Svc<'_>) -> AppResult<DebugStatus> {
    status_of(svc.inner()).await
}

#[tauri::command]
pub async fn debug_set_enabled(
    app: AppHandle,
    db: State<'_, Db>,
    svc: Svc<'_>,
    enabled: bool,
) -> AppResult<DebugStatus> {
    db.call(move |c| settings::set_debug_enabled(c, enabled))
        .await?;
    svc.set_enabled(enabled);
    if enabled {
        svc.log_environment(&environment(&app, &db, true).await?);
    }
    status_of(svc.inner()).await
}

/// 保存ダイアログを **Rust 側** で開き、ログ一式の ZIP を書き出す。キャンセルは `None`。
#[tauri::command]
pub async fn debug_export_logs(
    app: AppHandle,
    db: State<'_, Db>,
    svc: Svc<'_>,
) -> AppResult<Option<ExportResult>> {
    let info = environment(&app, &db, svc.is_enabled()).await?;
    let dialog_app = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        dialog_app
            .dialog()
            .file()
            .set_file_name(debug_log::export_file_name())
            .add_filter("ZIP", &["zip"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(file) = picked else {
        return Ok(None);
    };
    let dest: PathBuf = file
        .into_path()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    let svc = Arc::clone(svc.inner());
    let out = dest.clone();
    tokio::task::spawn_blocking(move || svc.export(&out, &info))
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;
    Ok(Some(ExportResult {
        path: dest.to_string_lossy().into_owned(),
    }))
}

#[tauri::command]
pub fn debug_open_log_dir(svc: Svc<'_>) -> AppResult<()> {
    tauri_plugin_opener::open_path(svc.dir(), None::<&str>)
        .map_err(|e| AppError::Storage(e.to_string()))
}

#[tauri::command]
pub async fn debug_clear_logs(svc: Svc<'_>) -> AppResult<DebugStatus> {
    let s = Arc::clone(svc.inner());
    tokio::task::spawn_blocking(move || s.clear())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    status_of(svc.inner()).await
}
