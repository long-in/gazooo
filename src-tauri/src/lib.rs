mod commands;
mod db;
mod domain;
mod engine;
mod error;
mod services;
mod util;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;

use crate::db::Db;
use crate::services::assistant::AssistantService;
use crate::services::assistant::anthropic::AnthropicClient;
use crate::services::engine::EngineService;
use crate::services::engine_process::EngineProcess;
use crate::services::events::TauriSink;
use crate::services::images::{ImageService, OsTrash};
use crate::services::inputs::InputService;
use crate::services::jobs::JobService;
use crate::services::models::ModelService;
use crate::services::models::catalog::Catalog;
use crate::services::models::download::DownloadPolicy;
use crate::services::presets::PresetService;
use crate::services::settings::Settings;
use crate::services::storage::StorageService;

/// アプリが使うディレクトリ(docs/06-cross-platform.md §1)。
///
/// `user_dir` はログイン中のユーザーの作業スペース(`{data_dir}/users/<id>`。docs/36 §2.1)。
/// `Clone` で共有され、ログイン・ログアウトは全サービスに反映される。
#[derive(Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    user_dir: Arc<std::sync::RwLock<Option<PathBuf>>>,
}

impl AppPaths {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            user_dir: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// ユーザーの作業スペースの置き場所(`{data_dir}/users`)。
    pub fn users_root(&self) -> PathBuf {
        self.data_dir.join("users")
    }

    pub fn user_workspace(&self, user_id: &str) -> PathBuf {
        self.users_root().join(user_id)
    }

    pub fn set_user_dir(&self, dir: Option<PathBuf>) {
        if let Ok(mut g) = self.user_dir.write() {
            *g = dir;
        }
    }

    /// ログイン中のユーザーの作業スペース。未ログインは `None`。
    pub fn current_user_dir(&self) -> Option<PathBuf> {
        self.user_dir.read().ok().and_then(|g| g.clone())
    }

    /// 既定の画像保存先。ログイン中は `users/<id>/images`、テストなど未ログインは `{data_dir}/images`。
    pub fn default_images_dir(&self) -> PathBuf {
        match self.current_user_dir() {
            Some(d) => d.join("images"),
            None => self.data_dir.join("images"),
        }
    }

    pub fn resolve_images_dir(&self, settings: &Settings) -> PathBuf {
        settings
            .images_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.default_images_dir())
    }
}

/// 非同期ログ書き込みのフラッシュを保証するため、アプリ終了まで保持する。
struct LogGuard(#[allow(dead_code)] tracing_appender::non_blocking::WorkerGuard);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let log_dir = app.path().app_log_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            std::fs::create_dir_all(&log_dir)?;

            // ログ(docs/31)。起動直後は INFO のみ。デバッグの設定は DB を開いた後に反映する
            let (debug_log, guard) = services::debug_log::DebugLog::init(&log_dir)?;
            app.manage(LogGuard(guard));

            // 端末共通の DB(ユーザー一覧・端末の設定)。ユーザーの DB はログイン時に開く(docs/36)
            let app_db = data_dir.join("app.db");
            let db = Db::new(&app_db)?;
            let paths = AppPaths::new(data_dir);
            let users = services::users::UserService::new(&app_db, db.clone(), paths.clone())?;
            app.manage(Arc::clone(&users));

            let comfyui_url = tauri::async_runtime::block_on(
                db.call_device(|c| services::settings::get_comfyui_url(c)),
            )?;

            // デバッグログの ON/OFF を復元し、ON の間は合計サイズを定期的に整理する(docs/31)
            if tauri::async_runtime::block_on(
                db.call_device(|c| services::settings::get_debug_enabled(c)),
            )? {
                debug_log.set_enabled(true);
                let managed = tauri::async_runtime::block_on(
                    db.call_device(|c| services::settings::get_managed_engine(c)),
                )?
                .enabled;
                debug_log.log_environment(&services::debug_log::describe_environment(
                    &app.package_info().version.to_string(),
                    &comfyui_url,
                    managed,
                    true,
                ));
            }
            tauri::async_runtime::spawn(Arc::clone(&debug_log).run_pruner());
            app.manage(debug_log);

            let engine = Arc::new(EngineService::new(&comfyui_url)?);
            let jobs = JobService::new(
                db.clone(),
                Arc::clone(&engine),
                Arc::new(TauriSink(app.handle().clone())),
                paths.clone(),
            );
            let images = ImageService::new(
                db.clone(),
                Arc::new(TauriSink(app.handle().clone())),
                paths.clone(),
                Arc::new(OsTrash),
            );
            // モデル取得(docs/12)。ダウンロードはユーザーの操作で開始されたものだけをキューで実行する
            let model_policy = DownloadPolicy::production();
            let models = ModelService::new(
                db.clone(),
                Arc::clone(&engine),
                Arc::new(Catalog::embedded(&model_policy)?),
                model_policy,
                Arc::new(TauriSink(app.handle().clone())),
            )?;
            tauri::async_runtime::spawn(Arc::clone(&models).run_worker());
            app.manage(models);
            let storage = StorageService::new(db.clone(), paths.clone(), Arc::new(OsTrash));
            app.manage(Arc::new(PresetService::new(db.clone())));
            let assistant = AssistantService::new(
                db.clone(),
                AnthropicClient::new()?,
                Arc::new(TauriSink(app.handle().clone())),
            );
            // ローカル LLM(docs/30)。導入・起動・メモリ解放をアプリが管理する
            let llm = services::llm::LlmService::new(
                db.clone(),
                paths.data_dir.join("llm"),
                services::llm::manifest::Manifest::embedded()?,
                Arc::new(TauriSink(app.handle().clone())),
                Some(log_dir.join("llm.log")),
            );
            assistant.set_runtime(llm.clone());
            // 翻訳専用モデル(docs/34 §9.13)。本体とは別のディレクトリ・マニフェスト。同時には起動しない
            let mt = services::llm::LlmService::new_mt(
                db.clone(),
                paths.data_dir.join("llm-mt"),
                services::llm::manifest::Manifest::embedded_mt()?,
                Arc::new(services::events::TauriMtSink(app.handle().clone())),
                Some(log_dir.join("llm-mt.log")),
            );
            services::llm::LlmService::link_peers(&llm, &mt);
            assistant.set_mt_runtime(mt.clone());
            jobs.set_generation_hook(llm.clone());
            tauri::async_runtime::spawn(Arc::clone(&llm).run_idle_monitor());
            tauri::async_runtime::spawn(Arc::clone(&mt).run_idle_monitor());
            app.manage(assistant);
            app.manage(llm);
            app.manage(services::llm::MtLlm(mt));
            app.manage(storage);
            // バックアップ(docs/35)。設定値・ギャラリー・プリセットの取得と復元
            app.manage(services::backup::BackupService::new(
                db.clone(),
                paths.clone(),
                Arc::new(OsTrash),
                Arc::new(commands::backup::TauriBackupSink(app.handle().clone())),
                app.package_info().version.to_string(),
            ));
            app.manage(engine);
            app.manage(Arc::clone(&jobs));
            app.manage(InputService::new(
                db.clone(),
                paths.clone(),
                Arc::clone(&images),
            ));
            app.manage(Arc::clone(&images));
            // ComfyUI の起動管理(docs/15)。アプリが起動した子プロセスだけを管理し、終了時に停止する
            let engine_proc = EngineProcess::new(
                Arc::new(TauriSink(app.handle().clone())),
                Some(log_dir.join("comfyui.log")),
            );
            app.manage(Arc::clone(&engine_proc));
            // ComfyUI のアプリ内インストール(docs/23)。利用者の操作で開始されたときだけ動く
            app.manage(services::engine_install::EngineInstaller::new(
                paths.data_dir.join("comfyui"),
                services::engine_install::manifest::Manifest::embedded()?,
                Arc::new(TauriSink(app.handle().clone())),
            ));
            {
                let db = db.clone();
                tauri::async_runtime::spawn(async move {
                    match db
                        .call_device(|c| services::settings::get_managed_engine(c))
                        .await
                    {
                        Ok(cfg) if cfg.enabled && cfg.auto_start => {
                            if let Err(e) = engine_proc.start(&cfg).await {
                                tracing::warn!(error = %e, "comfyui auto start failed");
                            }
                        }
                        Ok(_) => {}
                        Err(e) => tracing::warn!(error = %e, "managed engine config unreadable"),
                    }
                });
            }
            tauri::async_runtime::spawn(services::engine::monitor(app.handle().clone()));
            // ジョブのワーカー。ログインするまでは待機する。前回の実行中ジョブの復旧は、ログイン時に行う(docs/36)
            tauri::async_runtime::spawn(async move {
                jobs.run_worker().await;
            });

            tracing::info!("gazooo started");
            app.manage(db);
            app.manage(paths);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::user::user_list,
            commands::user::user_current,
            commands::user::user_create,
            commands::user::user_login,
            commands::user::user_logout,
            commands::user::user_rename,
            commands::user::user_delete,
            commands::user::user_lock_set,
            commands::user::user_lock_clear,
            commands::settings::settings_get,
            commands::settings::settings_update,
            commands::debug::debug_status,
            commands::debug::debug_set_enabled,
            commands::debug::debug_export_logs,
            commands::debug::debug_open_log_dir,
            commands::debug::debug_clear_logs,
            commands::backup::backup_estimate,
            commands::backup::backup_export,
            commands::backup::backup_inspect,
            commands::backup::backup_orphans,
            commands::backup::backup_restore,
            commands::backup::backup_cancel,
            commands::backup::backup_restart,
            commands::engine::engine_status,
            commands::engine::engine_list_models,
            commands::engine::engine_reconnect,
            commands::engine_process::engine_managed_get,
            commands::engine_process::engine_managed_set,
            commands::engine_process::engine_managed_suggest,
            commands::engine_process::engine_process_start,
            commands::engine_process::engine_process_stop,
            commands::engine_process::engine_process_restart,
            commands::engine_process::engine_process_logs,
            commands::engine_install::engine_install_status,
            commands::engine_install::engine_install_preflight,
            commands::engine_install::engine_install_start,
            commands::engine_install::engine_install_cancel,
            commands::engine_install::engine_install_remove,
            commands::engine_install::engine_install_logs,
            commands::jobs::job_enqueue,
            commands::jobs::job_list,
            commands::jobs::job_cancel,
            commands::jobs::job_reorder,
            commands::jobs::job_retry,
            commands::jobs::job_clear_finished,
            commands::jobs::job_remove,
            commands::jobs::queue_pause,
            commands::jobs::queue_resume,
            commands::jobs::queue_state,
            commands::images::image_list,
            commands::images::image_get,
            commands::images::image_set_favorite,
            commands::images::image_delete,
            commands::images::image_export,
            commands::images::image_reveal,
            commands::inputs::input_image_pick,
            commands::inputs::input_image_from_gallery,
            commands::inputs::input_image_get,
            commands::inputs::mask_save,
            commands::presets::preset_list,
            commands::presets::preset_save,
            commands::presets::preset_delete,
            commands::storage::storage_change_images_dir,
            commands::assistant::assistant_status,
            commands::assistant::assistant_set_key,
            commands::assistant::assistant_clear_key,
            commands::assistant::assistant_update,
            commands::assistant::assistant_local_check,
            commands::llm::llm_status,
            commands::llm::llm_install,
            commands::llm::llm_install_cancel,
            commands::llm::llm_remove,
            commands::llm::llm_start,
            commands::llm::llm_stop,
            commands::llm::llm_logs,
            commands::llm::llm_update_settings,
            commands::translate_mt::translate_mt_status,
            commands::translate_mt::translate_mt_install,
            commands::translate_mt::translate_mt_install_cancel,
            commands::translate_mt::translate_mt_remove,
            commands::translate_mt::translate_mt_license,
            commands::assistant::assistant_prompt_get,
            commands::assistant::assistant_prompt_set,
            commands::assistant::assistant_prompt_reset,
            commands::assistant::assistant_ask,
            commands::assistant::assistant_cancel,
            commands::assistant::assistant_conversation_list,
            commands::assistant::assistant_conversation_get,
            commands::assistant::assistant_conversation_delete,
            commands::assistant::assistant_suggestion_applied,
            commands::assistant::translate_status,
            commands::assistant::translate_update,
            commands::assistant::translate_run,
            commands::assistant::translate_cancel,
            commands::models::models_catalog,
            commands::models::models_download_start,
            commands::models::models_download_cancel,
            commands::models::models_download_list,
            commands::models::models_verify,
            commands::models::models_set_dir,
            commands::models::models_engine_dir_status,
            commands::models::models_apply_dir_to_engine,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // アプリが起動した ComfyUI は、終了時に確実に止める(異常終了時は Job Object / プロセスグループが止める)
            if let tauri::RunEvent::Exit = event
                && let Some(llm) = app.try_state::<Arc<services::llm::LlmService>>()
            {
                let llm = Arc::clone(&llm);
                tauri::async_runtime::block_on(async move {
                    let _ =
                        tokio::time::timeout(std::time::Duration::from_secs(15), llm.shutdown())
                            .await;
                });
            }
            if let tauri::RunEvent::Exit = event
                && let Some(mt) = app.try_state::<services::llm::MtLlm>()
            {
                let mt = Arc::clone(&mt.0);
                tauri::async_runtime::block_on(async move {
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(15), mt.shutdown())
                        .await;
                });
            }
            if let tauri::RunEvent::Exit = event
                && let Some(proc) = app.try_state::<Arc<EngineProcess>>()
            {
                let proc = Arc::clone(&proc);
                tauri::async_runtime::block_on(async move {
                    let _ =
                        tokio::time::timeout(std::time::Duration::from_secs(15), proc.stop()).await;
                });
            }
        });
}
