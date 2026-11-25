use std::sync::Arc;

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::{AppError, AppResult};
use crate::services::inputs::{InputImage, InputService};

/// ファイル選択ダイアログを **Rust 側** で開き、選ばれた PNG / JPEG / WebP / HEIC を参照画像として取り込む。
/// パスをフロントエンドから受け取らない(WebView から任意ファイルを読ませないため)。
/// キャンセルされたら `None`。
#[tauri::command]
pub async fn input_image_pick(
    app: AppHandle,
    inputs: State<'_, Arc<InputService>>,
) -> AppResult<Option<InputImage>> {
    let picked = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Image", &["png", "jpg", "jpeg", "webp", "heic", "heif"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(file) = picked else {
        return Ok(None);
    };
    let path = file
        .into_path()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    inputs.import_file(path).await.map(Some)
}

/// 複数の PNG / JPEG / WebP / HEIC を選び、参照画像として取り込む(顔の入れ替えの対象画像。docs/42 §3)。
/// `input_image_pick` と同じく、パスはフロントエンドから受け取らない。キャンセルされたら空。
/// 1 回で取り込む枚数は `MAX_PICK_MANY` まで(超えた分は無視する)。
#[tauri::command]
pub async fn input_image_pick_many(
    app: AppHandle,
    inputs: State<'_, Arc<InputService>>,
) -> AppResult<Vec<InputImage>> {
    let picked = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Image", &["png", "jpg", "jpeg", "webp", "heic", "heif"])
            .blocking_pick_files()
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    let mut out = Vec::new();
    for file in picked.unwrap_or_default().into_iter().take(MAX_PICK_MANY) {
        let path = file
            .into_path()
            .map_err(|e| AppError::Storage(e.to_string()))?;
        out.push(inputs.import_file(path).await?);
    }
    Ok(out)
}

/// `input_image_pick_many` で一度に取り込む枚数の上限(docs/42 Q-348)。
const MAX_PICK_MANY: usize = 50;

/// ギャラリーの画像を参照画像として取り込む。
#[tauri::command]
pub async fn input_image_from_gallery(
    inputs: State<'_, Arc<InputService>>,
    image_id: String,
) -> AppResult<InputImage> {
    inputs.import_gallery(&image_id).await
}

/// マスクエディタが描いた PNG(リクエスト本文のバイト列)を検証して保存する(docs/13 §3.5)。
#[tauri::command]
pub async fn mask_save(
    request: tauri::ipc::Request<'_>,
    inputs: State<'_, Arc<InputService>>,
) -> AppResult<InputImage> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(AppError::validation("mask", "マスクのデータが不正です"));
    };
    inputs.save_mask(bytes.clone()).await
}

/// 既存の参照画像の情報を取得する(存在確認を兼ねる。無ければ `IMAGE_NOT_FOUND`)。
#[tauri::command]
pub async fn input_image_get(
    inputs: State<'_, Arc<InputService>>,
    path: String,
) -> AppResult<InputImage> {
    inputs.get(&path).await
}
