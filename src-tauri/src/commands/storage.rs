use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::db::Db;
use crate::db::scopes::Scope;
use crate::error::{AppError, AppResult};
use crate::services::backup::BackupService;
use crate::services::engine::EngineService;
use crate::services::engine_install::EngineInstaller;
use crate::services::engine_process::EngineProcess;
use crate::services::jobs::JobService;
use crate::services::migration::{
    Blocker, EngineProbe, MigrationHost, MigrationPlan, MigrationProgress, MigrationReport,
    MigrationService, MigrationSink, Target, engine_target,
};
use crate::services::models::{ModelService, is_local_engine};
use crate::services::settings;
use crate::services::storage::StorageService;

/// 移行中は断るコマンド(docs/51 §5)。画像・保存先・ComfyUI・ユーザーを変える操作。
const BLOCKED_DURING_MIGRATION: &[&str] = &[
    "user_login",
    "user_logout",
    "user_delete",
    "backup_export",
    "backup_inspect",
    "backup_orphans",
    "backup_restore",
    "engine_managed_set",
    "engine_process_start",
    "engine_process_stop",
    "engine_process_restart",
    "engine_install_start",
    "engine_install_remove",
    "image_delete",
    "image_delete_non_favorite",
    "input_image_pick",
    "input_image_pick_many",
    "input_image_from_gallery",
    "mask_save",
    "models_download_start",
    "models_set_dir",
    "models_apply_dir_to_engine",
    "storage_change_images_dir",
    "settings_scope_set",
];

pub fn blocked_during_migration(command: &str) -> bool {
    BLOCKED_DURING_MIGRATION.contains(&command)
}

/// 移行中の印。IPC の入口・自動復旧・終了の確認が参照する。
#[derive(Clone, Default)]
pub struct MigrationFlag(pub Arc<AtomicBool>);

impl MigrationFlag {
    pub fn is_active(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// `storage://migration-progress` を送る。
pub struct TauriMigrationSink(pub AppHandle);

impl MigrationSink for TauriMigrationSink {
    fn progress(&self, p: &MigrationProgress) {
        let _ = self.0.emit("storage://migration-progress", p);
    }
}

pub struct TauriMigrationHost(pub AppHandle);

#[async_trait::async_trait]
impl MigrationHost for TauriMigrationHost {
    async fn hold_jobs(&self, hold: bool) {
        let jobs = Arc::clone(&self.0.state::<Arc<JobService>>());
        jobs.set_storage_hold(hold).await;
    }

    fn busy(&self, target: Target) -> Vec<Blocker> {
        let mut out = Vec::new();
        let downloading = self.0.state::<Arc<ModelService>>().has_active_download();
        if target == Target::Models && downloading {
            out.push(Blocker::DownloadActive);
        }
        if self.0.state::<Arc<BackupService>>().is_busy()
            || self.0.state::<Arc<EngineInstaller>>().is_running()
        {
            out.push(Blocker::Busy);
        }
        out
    }

    async fn models_source(&self) -> AppResult<Option<std::path::PathBuf>> {
        let models = Arc::clone(&self.0.state::<Arc<ModelService>>());
        Ok(models.resolve_dir().await?.0)
    }

    async fn engine_probe(&self) -> AppResult<EngineProbe> {
        let engine = self.0.state::<Arc<EngineService>>().engine()?;
        let local = is_local_engine(&engine.url());
        let running_argv = if local {
            // 応答しない ComfyUI で待たされないよう、短く打ち切る
            tokio::time::timeout(Duration::from_millis(1500), engine.launch_args())
                .await
                .ok()
                .and_then(Result::ok)
                .filter(|a| !a.is_empty())
        } else {
            None
        };
        let managed_running = self.0.state::<Arc<EngineProcess>>().has_child();
        let external_running = running_argv.is_some() && !managed_running;
        let argv = match running_argv {
            Some(a) => Some(a),
            // 停止中でも、起動管理の設定から ComfyUI の場所が分かる
            None => {
                let db = self.0.state::<Db>().inner().clone();
                let cfg = db.call(|c| settings::get_managed_engine(c)).await?;
                (cfg.enabled && !cfg.main_py.is_empty()).then(|| vec![cfg.main_py.clone()])
            }
        };
        let (config_file, block) = engine_target(argv.as_deref(), local);
        Ok(EngineProbe {
            managed_running,
            external_running,
            config_file,
            block,
        })
    }

    async fn stop_engine(&self) -> AppResult<()> {
        let proc = Arc::clone(&self.0.state::<Arc<EngineProcess>>());
        proc.stop().await?;
        if proc.has_child() {
            return Err(AppError::engine_proc(
                "ENGINE_BUSY",
                "ComfyUI を停止できませんでした",
            ));
        }
        Ok(())
    }

    async fn start_engine(&self) -> AppResult<()> {
        let proc = Arc::clone(&self.0.state::<Arc<EngineProcess>>());
        let db = self.0.state::<Db>().inner().clone();
        let cfg = db.call(|c| settings::get_managed_engine(c)).await?;
        proc.start(&cfg).await?;
        Ok(())
    }
}

type Migration<'a> = State<'a, Arc<MigrationService>>;

/// 画像を読めるのは保存先だけ(それ以外のフォルダは読めないまま)。
fn allow_images(app: &AppHandle, root: &std::path::Path) -> AppResult<()> {
    app.asset_protocol_scope()
        .allow_directory(root, true)
        .map_err(|e| AppError::Storage(e.to_string()))
}

/// 画像の保存先(全体 / 個別)を切り替える(ファイルは移さない)。
#[tauri::command]
pub async fn storage_change_images_dir(
    app: AppHandle,
    storage: State<'_, Arc<StorageService>>,
    scope: Scope,
    new_dir: Option<String>,
) -> AppResult<()> {
    let root = storage.change_images_dir(scope, new_dir).await?;
    allow_images(&app, &root)
}

#[tauri::command]
pub async fn storage_migration_plan(
    migration: Migration<'_>,
    target: Target,
    new_dir: Option<String>,
) -> AppResult<MigrationPlan> {
    migration.plan(target, new_dir).await
}

#[tauri::command]
pub async fn storage_migration_start(
    app: AppHandle,
    migration: Migration<'_>,
    target: Target,
    new_dir: Option<String>,
) -> AppResult<MigrationReport> {
    let report = migration.start(target, new_dir).await?;
    if target != Target::Models {
        // 移行は終わっているため、許可に失敗しても結果は返す(再起動すれば表示できる)
        let db = app.state::<Db>().inner().clone();
        let paths = app.state::<crate::AppPaths>().inner().clone();
        let allowed = match db.call(|c| settings::load(c)).await {
            Ok(s) => allow_images(&app, &paths.resolve_images_dir(&s)),
            Err(e) => Err(e),
        };
        if let Err(e) = allowed {
            tracing::error!(error = %e, "failed to allow the new image folder");
        }
    }
    Ok(report)
}

#[tauri::command]
pub fn storage_migration_cancel(migration: Migration<'_>) {
    migration.cancel();
}

/// 移行を中止して(切り替え以降なら完了を待って)アプリを終了する。
#[tauri::command]
pub async fn storage_migration_quit(app: AppHandle, migration: Migration<'_>) -> AppResult<()> {
    migration.cancel();
    while migration.is_active() {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_changes_but_not_reads_or_the_migration_controls() {
        for c in ["image_delete", "models_download_start", "user_logout"] {
            assert!(blocked_during_migration(c), "{c}");
        }
        for c in [
            "image_list",
            "settings_get",
            "storage_migration_cancel",
            "storage_migration_quit",
        ] {
            assert!(!blocked_during_migration(c), "{c}");
        }
    }
}
