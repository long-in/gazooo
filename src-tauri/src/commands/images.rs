use std::path::PathBuf;
use std::sync::Arc;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::db::Db;
use crate::db::images::ImageScope;
use crate::domain::job::{ImageItem, Page};
use crate::error::{AppError, AppResult};
use crate::services::images::{ImageDetail, ImageService};
use crate::services::settings;

/// 一覧の範囲(docs/40 §7): `"all"` | `"unassigned"` | `{ "projectId": "…" }`。省略は `"all"`。
#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
pub enum ScopeArg {
    Keyword(String),
    Project {
        #[serde(rename = "projectId")]
        project_id: String,
    },
}

impl ScopeArg {
    fn into_scope(self) -> AppResult<ImageScope> {
        match self {
            Self::Keyword(k) if k == "all" => Ok(ImageScope::All),
            Self::Keyword(k) if k == "unassigned" => Ok(ImageScope::Unassigned),
            Self::Keyword(_) => Err(AppError::validation("scope", "scope が不正です")),
            Self::Project { project_id } => Ok(ImageScope::Project(project_id)),
        }
    }
}

#[tauri::command]
pub async fn image_list(
    images: State<'_, Arc<ImageService>>,
    query: Option<String>,
    favorites_only: Option<bool>,
    scope: Option<ScopeArg>,
    cursor: Option<String>,
    limit: Option<u32>,
) -> AppResult<Page<ImageItem>> {
    let scope = scope
        .map(ScopeArg::into_scope)
        .transpose()?
        .unwrap_or_default();
    images
        .list(query, favorites_only.unwrap_or(false), scope, cursor, limit)
        .await
}

/// 画像を別のプロジェクトへ移す(`projectId` が `null` なら未分類へ)。
#[tauri::command]
pub async fn image_move(
    images: State<'_, Arc<ImageService>>,
    ids: Vec<String>,
    project_id: Option<String>,
) -> AppResult<()> {
    images.move_to_project(ids, project_id).await
}

#[tauri::command]
pub async fn image_get(images: State<'_, Arc<ImageService>>, id: String) -> AppResult<ImageDetail> {
    images.get(&id).await
}

#[tauri::command]
pub async fn image_set_favorite(
    images: State<'_, Arc<ImageService>>,
    id: String,
    value: bool,
) -> AppResult<()> {
    images.set_favorite(&id, value).await
}

#[tauri::command]
pub async fn image_delete(
    images: State<'_, Arc<ImageService>>,
    ids: Vec<String>,
    delete_files: bool,
) -> AppResult<()> {
    images.delete(ids, delete_files).await
}

#[derive(Debug, serde::Serialize)]
pub struct NonFavoriteCount {
    pub total: i64,
    pub favorites: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct NonFavoriteDeleted {
    pub deleted: usize,
    pub failed: usize,
}

/// スコープ内のお気に入り以外・お気に入りの件数(確認ダイアログ用。docs/49)。
#[tauri::command]
pub async fn image_count_non_favorite(
    images: State<'_, Arc<ImageService>>,
    scope: Option<ScopeArg>,
) -> AppResult<NonFavoriteCount> {
    let scope = scope
        .map(ScopeArg::into_scope)
        .transpose()?
        .unwrap_or_default();
    let (total, favorites) = images.count_non_favorite(scope).await?;
    Ok(NonFavoriteCount { total, favorites })
}

/// スコープ内のお気に入り以外を削除する(ファイルはゴミ箱へ。docs/49)。
#[tauri::command]
pub async fn image_delete_non_favorite(
    images: State<'_, Arc<ImageService>>,
    scope: Option<ScopeArg>,
    expected_count: i64,
) -> AppResult<NonFavoriteDeleted> {
    let scope = scope
        .map(ScopeArg::into_scope)
        .transpose()?
        .unwrap_or_default();
    let (deleted, failed) = images.delete_non_favorite(scope, expected_count).await?;
    Ok(NonFavoriteDeleted { deleted, failed })
}

#[tauri::command]
pub async fn image_export(
    images: State<'_, Arc<ImageService>>,
    id: String,
    dest_path: String,
) -> AppResult<()> {
    images.export(&id, &PathBuf::from(dest_path)).await
}

/// エクスプローラー / Finder で画像ファイルを選択した状態で開く。
#[tauri::command]
pub async fn image_reveal(images: State<'_, Arc<ImageService>>, id: String) -> AppResult<()> {
    let path = images.original_path(&id).await?;
    tauri_plugin_opener::reveal_item_in_dir(&path).map_err(|e| AppError::Storage(e.to_string()))
}

/// 実行中の一括ダウンロード(中止用のフラグ)。docs/44。
#[derive(Default)]
pub struct ExportJobs(Mutex<HashMap<String, Arc<AtomicBool>>>);

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ExportProgress {
    job_id: String,
    done: usize,
    total: usize,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ExportDone {
    job_id: String,
    saved: usize,
    failed: Vec<crate::services::images::ExportFailure>,
    cancelled: bool,
    error: Option<String>,
}

/// フォルダ選択ダイアログを開き、選ばれたら選択中の画像の原寸を保存し始める。
/// キャンセルなら `None`。進捗・結果はイベントで通知する(docs/44 §4)。
#[tauri::command]
pub async fn image_export_many(
    app: AppHandle,
    images: State<'_, Arc<ImageService>>,
    db: State<'_, Db>,
    jobs: State<'_, ExportJobs>,
    ids: Vec<String>,
) -> AppResult<Option<String>> {
    if ids.is_empty() {
        return Err(AppError::validation("ids", "画像を選んでください"));
    }
    let last = db.call(|c| settings::get_export_dir(c)).await?;
    let handle = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        let mut d = handle.dialog().file();
        if let Some(dir) = last.filter(|d| std::path::Path::new(d).is_dir()) {
            d = d.set_directory(dir);
        }
        d.blocking_pick_folder()
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(dir) = picked else {
        return Ok(None);
    };
    let dir = dir
        .into_path()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    let saved_dir = dir.to_string_lossy().into_owned();
    db.call(move |c| settings::set_export_dir(c, &saved_dir))
        .await?;

    let job_id = uuid::Uuid::now_v7().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut m) = jobs.0.lock() {
        m.insert(job_id.clone(), cancel.clone());
    }
    let svc = images.inner().clone();
    let (jid, emit_app) = (job_id.clone(), app.clone());
    tokio::spawn(async move {
        let prog_app = emit_app.clone();
        let prog_id = jid.clone();
        let res = svc
            .export_many(ids, dir, cancel, move |done, total| {
                let _ = prog_app.emit(
                    "image-export://progress",
                    ExportProgress {
                        job_id: prog_id.clone(),
                        done,
                        total,
                    },
                );
            })
            .await;
        if let Some(jobs) = emit_app.try_state::<ExportJobs>()
            && let Ok(mut m) = jobs.0.lock()
        {
            m.remove(&jid);
        }
        let done = match res {
            Ok(r) => ExportDone {
                job_id: jid,
                saved: r.saved,
                failed: r.failed,
                cancelled: r.cancelled,
                error: None,
            },
            Err(e) => ExportDone {
                job_id: jid,
                saved: 0,
                failed: vec![],
                cancelled: false,
                error: Some(e.to_string()),
            },
        };
        let _ = emit_app.emit("image-export://done", done);
    });
    Ok(Some(job_id))
}

/// 一括ダウンロードを中止する(実行中の 1 枚は完了させ、未着手は処理しない)。
#[tauri::command]
pub async fn image_export_cancel(jobs: State<'_, ExportJobs>, job_id: String) -> AppResult<()> {
    if let Ok(m) = jobs.0.lock()
        && let Some(f) = m.get(&job_id)
    {
        f.store(true, Ordering::SeqCst);
    }
    Ok(())
}

/// 前回の一括ダウンロードの保存先を開く(パスはフロントから受け取らない)。
#[tauri::command]
pub async fn image_export_open_dir(db: State<'_, Db>) -> AppResult<()> {
    let dir = db.call(|c| settings::get_export_dir(c)).await?;
    let Some(dir) = dir.filter(|d| std::path::Path::new(d).is_dir()) else {
        return Err(AppError::validation("dir", "保存先のフォルダがありません"));
    };
    tauri_plugin_opener::open_path(dir, None::<&str>).map_err(|e| AppError::Storage(e.to_string()))
}
