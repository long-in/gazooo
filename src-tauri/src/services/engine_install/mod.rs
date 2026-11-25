//! ComfyUI のアプリ内インストール(docs/23)。
//!
//! `<app_data>/comfyui/` に、単体配布の Python・venv・ComfyUI 本体・PyTorch・カスタムノードを導入する。
//! 各手順は冪等で、完了した手順は印(`.gazooo-install.json`)に記録して、再実行時に飛ばす。
//! 通信は利用者が同意して開始したときだけ。取得は許可ホストの https のみ、SHA-256 を照合する。

pub mod archive;
pub mod manifest;

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use command_group::{AsyncCommandGroup, AsyncGroupChild};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;

use self::manifest::{Component, Manifest, Platform, Source};
use crate::error::{AppError, AppResult};
use crate::services::engine_process::config::{DEFAULT_PORT, ManagedEngineConfig};
use crate::services::models::download::{
    self, CancelMode, DownloadPolicy, Outcome, Phase, Progress, Request,
};

pub const MARKER: &str = ".gazooo-install.json";
/// 基本(Python・ComfyUI・PyTorch)に必要な空き容量の下限(MB)。
const BASE_REQUIRED_MB: u64 = 12_000;
const TOTAL_STEPS: u32 = 8;
const LOG_LINES: usize = 500;
const LOG_LINE_MAX: usize = 2000;
const VERIFY_TIMEOUT: Duration = Duration::from_secs(300);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn fail(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::engine_proc(code, message)
}

fn cancelled_error() -> AppError {
    fail("ENGINE_INSTALL_CANCELLED", "導入を中止しました")
}

fn is_cancelled(e: &AppError) -> bool {
    e.code() == "ENGINE_INSTALL_CANCELLED"
}

// ---------------------------------------------------------------- 状態・記録

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallStatus {
    Idle,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallErrorView {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallState {
    pub status: InstallStatus,
    /// 現在の手順(1 始まり)。実行中以外は 0。
    pub step: u32,
    pub total_steps: u32,
    pub step_id: Option<String>,
    pub received: Option<u64>,
    pub total: Option<u64>,
    pub error: Option<InstallErrorView>,
}

impl InstallState {
    fn idle() -> Self {
        Self {
            status: InstallStatus::Idle,
            step: 0,
            total_steps: TOTAL_STEPS,
            step_id: None,
            received: None,
            total: None,
            error: None,
        }
    }
}

pub trait InstallSink: Send + Sync {
    fn state(&self, state: &InstallState);
    fn log(&self, line: &str);
}

/// 導入の記録(`<root>/.gazooo-install.json`)。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InstallRecord {
    pub manifest_version: String,
    pub comfyui_tag: String,
    pub components: Vec<String>,
    /// 完了した手順 → その時の入力の識別子(ハッシュやバージョン)。
    pub steps: BTreeMap<String, String>,
    /// 同意した時刻(UNIX 秒)。
    pub accepted_at: i64,
    /// 検証まで完了したか。
    pub complete: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    pub components: Vec<String>,
    /// ライセンスと取得元の一覧を見て同意した。
    pub accepted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GpuStatus {
    Found,
    Missing,
    NotApplicable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    pub platform: Option<Platform>,
    pub supported: bool,
    pub unsupported_reason: Option<String>,
    pub free_bytes: Option<u64>,
    pub required_bytes: u64,
    pub enough_space: bool,
    pub gpu: GpuStatus,
    pub gpu_name: Option<String>,
    pub install_dir: String,
}

/// 導入先のフォルダ構成(08 のレイアウトと同じ並び)。
#[derive(Debug, Clone)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn python_dir(&self) -> PathBuf {
        self.root.join("python")
    }
    pub fn python_exe(&self, platform: Platform) -> PathBuf {
        match platform {
            Platform::WindowsX64 => self.python_dir().join("python.exe"),
            Platform::MacosArm64 => self.python_dir().join("bin").join("python3"),
        }
    }
    pub fn venv_dir(&self) -> PathBuf {
        self.root.join("venv")
    }
    pub fn venv_python(&self, platform: Platform) -> PathBuf {
        match platform {
            Platform::WindowsX64 => self.venv_dir().join("Scripts").join("python.exe"),
            Platform::MacosArm64 => self.venv_dir().join("bin").join("python"),
        }
    }
    pub fn comfyui_dir(&self) -> PathBuf {
        self.root.join("ComfyUI")
    }
    pub fn main_py(&self) -> PathBuf {
        self.comfyui_dir().join("main.py")
    }
    pub fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }
    pub fn nodes_dir(&self) -> PathBuf {
        self.data_dir().join("custom_nodes")
    }
    pub fn downloads_dir(&self) -> PathBuf {
        self.root.join("downloads")
    }
    pub fn marker(&self) -> PathBuf {
        self.root.join(MARKER)
    }

    pub fn load_record(&self) -> Option<InstallRecord> {
        let text = std::fs::read_to_string(self.marker()).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn save_record(&self, record: &InstallRecord) -> AppResult<()> {
        std::fs::create_dir_all(&self.root)?;
        let tmp = self.root.join(format!("{MARKER}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(record)?)?;
        std::fs::rename(&tmp, self.marker())?;
        Ok(())
    }

    /// 導入後の起動管理の設定(docs/23 §4.2 の 9)。
    pub fn managed_config(&self, platform: Platform) -> ManagedEngineConfig {
        ManagedEngineConfig {
            enabled: true,
            main_py: self.main_py().to_string_lossy().into_owned(),
            python: self.venv_python(platform).to_string_lossy().into_owned(),
            base_directory: Some(self.data_dir().to_string_lossy().into_owned()),
            port: DEFAULT_PORT,
            auto_start: false,
            auto_recover: false,
        }
    }
}

/// 導入が成功したときに呼ばれる(設定の保存など。呼び出し側が与える)。
pub type Finalize =
    Arc<dyn Fn(ManagedEngineConfig) -> BoxFuture<'static, AppResult<()>> + Send + Sync>;

struct Inner {
    state: InstallState,
    cancel_tx: Option<watch::Sender<CancelMode>>,
}

/// ログの保持(最新 500 行)と通知。子プロセスの出力を読むタスクとも共有する。
struct LogBuf {
    lines: Mutex<VecDeque<String>>,
    sink: Arc<dyn InstallSink>,
}

impl LogBuf {
    fn push(&self, line: &str) {
        let mut line = strip_ansi(line);
        if line.chars().count() > LOG_LINE_MAX {
            line = line.chars().take(LOG_LINE_MAX).collect();
        }
        if let Ok(mut g) = self.lines.lock() {
            if g.len() >= LOG_LINES {
                g.pop_front();
            }
            g.push_back(line.clone());
        }
        self.sink.log(&line);
    }

    fn clear(&self) {
        if let Ok(mut g) = self.lines.lock() {
            g.clear();
        }
    }

    fn tail(&self, n: usize) -> Vec<String> {
        self.lines
            .lock()
            .map(|g| {
                let skip = g.len().saturating_sub(n);
                g.iter().skip(skip).cloned().collect()
            })
            .unwrap_or_default()
    }
}

pub struct EngineInstaller {
    layout: Layout,
    manifest: Manifest,
    platform: Option<Platform>,
    policy: DownloadPolicy,
    sink: Arc<dyn InstallSink>,
    inner: Mutex<Inner>,
    log: Arc<LogBuf>,
    cancel_flag: Arc<AtomicBool>,
}

impl EngineInstaller {
    pub fn new(root: PathBuf, manifest: Manifest, sink: Arc<dyn InstallSink>) -> Arc<Self> {
        let policy = manifest.policy();
        Self::with_options(root, manifest, Platform::current(), policy, sink)
    }

    pub fn with_options(
        root: PathBuf,
        manifest: Manifest,
        platform: Option<Platform>,
        policy: DownloadPolicy,
        sink: Arc<dyn InstallSink>,
    ) -> Arc<Self> {
        Arc::new(Self {
            layout: Layout { root },
            manifest,
            platform,
            policy,
            log: Arc::new(LogBuf {
                lines: Mutex::new(VecDeque::new()),
                sink: Arc::clone(&sink),
            }),
            sink,
            inner: Mutex::new(Inner {
                state: InstallState::idle(),
                cancel_tx: None,
            }),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn state(&self) -> InstallState {
        self.inner
            .lock()
            .map(|g| g.state.clone())
            .unwrap_or_else(|_| InstallState::idle())
    }

    pub fn is_running(&self) -> bool {
        self.state().status == InstallStatus::Running
    }

    pub fn logs(&self, lines: usize) -> Vec<String> {
        self.log.tail(lines)
    }

    fn push_log(&self, line: &str) {
        self.log.push(line);
    }

    fn update(&self, f: impl FnOnce(&mut InstallState)) {
        let snapshot = {
            let Ok(mut g) = self.inner.lock() else { return };
            f(&mut g.state);
            g.state.clone()
        };
        self.sink.state(&snapshot);
    }

    fn begin_step(&self, step: u32, id: &str, label: &str) {
        self.push_log(&format!("==> [{step}/{TOTAL_STEPS}] {label}"));
        self.update(|s| {
            s.step = step;
            s.step_id = Some(id.to_string());
            s.received = None;
            s.total = None;
        });
    }

    // ------------------------------------------------------------ 事前チェック

    /// 何もダウンロードせずに、導入できる環境かを調べる(docs/23 §4.1)。
    pub async fn preflight(&self, components: &[String]) -> AppResult<Preflight> {
        let extra: u64 = components
            .iter()
            .filter_map(|id| self.manifest.component(id))
            .map(|c| c.disk_mb)
            .sum();
        let required_bytes = (BASE_REQUIRED_MB + extra) * 1024 * 1024;
        let platform = self.platform;
        let (supported, reason) = match platform {
            Some(_) => (true, None),
            None => (
                false,
                Some(
                    "この OS・CPU は対象外です(Windows 10/11 x64 と macOS Apple Silicon に対応)"
                        .to_string(),
                ),
            ),
        };
        let root = self.layout.root.clone();
        let free_bytes = tokio::task::spawn_blocking(move || free_space_near(&root))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
        let (gpu, gpu_name) = match platform {
            Some(Platform::WindowsX64) => {
                match tokio::task::spawn_blocking(detect_nvidia)
                    .await
                    .map_err(|e| AppError::internal(e.to_string()))?
                {
                    Some(name) => (GpuStatus::Found, Some(name)),
                    None => (GpuStatus::Missing, None),
                }
            }
            _ => (GpuStatus::NotApplicable, None),
        };
        Ok(Preflight {
            platform,
            supported,
            unsupported_reason: reason,
            enough_space: free_bytes.is_none_or(|f| f >= required_bytes),
            free_bytes,
            required_bytes,
            gpu,
            gpu_name,
            install_dir: self.layout.root.to_string_lossy().into_owned(),
        })
    }

    // ------------------------------------------------------------ 開始・中止

    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
        if let Ok(g) = self.inner.lock()
            && let Some(tx) = &g.cancel_tx
        {
            let _ = tx.send(CancelMode::Cancel);
        }
    }

    /// 導入を開始する。手順はバックグラウンドで進み、状態は `InstallSink` で通知する。
    pub async fn start(
        self: &Arc<Self>,
        req: StartRequest,
        finalize: Finalize,
    ) -> AppResult<InstallState> {
        if !req.accepted {
            return Err(AppError::validation(
                "accepted",
                "ライセンスと取得元への同意が必要です",
            ));
        }
        let platform = self.platform.ok_or_else(|| {
            fail(
                "ENGINE_INSTALL_UNSUPPORTED",
                "この OS・CPU には対応していません",
            )
        })?;
        let mut selected = Vec::new();
        for id in &req.components {
            let c = self.manifest.component(id).ok_or_else(|| {
                AppError::validation("components", format!("不明なコンポーネント: {id}"))
            })?;
            if !selected.iter().any(|s: &Component| s.id == c.id) {
                selected.push(c.clone());
            }
        }
        let pre = self.preflight(&req.components).await?;
        if !pre.enough_space {
            return Err(AppError::EngineProc {
                code: "ENGINE_INSTALL_DISK_FULL",
                message: "ディスクの空き容量が足りません".into(),
                details: Some(serde_json::json!({
                    "freeBytes": pre.free_bytes,
                    "requiredBytes": pre.required_bytes,
                })),
            });
        }

        let (tx, rx) = watch::channel(CancelMode::Run);
        {
            let mut g = self
                .inner
                .lock()
                .map_err(|_| AppError::internal("installer lock poisoned"))?;
            if g.state.status == InstallStatus::Running {
                return Err(fail("ENGINE_BUSY", "すでに導入を実行中です"));
            }
            self.cancel_flag.store(false, Ordering::Relaxed);
            g.cancel_tx = Some(tx);
            self.log.clear();
            g.state = InstallState {
                status: InstallStatus::Running,
                step: 0,
                ..InstallState::idle()
            };
        }
        self.sink.state(&self.state());

        let me = Arc::clone(self);
        tokio::spawn(async move {
            let result = me.run(platform, selected, rx).await;
            let result = match result {
                Ok(cfg) => finalize(cfg).await,
                Err(e) => Err(e),
            };
            match result {
                Ok(()) => me.update(|s| {
                    s.status = InstallStatus::Done;
                    s.error = None;
                }),
                Err(e) if is_cancelled(&e) => {
                    me.push_log("導入を中止しました(再実行すると続きから再開します)");
                    me.update(|s| {
                        s.status = InstallStatus::Cancelled;
                        s.error = None;
                    });
                }
                Err(e) => {
                    me.push_log(&format!("エラー: {e}"));
                    me.update(|s| {
                        s.status = InstallStatus::Failed;
                        s.error = Some(InstallErrorView {
                            code: e.code().to_string(),
                            message: e.to_string(),
                        });
                    });
                }
            }
            if let Ok(mut g) = me.inner.lock() {
                g.cancel_tx = None;
            }
        });
        Ok(self.state())
    }

    // ------------------------------------------------------------ 手順

    fn check_cancel(&self) -> AppResult<()> {
        if self.cancel_flag.load(Ordering::Relaxed) {
            Err(cancelled_error())
        } else {
            Ok(())
        }
    }

    async fn run(
        self: &Arc<Self>,
        platform: Platform,
        selected: Vec<Component>,
        mut cancel: watch::Receiver<CancelMode>,
    ) -> AppResult<ManagedEngineConfig> {
        let l = &self.layout;
        tokio::fs::create_dir_all(l.downloads_dir()).await?;
        let mut record = l.load_record().unwrap_or_default();
        if record.manifest_version != self.manifest.manifest_version {
            // 版が変わったら、完了印はすべて無効(取得し直す)
            record.steps.clear();
        }
        record.manifest_version = self.manifest.manifest_version.clone();
        record.comfyui_tag = self.manifest.comfyui.tag.clone();
        record.components = selected.iter().map(|c| c.id.clone()).collect();
        record.complete = false;
        record.accepted_at = now_unix();
        l.save_record(&record)?;

        let m = &self.manifest;
        let py_src = m.python_source(platform);

        // 1. Python
        self.begin_step(1, "python", "Python を取得して展開します");
        if !self.done(&record, "python", &py_src.sha256) || !l.python_exe(platform).is_file() {
            let archive = self.fetch(py_src, "python.tar.gz", &mut cancel).await?;
            let tmp = l.root.join(".extract-python");
            self.extract(&archive, &tmp, ExtractKind::TarGz).await?;
            replace_dir(&tmp.join("python"), &l.python_dir())?;
            let _ = std::fs::remove_dir_all(&tmp);
            self.mark(&mut record, "python", &py_src.sha256)?;
        }
        self.check_cancel()?;

        // 2. venv
        self.begin_step(2, "venv", "仮想環境(venv)を作成します");
        if !self.done(&record, "venv", &py_src.sha256) || !l.venv_python(platform).is_file() {
            let _ = std::fs::remove_dir_all(l.venv_dir());
            self.run_command(
                &l.python_exe(platform),
                &["-m".into(), "venv".into(), l.venv_dir().into_os_string()],
                &l.root,
                &mut cancel,
            )
            .await?;
            self.mark(&mut record, "venv", &py_src.sha256)?;
        }
        self.check_cancel()?;
        let venv_py = l.venv_python(platform);

        // 3. ComfyUI 本体
        self.begin_step(
            3,
            "comfyui",
            &format!("ComfyUI {} を取得して展開します", m.comfyui.tag),
        );
        let src = &m.comfyui.source;
        if !self.done(&record, "comfyui", &src.sha256) || !l.main_py().is_file() {
            let archive = self.fetch(src, "comfyui.zip", &mut cancel).await?;
            let tmp = l.root.join(".extract-comfyui");
            self.extract(&archive, &tmp, ExtractKind::Zip { strip: 1 })
                .await?;
            replace_dir(&tmp, &l.comfyui_dir())?;
            self.mark(&mut record, "comfyui", &src.sha256)?;
        }
        self.check_cancel()?;

        // 4. PyTorch
        let torch_token = format!(
            "{}/{}/{}/{:?}",
            m.torch.version, m.torch.torchvision, m.torch.torchaudio, platform
        );
        self.begin_step(
            4,
            "torch",
            &format!("PyTorch {} を導入します(時間がかかります)", m.torch.version),
        );
        if !self.done(&record, "torch", &torch_token) {
            self.run_command(&venv_py, &self.torch_args(platform), &l.root, &mut cancel)
                .await?;
            self.mark(&mut record, "torch", &torch_token)?;
        }

        // 5. ComfyUI の依存
        self.begin_step(5, "comfyui-deps", "ComfyUI の依存パッケージを導入します");
        let deps_token = format!("{}/{torch_token}", src.sha256);
        if !self.done(&record, "comfyui-deps", &deps_token) {
            self.pip_requirements(
                &venv_py,
                &l.comfyui_dir().join("requirements.txt"),
                &mut cancel,
            )
            .await?;
            self.mark(&mut record, "comfyui-deps", &deps_token)?;
        }

        // 6. カスタムノード
        self.begin_step(6, "nodes", "カスタムノードを導入します");
        for c in &selected {
            self.install_node(
                &mut record,
                &venv_py,
                &c.name,
                &c.repo,
                &c.id,
                &c.source,
                &mut cancel,
            )
            .await?;
            if let Some(e) = &c.extra {
                self.install_node(
                    &mut record,
                    &venv_py,
                    &e.name,
                    &e.repo,
                    &format!("{}-extra", c.id),
                    &e.source,
                    &mut cancel,
                )
                .await?;
            }
        }

        // 7. data の骨組み
        self.begin_step(7, "skeleton", "データフォルダを準備します");
        for sub in [
            "models/checkpoints",
            "models/unet",
            "models/clip",
            "models/vae",
            "models/loras",
            "models/controlnet",
            "models/ultralytics/bbox",
            "custom_nodes",
            "input",
            "output",
            "user",
        ] {
            tokio::fs::create_dir_all(l.data_dir().join(sub)).await?;
        }
        self.check_cancel()?;

        // 8. 検証
        self.begin_step(8, "verify", "ComfyUI を起動して動作を確認します");
        let expect: Vec<String> = selected
            .iter()
            .flat_map(|c| {
                std::iter::once(c.expect_node.clone())
                    .chain(c.extra.iter().map(|e| e.expect_node.clone()))
            })
            .collect();
        self.verify(platform, &expect, &mut cancel).await?;

        record.complete = true;
        l.save_record(&record)?;
        // 一時ファイル(アーカイブ)は不要
        let _ = std::fs::remove_dir_all(l.downloads_dir());
        self.push_log("導入が完了しました");
        Ok(l.managed_config(platform))
    }

    fn done(&self, record: &InstallRecord, id: &str, token: &str) -> bool {
        let ok = record.steps.get(id).is_some_and(|t| t == token);
        if ok {
            self.push_log(&format!("完了済みのため飛ばします: {id}"));
        }
        ok
    }

    fn mark(&self, record: &mut InstallRecord, id: &str, token: &str) -> AppResult<()> {
        record.steps.insert(id.to_string(), token.to_string());
        self.layout.save_record(record)
    }

    fn torch_args(&self, platform: Platform) -> Vec<std::ffi::OsString> {
        let t = &self.manifest.torch;
        let mut a: Vec<std::ffi::OsString> = pip_base();
        a.push(format!("torch=={}", t.version).into());
        a.push(format!("torchvision=={}", t.torchvision).into());
        a.push(format!("torchaudio=={}", t.torchaudio).into());
        if platform == Platform::WindowsX64 {
            a.push("--index-url".into());
            a.push(t.windows_index.clone().into());
        }
        a
    }

    async fn pip_requirements(
        &self,
        venv_py: &Path,
        requirements: &Path,
        cancel: &mut watch::Receiver<CancelMode>,
    ) -> AppResult<()> {
        if !requirements.is_file() {
            self.push_log(&format!(
                "requirements.txt が無いため飛ばします: {}",
                requirements.display()
            ));
            return Ok(());
        }
        let mut args = pip_base();
        args.push("-r".into());
        args.push(requirements.as_os_str().to_os_string());
        self.run_command(venv_py, &args, &self.layout.root, cancel)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn install_node(
        &self,
        record: &mut InstallRecord,
        venv_py: &Path,
        name: &str,
        repo: &str,
        key: &str,
        src: &Source,
        cancel: &mut watch::Receiver<CancelMode>,
    ) -> AppResult<()> {
        let dir_name = repo.rsplit('/').next().unwrap_or(name).to_string();
        let dest = self.layout.nodes_dir().join(&dir_name);
        let id = format!("node:{key}");
        if !self.done(record, &id, &src.sha256) || !dest.is_dir() {
            self.push_log(&format!("-- {name}"));
            let archive = self.fetch(src, &format!("{key}.zip"), cancel).await?;
            let tmp = self.layout.root.join(format!(".extract-{key}"));
            self.extract(&archive, &tmp, ExtractKind::Zip { strip: 1 })
                .await?;
            tokio::fs::create_dir_all(self.layout.nodes_dir()).await?;
            replace_dir(&tmp, &dest)?;
            self.mark(record, &id, &src.sha256)?;
        }
        let deps_id = format!("{id}:deps");
        let deps_token = format!("{}/{}", src.sha256, self.manifest.torch.version);
        if !self.done(record, &deps_id, &deps_token) {
            // WD14 Tagger の requirements は `onnxruntime`(CPU 版)。`onnxruntime-gpu` など別の版が
            // 入っていると、同じフォルダを上書きして壊すため、導入済みなら重ねない(docs/39 §13.2)
            if key == "tagger" && onnxruntime_installed(&self.layout.venv_dir()) {
                self.push_log("onnxruntime が導入済みのため、依存の導入を飛ばします");
            } else {
                self.pip_requirements(venv_py, &dest.join("requirements.txt"), cancel)
                    .await?;
            }
            self.mark(record, &deps_id, &deps_token)?;
        }
        self.check_cancel()
    }

    /// 取得して SHA-256 を照合し、`downloads/<file_name>` に置く(既に正しいものがあれば再利用)。
    async fn fetch(
        &self,
        src: &Source,
        file_name: &str,
        cancel: &mut watch::Receiver<CancelMode>,
    ) -> AppResult<PathBuf> {
        let final_path = self.layout.downloads_dir().join(file_name);
        let part = self
            .layout
            .downloads_dir()
            .join(format!("{file_name}.part"));
        if final_path.is_file() {
            if download::sha256_file(&final_path).await? == src.sha256 {
                self.push_log(&format!("取得済みのファイルを使います: {file_name}"));
                return Ok(final_path);
            }
            let _ = tokio::fs::remove_file(&final_path).await;
        }
        self.push_log(&format!(
            "ダウンロード: {file_name}({} MB)",
            src.size_bytes / 1024 / 1024
        ));
        let client = download::build_client(&self.policy)?;
        let req = Request {
            url: &src.url,
            part_path: &part,
            final_path: &final_path,
            size_bytes: src.size_bytes,
            sha256: &src.sha256,
        };
        let total = src.size_bytes;
        let mut on_progress = |p: Progress| {
            let verifying = p.phase == Phase::Verifying;
            self.update(|s| {
                s.received = Some(p.received);
                s.total = Some(total);
                if verifying {
                    s.received = Some(total);
                }
            });
        };
        match download::download(&client, &self.policy, &req, cancel, &mut on_progress)
            .await
            .map_err(map_download_error)?
        {
            Outcome::Done => Ok(final_path),
            Outcome::Cancelled(_) => Err(cancelled_error()),
        }
    }

    async fn extract(&self, archive: &Path, dest: &Path, kind: ExtractKind) -> AppResult<()> {
        self.push_log(&format!("展開しています: {}", archive.display()));
        let _ = tokio::fs::remove_dir_all(dest).await;
        let (archive, dest) = (archive.to_path_buf(), dest.to_path_buf());
        let flag = Arc::clone(&self.cancel_flag);
        tokio::task::spawn_blocking(move || match kind {
            ExtractKind::TarGz => archive::extract_tar_gz(&archive, &dest, &flag),
            ExtractKind::Zip { strip } => archive::extract_zip(&archive, &dest, strip, &flag),
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
    }

    /// 子プロセスを実行し、出力をログへ流す。失敗・中止ではプロセスグループごと終了させる。
    async fn run_command(
        &self,
        program: &Path,
        args: &[std::ffi::OsString],
        cwd: &Path,
        cancel: &mut watch::Receiver<CancelMode>,
    ) -> AppResult<()> {
        self.push_log(&format!("$ {} {}", program.display(), display_args(args)));
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        apply_env(&mut cmd);
        let mut builder = cmd.group();
        #[cfg(windows)]
        builder.creation_flags(CREATE_NO_WINDOW);
        let mut child = builder.kill_on_drop(true).spawn().map_err(|e| {
            fail(
                "ENGINE_INSTALL_COMMAND_FAILED",
                format!("{} を実行できません: {e}", program.display()),
            )
        })?;
        let mut readers = Vec::new();
        if let Some(out) = child.inner().stdout.take() {
            readers.push(self.spawn_reader(out));
        }
        if let Some(err) = child.inner().stderr.take() {
            readers.push(self.spawn_reader(err));
        }
        let status = loop {
            tokio::select! {
                r = child.wait() => break r,
                _ = cancel.changed() => {
                    if *cancel.borrow() != CancelMode::Run {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        return Err(cancelled_error());
                    }
                }
            }
        };
        let _ = child.kill().await; // 孫プロセスが残らないように
        for r in readers {
            let _ = r.await;
        }
        let status = status.map_err(|e| {
            fail(
                "ENGINE_INSTALL_COMMAND_FAILED",
                format!("プロセスの終了を待てません: {e}"),
            )
        })?;
        if status.success() {
            Ok(())
        } else {
            Err(fail(
                "ENGINE_INSTALL_COMMAND_FAILED",
                format!(
                    "コマンドが失敗しました(終了コード {})。ログを確認してください",
                    status.code().map_or("不明".to_string(), |c| c.to_string())
                ),
            ))
        }
    }

    fn spawn_reader<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
        &self,
        reader: R,
    ) -> tokio::task::JoinHandle<()> {
        let log = Arc::clone(&self.log);
        tokio::spawn(async move {
            let mut r = BufReader::new(reader);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match r.read_until(b'\n', &mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {
                        let text = String::from_utf8_lossy(&buf).into_owned();
                        for seg in text.split(['\r', '\n']).map(str::trim_end) {
                            if !seg.is_empty() {
                                log.push(seg);
                            }
                        }
                    }
                }
            }
        })
    }

    /// ComfyUI を一時的に起動し、期待するノードが現れることを確認して停止する(docs/23 §4.2 の 8)。
    async fn verify(
        &self,
        platform: Platform,
        expect: &[String],
        cancel: &mut watch::Receiver<CancelMode>,
    ) -> AppResult<()> {
        let l = &self.layout;
        let venv_py = l.venv_python(platform);
        // 参考情報: PyTorch と利用できるデバイス
        let _ = self
            .run_command(
                &venv_py,
                &[
                    "-c".into(),
                    "import torch; print('torch', torch.__version__, '/ cuda:', torch.cuda.is_available(), '/ mps:', getattr(torch.backends, 'mps', None) is not None and torch.backends.mps.is_available())".into(),
                ],
                &l.root,
                cancel,
            )
            .await;
        self.check_cancel()?;

        let port = free_port()?;
        let mut cmd = Command::new(&venv_py);
        cmd.args([
            l.main_py().into_os_string(),
            "--listen".into(),
            "127.0.0.1".into(),
            "--port".into(),
            port.to_string().into(),
            "--base-directory".into(),
            l.data_dir().into_os_string(),
            "--disable-auto-launch".into(),
            // 検証では GPU を使わない(起動を速く、GPU の状態に左右されないように)
            "--cpu".into(),
        ])
        .current_dir(l.comfyui_dir())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
        apply_env(&mut cmd);
        let mut builder = cmd.group();
        #[cfg(windows)]
        builder.creation_flags(CREATE_NO_WINDOW);
        let mut child = builder.kill_on_drop(true).spawn().map_err(|e| {
            fail(
                "ENGINE_INSTALL_VERIFY_FAILED",
                format!("ComfyUI を起動できません: {e}"),
            )
        })?;
        let mut readers = Vec::new();
        if let Some(out) = child.inner().stdout.take() {
            readers.push(self.spawn_reader(out));
        }
        if let Some(err) = child.inner().stderr.take() {
            readers.push(self.spawn_reader(err));
        }

        let result = self
            .poll_object_info(&mut child, port, expect, cancel)
            .await;
        let _ = child.kill().await;
        let _ = child.wait().await;
        for r in readers {
            let _ = r.await;
        }
        result
    }

    async fn poll_object_info(
        &self,
        child: &mut AsyncGroupChild,
        port: u16,
        expect: &[String],
        cancel: &mut watch::Receiver<CancelMode>,
    ) -> AppResult<()> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| AppError::internal(e.to_string()))?;
        let url = format!("http://127.0.0.1:{port}/object_info");
        let deadline = Instant::now() + VERIFY_TIMEOUT;
        loop {
            self.check_cancel()?;
            if let Some(status) = child
                .try_wait()
                .map_err(|e| AppError::internal(e.to_string()))?
            {
                return Err(fail(
                    "ENGINE_INSTALL_VERIFY_FAILED",
                    format!(
                        "ComfyUI が起動の途中で終了しました(終了コード {})。ログを確認してください",
                        status.code().map_or("不明".to_string(), |c| c.to_string())
                    ),
                ));
            }
            if let Ok(resp) = client.get(&url).send().await
                && resp.status().is_success()
                && let Ok(body) = resp.bytes().await
            {
                let missing = missing_nodes(&body, expect)?;
                if missing.is_empty() {
                    self.push_log("ComfyUI が応答し、必要なノードを確認しました");
                    return Ok(());
                }
                return Err(fail(
                    "ENGINE_INSTALL_VERIFY_FAILED",
                    format!("必要なノードが見つかりません: {}", missing.join(", ")),
                ));
            }
            if Instant::now() >= deadline {
                return Err(fail(
                    "ENGINE_INSTALL_VERIFY_FAILED",
                    "ComfyUI の起動を確認できませんでした(タイムアウト)。ログを確認してください",
                ));
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                _ = cancel.changed() => {}
            }
        }
    }

    // ------------------------------------------------------------ 削除

    /// 導入したものを削除する。`keep_data` なら `data/`(モデル・出力・設定)は残す。
    pub async fn remove(&self, keep_data: bool) -> AppResult<()> {
        if self.is_running() {
            return Err(fail("ENGINE_BUSY", "導入の実行中は削除できません"));
        }
        let root = self.layout.root.clone();
        let l = self.layout.clone();
        tokio::task::spawn_blocking(move || -> AppResult<()> {
            if !root.exists() {
                return Ok(());
            }
            if keep_data {
                for dir in [
                    l.python_dir(),
                    l.venv_dir(),
                    l.comfyui_dir(),
                    l.downloads_dir(),
                ] {
                    remove_dir_robust(&dir)?;
                }
                let _ = std::fs::remove_file(l.marker());
            } else {
                remove_dir_robust(&root)?;
            }
            Ok(())
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;
        self.update(|s| *s = InstallState::idle());
        Ok(())
    }
}

// ---------------------------------------------------------------- 補助

enum ExtractKind {
    TarGz,
    Zip { strip: usize },
}

fn pip_base() -> Vec<std::ffi::OsString> {
    ["-m", "pip", "install", "--disable-pip-version-check"]
        .iter()
        .map(Into::into)
        .collect()
}

/// pip や Python に渡す環境を固定する。利用者の環境変数で、導入先や取得元が変わらないようにする。
fn apply_env(cmd: &mut Command) {
    cmd.env("PIP_DISABLE_PIP_VERSION_CHECK", "1")
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("PIP_NO_INPUT", "1");
    #[cfg(target_os = "macos")]
    cmd.env("PYTORCH_ENABLE_MPS_FALLBACK", "1");
    for k in [
        "PYTHONHOME",
        "PYTHONPATH",
        "PIP_INDEX_URL",
        "PIP_EXTRA_INDEX_URL",
        "PIP_FIND_LINKS",
        "PIP_CONFIG_FILE",
        "PIP_REQUIRE_VIRTUALENV",
    ] {
        cmd.env_remove(k);
    }
}

fn display_args(args: &[std::ffi::OsString]) -> String {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn map_download_error(e: AppError) -> AppError {
    match e {
        AppError::Models { code, message, .. } => {
            let code = match code {
                "MODELS_HASH_MISMATCH" => "ENGINE_INSTALL_HASH_MISMATCH",
                "MODELS_DISK_FULL" => "ENGINE_INSTALL_DISK_FULL",
                "MODELS_HOST_NOT_ALLOWED" => "ENGINE_INSTALL_HOST_NOT_ALLOWED",
                _ => "ENGINE_INSTALL_DOWNLOAD_FAILED",
            };
            fail(code, message)
        }
        other => other,
    }
}

/// `src` を `dest` へ移す(既存の `dest` は先に削除)。
fn replace_dir(src: &Path, dest: &Path) -> AppResult<()> {
    if dest.exists() {
        remove_dir_robust(dest)?;
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(src, dest).map_err(|e| {
        fail(
            "ENGINE_INSTALL_EXTRACT_FAILED",
            format!("フォルダを移動できません: {e}"),
        )
    })
}

/// 読み取り専用の属性(Windows の `.git` 配下など)があっても消せるようにする。
fn remove_dir_robust(dir: &Path) -> AppResult<()> {
    if !dir.exists() {
        return Ok(());
    }
    if std::fs::remove_dir_all(dir).is_ok() {
        return Ok(());
    }
    fn clear_readonly(p: &Path) {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                let path = e.path();
                if let Ok(meta) = e.metadata() {
                    let mut perm = meta.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    perm.set_readonly(false);
                    let _ = std::fs::set_permissions(&path, perm);
                    if meta.is_dir() {
                        clear_readonly(&path);
                    }
                }
            }
        }
    }
    clear_readonly(dir);
    std::fs::remove_dir_all(dir).map_err(|e| {
        fail(
            "ENGINE_INSTALL_REMOVE_FAILED",
            format!("削除できませんでした({}): {e}", dir.display()),
        )
    })
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn free_port() -> AppResult<u16> {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(l.local_addr()?.port())
}

/// 指定パスか、その最も近い既存の親での空き容量。
fn free_space_near(path: &Path) -> Option<u64> {
    let mut p = Some(path);
    while let Some(dir) = p {
        if dir.exists() {
            return fs4::available_space(dir).ok();
        }
        p = dir.parent();
    }
    None
}

/// Windows: `nvidia-smi -L` の最初の行(GPU 名)。無ければ `None`。
fn detect_nvidia() -> Option<String> {
    let mut cmd = std::process::Command::new("nvidia-smi");
    cmd.arg("-L");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|l| l.contains("GPU"))
        .map(|l| l.trim().to_string())
}

/// `/object_info` の応答に無いノード名を返す。
fn missing_nodes(body: &[u8], expect: &[String]) -> AppResult<Vec<String>> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| {
        fail(
            "ENGINE_INSTALL_VERIFY_FAILED",
            "ComfyUI の応答を解釈できません",
        )
    })?;
    Ok(expect
        .iter()
        .filter(|n| v.get(n.as_str()).is_none())
        .cloned()
        .collect())
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// venv に onnxruntime 系のパッケージ(`onnxruntime` / `onnxruntime-gpu` など)が入っているか。
/// Windows は `Lib/site-packages`、macOS は `lib/python*/site-packages` の `.dist-info` で判定する。
fn onnxruntime_installed(venv: &Path) -> bool {
    let mut dirs = vec![venv.join("Lib").join("site-packages")];
    if let Ok(rd) = std::fs::read_dir(venv.join("lib")) {
        dirs.extend(rd.flatten().map(|e| e.path().join("site-packages")));
    }
    dirs.iter().any(|d| {
        std::fs::read_dir(d).is_ok_and(|rd| {
            rd.flatten().any(|e| {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                name.ends_with(".dist-info")
                    && (name.starts_with("onnxruntime-") || name.starts_with("onnxruntime_"))
            })
        })
    })
}

#[cfg(test)]
mod tests;
