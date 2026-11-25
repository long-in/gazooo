//! ComfyUI のアプリ内インストールのコマンド(docs/23、docs/04-ipc-api.md「ComfyUI のインストール」)。

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::services::engine::EngineService;
use crate::services::engine_install::manifest::{Component, Platform};
use crate::services::engine_install::{
    EngineInstaller, Finalize, InstallRecord, InstallState, Preflight, StartRequest,
};
use crate::services::engine_process::EngineProcess;
use crate::services::engine_process::config::ManagedEngineConfig;
use crate::services::jobs::JobService;
use crate::services::settings;

type Installer<'a> = State<'a, Arc<EngineInstaller>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentView {
    pub id: String,
    pub name: String,
    pub license: String,
    pub download_mb: u64,
    pub disk_mb: u64,
}

impl From<&Component> for ComponentView {
    fn from(c: &Component) -> Self {
        Self {
            id: c.id.clone(),
            name: c.name.clone(),
            license: c.license.clone(),
            download_mb: c.download_mb,
            disk_mb: c.disk_mb,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseView {
    pub python_version: String,
    pub comfyui_tag: String,
    pub comfyui_license: String,
    pub torch_version: String,
    pub torch_license: String,
    pub download_mb: u64,
    pub disk_mb: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallView {
    pub state: InstallState,
    /// 検証まで完了した導入の記録。
    pub record: Option<InstallRecord>,
    /// アプリに内蔵のマニフェストの版が、導入済みの版と異なる。
    pub update_available: bool,
    pub manifest_version: String,
    pub install_dir: String,
    pub platform: Option<Platform>,
    pub base: BaseView,
    pub components: Vec<ComponentView>,
    /// 接続する取得元のホスト(同意ダイアログに表示する)。
    pub hosts: Vec<String>,
}

fn view(installer: &EngineInstaller) -> InstallView {
    let m = installer.manifest();
    let record = installer.layout().load_record().filter(|r| r.complete);
    InstallView {
        state: installer.state(),
        update_available: record
            .as_ref()
            .is_some_and(|r| r.manifest_version != m.manifest_version),
        record,
        manifest_version: m.manifest_version.clone(),
        install_dir: installer.layout().root.to_string_lossy().into_owned(),
        platform: Platform::current(),
        base: BaseView {
            python_version: m.python.version.clone(),
            comfyui_tag: m.comfyui.tag.clone(),
            comfyui_license: m.comfyui.license.clone(),
            torch_version: m.torch.version.clone(),
            torch_license: m.torch.license.clone(),
            download_mb: m.torch.download_mb + 100,
            disk_mb: m.torch.disk_mb + 1000,
        },
        components: m.components.iter().map(ComponentView::from).collect(),
        hosts: m.allowed_hosts.clone(),
    }
}

/// 実行中のジョブがある・ComfyUI をアプリが起動中のときは、導入・削除しない。
async fn ensure_idle(jobs: &JobService, proc: &EngineProcess) -> AppResult<()> {
    if proc.has_child() {
        return Err(AppError::engine_proc(
            "ENGINE_BUSY",
            "ComfyUI が起動中です。先に停止してください",
        ));
    }
    let (running, queued) = jobs.active_counts().await?;
    if running + queued > 0 {
        return Err(AppError::EngineProc {
            code: "ENGINE_BUSY",
            message: "実行中または待機中の生成があります".into(),
            details: Some(serde_json::json!({ "running": running, "queued": queued })),
        });
    }
    Ok(())
}

#[tauri::command]
pub async fn engine_install_status(installer: Installer<'_>) -> AppResult<InstallView> {
    Ok(view(&installer))
}

#[tauri::command]
pub async fn engine_install_preflight(
    installer: Installer<'_>,
    components: Vec<String>,
) -> AppResult<Preflight> {
    installer.preflight(&components).await
}

#[tauri::command]
pub async fn engine_install_start(
    app: AppHandle,
    installer: Installer<'_>,
    jobs: State<'_, Arc<JobService>>,
    proc: State<'_, Arc<EngineProcess>>,
    request: StartRequest,
) -> AppResult<InstallState> {
    ensure_idle(&jobs, &proc).await?;
    // 導入が成功したら、起動管理を有効にしてパスを設定する(docs/23 §4.2 の 9)
    let finalize: Finalize = {
        let app = app.clone();
        Arc::new(move |cfg: ManagedEngineConfig| {
            let app = app.clone();
            Box::pin(async move {
                let db = app.state::<Db>();
                let url = cfg.url();
                let saved = cfg.clone();
                db.call(move |c| {
                    settings::set_managed_engine(c, &saved)?;
                    let patch = serde_json::from_value(serde_json::json!({ "comfyuiUrl": url }))?;
                    settings::update(c, patch)?;
                    Ok(())
                })
                .await?;
                if let Some(engine) = app.try_state::<Arc<EngineService>>()
                    && engine.set_url(&cfg.url())?
                {
                    let engine = Arc::clone(&engine);
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = engine.refresh(&app, true).await;
                    });
                }
                Ok(())
            })
        })
    };
    installer.start(request, finalize).await
}

#[tauri::command]
pub async fn engine_install_cancel(installer: Installer<'_>) -> AppResult<()> {
    installer.cancel();
    Ok(())
}

#[tauri::command]
pub async fn engine_install_remove(
    db: State<'_, Db>,
    installer: Installer<'_>,
    jobs: State<'_, Arc<JobService>>,
    proc: State<'_, Arc<EngineProcess>>,
    keep_data: bool,
) -> AppResult<InstallView> {
    ensure_idle(&jobs, &proc).await?;
    installer.remove(keep_data).await?;
    // 削除した導入を指す起動管理の設定は、無効に戻す(別の場所の設定はそのまま)
    let root = installer.layout().root.to_string_lossy().into_owned();
    db.call(move |c| {
        let cfg = settings::get_managed_engine(c)?;
        if cfg.main_py.starts_with(&root) {
            settings::set_managed_engine(c, &ManagedEngineConfig::default())?;
        }
        Ok(())
    })
    .await?;
    Ok(view(&installer))
}

#[derive(Serialize)]
pub struct Logs {
    pub lines: Vec<String>,
}

#[tauri::command]
pub async fn engine_install_logs(
    installer: Installer<'_>,
    lines: Option<usize>,
) -> AppResult<Logs> {
    Ok(Logs {
        lines: installer.logs(lines.unwrap_or(500).min(500)),
    })
}
