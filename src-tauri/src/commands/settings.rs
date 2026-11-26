use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::db::Db;
use crate::db::scopes::{self, Group, Layer, Scope};
use crate::error::{AppError, AppResult};
use crate::services::engine::EngineService;
use crate::services::settings::{self, Settings, SettingsPatch};

#[tauri::command]
pub async fn settings_get(db: State<'_, Db>, scope: Option<Scope>) -> AppResult<Settings> {
    match Layer::from(scope) {
        // 全体設定は、ログイン前(ユーザーの選択画面のテーマと言語)にも読む
        Layer::Global => {
            db.call_device(|c| settings::load_in(c, Layer::Global))
                .await
        }
        Layer::Effective => db.call(|c| settings::load(c)).await,
    }
}

/// 個別設定にしている組(docs/52 §3)。
#[tauri::command]
pub async fn settings_scope_get(db: State<'_, Db>) -> AppResult<Vec<Group>> {
    db.call(|c| {
        let mut out = Vec::new();
        for g in Group::ALL {
            if scopes::is_overridden(c, g)? {
                out.push(g);
            }
        }
        Ok(out)
    })
    .await
}

/// 組を個別設定にする / 全体設定に戻す。
#[tauri::command]
pub async fn settings_scope_set(
    db: State<'_, Db>,
    group: Group,
    overridden: bool,
) -> AppResult<()> {
    db.call(move |c| {
        let tx = c.transaction()?;
        scopes::set_override(&tx, group, overridden)?;
        tx.commit()?;
        Ok(())
    })
    .await
}

/// 全体設定の組を標準値に戻す。
#[tauri::command]
pub async fn settings_scope_reset_global(db: State<'_, Db>, group: Group) -> AppResult<()> {
    db.call(move |c| scopes::reset_global(c, group)).await
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
            "保存先の変更は、設定の「保存先」タブの専用の操作で行ってください",
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
