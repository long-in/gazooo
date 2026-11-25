//! モデル取得のコマンド(docs/12-model-downloader.md §8、docs/04-ipc-api.md「モデル取得」)。

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::error::AppResult;
use crate::services::models::{CatalogView, DirView, DownloadState, EngineDirStatus, ModelService};

type Svc<'a> = State<'a, Arc<ModelService>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResult {
    pub download_id: String,
}

#[derive(Serialize)]
pub struct VerifyResult {
    pub ok: bool,
}

#[tauri::command]
pub async fn models_catalog(svc: Svc<'_>) -> AppResult<CatalogView> {
    svc.catalog_view().await
}

#[tauri::command]
pub async fn models_download_start(
    svc: Svc<'_>,
    family_id: String,
    variant_id: String,
    consent: Option<bool>,
) -> AppResult<StartResult> {
    Ok(StartResult {
        download_id: svc
            .start(&family_id, &variant_id, consent.unwrap_or(false))
            .await?,
    })
}

#[tauri::command]
pub async fn models_download_cancel(
    svc: Svc<'_>,
    download_id: String,
    discard: Option<bool>,
) -> AppResult<()> {
    svc.cancel(&download_id, discard.unwrap_or(false))
}

#[tauri::command]
pub async fn models_download_list(svc: Svc<'_>) -> AppResult<Vec<DownloadState>> {
    svc.list()
}

#[tauri::command]
pub async fn models_verify(
    svc: Svc<'_>,
    family_id: String,
    variant_id: String,
) -> AppResult<VerifyResult> {
    Ok(VerifyResult {
        ok: svc.verify(&family_id, &variant_id).await?,
    })
}

#[tauri::command]
pub async fn models_engine_dir_status(svc: Svc<'_>) -> AppResult<EngineDirStatus> {
    svc.engine_dir_status().await
}

#[tauri::command]
pub async fn models_apply_dir_to_engine(
    svc: Svc<'_>,
    clear: Option<bool>,
) -> AppResult<EngineDirStatus> {
    svc.apply_dir_to_engine(clear.unwrap_or(false)).await
}

#[tauri::command]
pub async fn models_set_dir(svc: Svc<'_>, path: Option<String>) -> AppResult<DirView> {
    svc.set_dir(path).await
}
