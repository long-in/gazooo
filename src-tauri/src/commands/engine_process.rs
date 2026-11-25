//! ComfyUI の起動管理のコマンド(docs/15 §7、docs/04-ipc-api.md「ComfyUI の起動管理」)。

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::services::engine::EngineService;
use crate::services::engine_install::EngineInstaller;
use crate::services::engine_process::config::{ManagedEngineConfig, Suggestions, detect, suggest};
use crate::services::engine_process::known::{KnownDirs, detect_known};
use crate::services::engine_process::{EngineProcState, EngineProcess};
use crate::services::jobs::JobService;
use crate::services::models::is_local_engine;
use crate::services::settings;

type Proc<'a> = State<'a, Arc<EngineProcess>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedView {
    pub config: ManagedEngineConfig,
    pub state: EngineProcState,
}

/// 起動中の ComfyUI への問い合わせの上限(docs/32 Q-183)。
const LAUNCH_ARGS_TIMEOUT: Duration = Duration::from_millis(1500);

async fn load_config(db: &Db) -> AppResult<ManagedEngineConfig> {
    db.call(|c| settings::get_managed_engine(c)).await
}

/// OS の標準フォルダと、アプリが導入した ComfyUI の場所(docs/22 §3)。
fn known_dirs(app: &AppHandle, installer: &EngineInstaller) -> KnownDirs {
    let p = app.path();
    KnownDirs {
        app_comfyui: Some(installer.layout().root.clone()),
        home: p.home_dir().ok(),
        documents: p.document_dir().ok(),
        desktop: p.desktop_dir().ok(),
        downloads: p.download_dir().ok(),
        local_data: p.local_data_dir().ok(),
    }
}

/// 設定と状態だけを返す(DB のみ。探索はしない。docs/32)。
async fn view(db: &Db, proc: &EngineProcess) -> AppResult<ManagedView> {
    Ok(ManagedView {
        config: load_config(db).await?,
        state: proc.state(),
    })
}

/// 入力候補を求める。優先順位(docs/22 §2): 起動中の ComfyUI → 既知の場所の一覧(アプリが導入したものが最優先)→ 実行ファイル周辺(開発用)。
async fn suggestions(
    app: &AppHandle,
    installer: &EngineInstaller,
    engine: &EngineService,
) -> AppResult<Suggestions> {
    let engine = engine.engine()?;
    let from_running = if is_local_engine(&engine.url()) {
        // 応答しない ComfyUI で待たされないよう、短く打ち切る(失敗・超過は「候補なし」)
        match tokio::time::timeout(LAUNCH_ARGS_TIMEOUT, engine.launch_args()).await {
            Ok(Ok(argv)) => suggest(&argv),
            _ => Suggestions::default(),
        }
    } else {
        Suggestions::default()
    };
    if from_running.main_py.is_some() {
        return Ok(from_running);
    }
    let dirs = known_dirs(app, installer);
    // ファイルの存在確認は同期 I/O なので、非同期ランタイムを塞がないようにする
    tokio::task::spawn_blocking(move || {
        let known = detect_known(&dirs);
        if known.main_py.is_some() {
            known
        } else {
            detect(&search_starts())
        }
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))
}

/// 既定の探索の起点(実行ファイルの場所と、カレントディレクトリ)。
fn search_starts() -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        v.push(exe);
    }
    if let Ok(cwd) = std::env::current_dir() {
        v.push(cwd);
    }
    v
}

#[tauri::command]
pub async fn engine_managed_get(db: State<'_, Db>, proc: Proc<'_>) -> AppResult<ManagedView> {
    view(&db, &proc).await
}

/// 入力候補の取得(docs/32)。`engine_managed_get` とは別に呼ぶ。
#[tauri::command]
pub async fn engine_managed_suggest(
    app: AppHandle,
    installer: State<'_, Arc<EngineInstaller>>,
    engine: State<'_, Arc<EngineService>>,
) -> AppResult<Suggestions> {
    suggestions(&app, &installer, &engine).await
}

#[tauri::command]
pub async fn engine_managed_set(
    app: AppHandle,
    db: State<'_, Db>,
    engine: State<'_, Arc<EngineService>>,
    proc: Proc<'_>,
    config: ManagedEngineConfig,
) -> AppResult<ManagedView> {
    config.validate()?;
    if !config.enabled && proc.has_child() {
        return Err(AppError::engine_proc(
            "ENGINE_BUSY",
            "ComfyUI が起動中です。先に停止してから、アプリでの管理を無効にしてください",
        ));
    }
    let url = config.enabled.then(|| config.url());
    let saved = config.clone();
    db.call(move |c| {
        settings::set_managed_engine(c, &saved)?;
        // 有効にしたら、接続先をローカルの管理ポートに合わせる
        if let Some(url) = url {
            let patch = serde_json::from_value(serde_json::json!({ "comfyuiUrl": url }))?;
            settings::update(c, patch)?;
        }
        Ok(())
    })
    .await?;
    if config.enabled && engine.set_url(&config.url())? {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(engine) = app.try_state::<Arc<EngineService>>() {
                let _ = engine.refresh(&app, true).await;
            }
        });
    }
    view(&db, &proc).await
}

#[tauri::command]
pub async fn engine_process_start(db: State<'_, Db>, proc: Proc<'_>) -> AppResult<EngineProcState> {
    let cfg = load_config(&db).await?;
    proc.start(&cfg).await
}

#[tauri::command]
pub async fn engine_process_stop(proc: Proc<'_>) -> AppResult<EngineProcState> {
    proc.stop().await
}

#[tauri::command]
pub async fn engine_process_restart(
    db: State<'_, Db>,
    jobs: State<'_, Arc<JobService>>,
    proc: Proc<'_>,
    force: Option<bool>,
) -> AppResult<EngineProcState> {
    let cfg = load_config(&db).await?;
    if !cfg.enabled {
        return Err(AppError::engine_proc(
            "ENGINE_NOT_MANAGED",
            "ComfyUI の起動管理が有効ではありません",
        ));
    }
    if !force.unwrap_or(false) {
        let (running, queued) = jobs.active_counts().await?;
        if running + queued > 0 {
            return Err(AppError::EngineProc {
                code: "ENGINE_BUSY",
                message: "実行中または待機中の生成があります".into(),
                details: Some(serde_json::json!({ "running": running, "queued": queued })),
            });
        }
    }
    proc.restart(&cfg).await
}

#[derive(Serialize)]
pub struct Logs {
    pub lines: Vec<String>,
}

#[tauri::command]
pub async fn engine_process_logs(proc: Proc<'_>, lines: Option<usize>) -> AppResult<Logs> {
    Ok(Logs {
        lines: proc.logs(lines.unwrap_or(500).min(500)),
    })
}
