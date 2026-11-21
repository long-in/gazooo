//! バックアップ(docs/35 §6)。ファイルのパスはフロントから受け取らず、Rust 側のダイアログで得る。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use crate::domain::job::JobStatus;
use crate::error::{AppError, AppResult};
use crate::services::backup::{
    BackupService, BackupSink, Estimate, ExportReport, Include, Manifest, OrphanReport,
    RestoreRequest,
};
use crate::services::jobs::JobService;
use crate::util::{compact_timestamp, now_ms};

type Svc<'a> = State<'a, Arc<BackupService>>;

/// `backup:progress` を送る。
pub struct TauriBackupSink(pub AppHandle);

impl BackupSink for TauriBackupSink {
    fn progress(&self, phase: &str, done: u64, total: u64) {
        let _ = self.0.emit(
            "backup:progress",
            serde_json::json!({ "phase": phase, "done": done, "total": total }),
        );
    }
}

fn export_file_name() -> String {
    format!(
        "gazooo-backup-{}.gazooo-backup",
        compact_timestamp(now_ms())
    )
}

#[tauri::command]
pub async fn backup_estimate(svc: Svc<'_>, include: Include) -> AppResult<Estimate> {
    svc.estimate(include).await
}

/// 保存ダイアログを Rust 側で開いてバックアップを書く。キャンセル・中止は `None`。
#[tauri::command]
pub async fn backup_export(
    app: AppHandle,
    svc: Svc<'_>,
    include: Include,
    include_api_key: bool,
) -> AppResult<Option<ExportReport>> {
    if !include.any() {
        return Err(AppError::validation(
            "include",
            "バックアップに含めるものを 1 つ以上選んでください",
        ));
    }
    let picked = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_file_name(export_file_name())
            .add_filter("gazooo backup", &["gazooo-backup"])
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
    svc.export(include, include_api_key, dest).await
}

/// ファイル選択ダイアログを Rust 側で開き、バックアップを検査する。キャンセルは `None`。
#[tauri::command]
pub async fn backup_inspect(app: AppHandle, svc: Svc<'_>) -> AppResult<Option<Manifest>> {
    let picked = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("gazooo backup", &["gazooo-backup"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(file) = picked else {
        return Ok(None);
    };
    let src: PathBuf = file
        .into_path()
        .map_err(|e| AppError::Storage(e.to_string()))?;
    svc.inspect(src).await.map(Some)
}

#[tauri::command]
pub async fn backup_orphans(svc: Svc<'_>, include: Include) -> AppResult<OrphanReport> {
    svc.orphans(include).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub restart_required: bool,
}

/// キューを止め、待機中・実行中のジョブを取り消して、終わるのを待つ(最大 30 秒)。
async fn stop_active_jobs(jobs: &Arc<JobService>) -> AppResult<()> {
    jobs.pause();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let page = jobs
            .list(vec![JobStatus::Queued, JobStatus::Running], None, Some(200))
            .await?;
        if page.items.is_empty() {
            return Ok(());
        }
        for job in &page.items {
            let _ = jobs.cancel(&job.id).await;
        }
        if Instant::now() >= deadline {
            return Err(AppError::backup(
                "BACKUP_BUSY",
                "実行中のジョブを停止できませんでした。終了してから再度お試しください",
            ));
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

#[tauri::command]
pub async fn backup_restore(
    svc: Svc<'_>,
    jobs: State<'_, Arc<JobService>>,
    include: Include,
    restore_api_key: bool,
    stop_jobs: bool,
    delete_orphans: bool,
) -> AppResult<RestoreResult> {
    let was_paused = jobs.is_paused();
    if stop_jobs && let Err(e) = stop_active_jobs(&jobs).await {
        if !was_paused {
            jobs.resume();
        }
        return Err(e);
    }
    let res = svc
        .restore(RestoreRequest {
            include,
            restore_api_key,
            delete_orphans,
        })
        .await;
    if res.is_err() && stop_jobs && !was_paused {
        jobs.resume();
    }
    res?;
    Ok(RestoreResult {
        restart_required: true,
    })
}

#[tauri::command]
pub fn backup_cancel(svc: Svc<'_>) {
    svc.cancel();
}

/// 復元の完了後にアプリを再起動する(DB の状態・画像保存先のスコープなどを初期化し直すため)。
#[tauri::command]
pub fn backup_restart(app: AppHandle) {
    app.restart();
}
