use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::services::engine::EngineService;
use crate::services::settings::{self, Settings, SettingsPatch};

#[tauri::command]
pub async fn settings_get(db: State<'_, Db>) -> AppResult<Settings> {
    db.call(|c| settings::load(c)).await
}

#[tauri::command]
pub async fn settings_update(
    app: AppHandle,
    db: State<'_, Db>,
    engine: State<'_, Arc<EngineService>>,
    patch: SettingsPatch,
) -> AppResult<Settings> {
    // 画像の保存先は、既存画像の扱いを伴うため専用コマンド(storage_change_images_dir)で変更する
    if patch.images_dir.is_some() {
        return Err(AppError::validation(
            "imagesDir",
            "保存先の変更は専用の操作(既存画像の移動の選択つき)で行ってください",
        ));
    }
    let updated = db.call(move |c| settings::update(c, patch)).await?;
    // ComfyUI の URL が変わった場合は、新しい接続先へ即座に再接続する
    if engine.set_url(&updated.comfyui_url)? {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(engine) = app.try_state::<Arc<EngineService>>() {
                let _ = engine.refresh(&app, true).await;
            }
        });
    }
    Ok(updated)
}
