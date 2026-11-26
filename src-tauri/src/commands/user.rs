//! ユーザー機能のコマンド(docs/36 §10)。ユーザー系は未ログインでも呼べる。

use std::sync::Arc;

use serde::Deserialize;
use tauri::{AppHandle, Manager, State};

use crate::AppPaths;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::services::assistant::AssistantService;
use crate::services::images::ImageService;
use crate::services::jobs::JobService;
use crate::services::settings;
use crate::services::users::{AuthKind, DeleteResult, UserService, UserSummary};

type Users<'a> = State<'a, Arc<UserService>>;

#[tauri::command]
pub async fn user_list(users: Users<'_>) -> AppResult<Vec<UserSummary>> {
    users.list().await
}

#[tauri::command]
pub async fn user_current(users: Users<'_>) -> AppResult<Option<UserSummary>> {
    users.current().await
}

/// 削除で一緒に消える、全体の保存先の中の画像フォルダ。
#[tauri::command]
pub async fn user_delete_preview(users: Users<'_>, id: String) -> AppResult<Option<String>> {
    users.delete_preview(id).await
}

#[tauri::command]
pub async fn user_create(users: Users<'_>, name: String, color: String) -> AppResult<UserSummary> {
    users.create(name, color).await
}

/// ログインして、そのユーザーの作業スペースを使えるようにする。
#[tauri::command]
pub async fn user_login(
    app: AppHandle,
    users: Users<'_>,
    jobs: State<'_, Arc<JobService>>,
    images: State<'_, Arc<ImageService>>,
    id: String,
    secret: Option<String>,
) -> AppResult<UserSummary> {
    let user = users.login(id, secret).await?;
    // 画像保存先を作り、asset プロトコルで読めるようにする(許可は取り消せないため、
    // 分離はデータ系コマンドがログイン中のユーザーのパスしか返さないことで保つ。docs/36 §6)
    let db = app.state::<Db>();
    let paths = app.state::<AppPaths>();
    let s = db.call(|c| settings::load(c)).await?;
    let dir = paths.resolve_images_dir(&s);
    // 保存先のドライブが外れていてもログインは続ける(全体の保存先だと全員が入れず、直せなくなるため)
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(error = %e, "image folder is not available");
    }
    if let Err(e) = app.asset_protocol_scope().allow_directory(&dir, true) {
        tracing::warn!(error = %e, "failed to allow the image folder");
    }
    // 前回の実行中ジョブを失敗扱いにしてから、そのユーザーの待機中ジョブを再開する
    if let Err(e) = jobs.recover().await {
        tracing::error!(error = %e, "job recovery failed");
    }
    jobs.wake_worker();
    let images = Arc::clone(&images);
    tauri::async_runtime::spawn(async move {
        match images.backfill_thumbnails().await {
            Ok(0) => {}
            Ok(n) => tracing::info!(count = n, "thumbnails backfilled"),
            Err(e) => tracing::warn!(error = %e, "thumbnail backfill failed"),
        }
    });
    Ok(user)
}

/// ログアウト。実行中のジョブがあれば、`cancel_jobs` が true のときだけ取り消して続ける。
#[tauri::command]
pub async fn user_logout(
    users: Users<'_>,
    jobs: State<'_, Arc<JobService>>,
    assistant: State<'_, Arc<AssistantService>>,
    cancel_jobs: Option<bool>,
) -> AppResult<()> {
    if !users.is_logged_in() {
        return Ok(());
    }
    if jobs.has_running() {
        if cancel_jobs != Some(true) {
            return Err(AppError::user(
                "USER_BUSY",
                "生成中のジョブがあります。取り消してから切り替えてください",
            ));
        }
        jobs.stop_running().await?;
    }
    // 相談・翻訳のストリームは、次のユーザーの DB に書かないよう先に止める
    assistant.cancel_all();
    users.logout()
}

#[tauri::command]
pub async fn user_rename(
    users: Users<'_>,
    id: String,
    name: String,
    secret: Option<String>,
) -> AppResult<UserSummary> {
    users.rename(id, name, secret).await
}

#[tauri::command]
pub async fn user_set_color(users: Users<'_>, id: String, color: String) -> AppResult<UserSummary> {
    users.set_color(id, color).await
}

#[tauri::command]
pub async fn user_delete(
    users: Users<'_>,
    id: String,
    secret: Option<String>,
    confirm_name: Option<String>,
) -> AppResult<DeleteResult> {
    users.delete(id, secret, confirm_name).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockSet {
    pub kind: AuthKind,
    pub secret: String,
    pub pin_length: Option<u32>,
    pub current: Option<String>,
}

#[tauri::command]
pub async fn user_lock_set(users: Users<'_>, input: LockSet) -> AppResult<UserSummary> {
    users
        .lock_set(input.kind, input.secret, input.pin_length, input.current)
        .await
}

#[tauri::command]
pub async fn user_lock_clear(users: Users<'_>, current: Option<String>) -> AppResult<UserSummary> {
    users.lock_clear(current).await
}
