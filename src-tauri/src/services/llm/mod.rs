//! ローカル LLM(Qwen3.5-9B)の導入・起動・メモリ解放(docs/30-local-llm.md)。
//!
//! llama.cpp の `llama-server` を、アプリが `{app_data}/llm/` に導入し、子プロセスとして
//! 起動・停止する。プロセスを終了すれば RAM / VRAM が OS に返る。待ち受けは 127.0.0.1 のみ。

mod installer;
pub mod manifest;

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use self::manifest::Manifest;
use crate::db::Db;
use crate::db::assistant::{self as store, LlmSettings};
use crate::error::{AppError, AppResult};
use crate::services::assistant::{LocalEndpoint, LocalRuntime};
use crate::services::engine_install::manifest::Platform;
use crate::services::engine_process::config::LaunchSpec;
use crate::services::engine_process::{
    EngineProcState, EngineProcess, NAME_LLM, NAME_MT, Probe, ProcSink, ProcStatus,
};
use crate::services::models::download::{CancelMode, DownloadPolicy};

pub use self::installer::{InstallState, InstallStatus};

/// PLaMo Community License の全文(導入前の同意画面でも読めるよう、アプリに同梱する。docs/34 §9.13)。
/// マニフェストの付属ファイル(`LICENSE-ja.txt` / `LICENSE-en.txt`)と同一であることをテストで確認する。
const PLAMO_LICENSE_JA: &str = include_str!("licenses/plamo-community-license-ja.txt");
const PLAMO_LICENSE_EN: &str = include_str!("licenses/plamo-community-license-en.txt");

/// 言語(`ja` / `en`)ごとのライセンス全文。
pub fn plamo_license(lang: &str) -> Option<&'static str> {
    match lang {
        "ja" => Some(PLAMO_LICENSE_JA),
        "en" => Some(PLAMO_LICENSE_EN),
        _ => None,
    }
}

/// 翻訳専用モデルの `LlmService`(docs/34 §9.13)。Tauri の状態として、本体と区別するための包み。
pub struct MtLlm(pub Arc<LlmService>);

/// API で使うモデル名(`llama-server --alias`)。
pub const MODEL_ALIAS: &str = "qwen3.5-9b";
/// 翻訳専用モデルのモデル名(docs/34 §9.13)。
pub const MT_MODEL_ALIAS: &str = "plamo-3-translate";
/// 翻訳専用モデルのコンテキスト長(入力 2,000 文字 + 訳文に十分。小さくして VRAM を抑える)。
const MT_CONTEXT_SIZE: u32 = 4096;
/// コンテキスト長。システムプロンプト(約 8,000 文字)+ フォーム + 履歴 20 件 + 回答(2,048 トークン)に十分な値。
const CONTEXT_SIZE: u32 = 16384;
/// 起動(モデルの読み込み)を待つ上限。
const READY_TIMEOUT: Duration = Duration::from_secs(120);
/// `llama-server` 以外に GPU / OS が使う分の目安(MB)。モデルのサイズに足して表示する。
const MEMORY_OVERHEAD_MB: u64 = 1200;
const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(10);

pub fn llm_error(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::assistant(code, message)
}

// ---------------------------------------------------------------- 状態

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmState {
    NotInstalled,
    Stopped,
    Starting,
    Ready,
    Stopping,
    Error,
}

fn map_state(s: ProcStatus) -> LlmState {
    match s {
        ProcStatus::Stopped => LlmState::Stopped,
        ProcStatus::Starting => LlmState::Starting,
        ProcStatus::Running => LlmState::Ready,
        ProcStatus::Stopping => LlmState::Stopping,
        ProcStatus::Crashed => LlmState::Error,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmStatus {
    /// このビルドの OS で使えるか(Windows x64 / macOS Apple Silicon)。
    pub supported: bool,
    pub installed: bool,
    /// マニフェストの内容が導入済みと異なる(更新できる)。
    pub update_available: bool,
    pub runtime: Option<String>,
    pub model: Option<String>,
    /// ライセンス(導入の同意画面に出す)。
    pub runtime_license: String,
    pub model_license: String,
    /// 導入済みのライセンス全文のファイル名(設定画面の表示用)。
    pub license_files: Vec<String>,
    /// 導入・更新でダウンロードするサイズの目安(MB)。
    pub download_mb: u64,
    pub state: LlmState,
    pub message: Option<String>,
    pub idle_minutes: u32,
    pub release_before_generation: bool,
    /// 起動中・準備完了のときの、メモリ使用量の目安(MB)。
    pub memory_hint_mb: Option<u64>,
    pub install: InstallState,
}

/// フロントエンドへの通知(`llm://state` / `llm://install-progress`)。テストで差し替える。
pub trait LlmSink: Send + Sync {
    fn state(&self, state: LlmState, message: Option<&str>);
    fn install(&self, state: &InstallState);
}

/// `EngineProcess` の状態通知を `LlmSink` へ中継する。
struct ProcBridge(Arc<dyn LlmSink>);

impl ProcSink for ProcBridge {
    fn state(&self, s: &EngineProcState) {
        self.0.state(map_state(s.status), s.message.as_deref());
    }
    fn log(&self, _line: &str) {}
}

// ---------------------------------------------------------------- 配置・導入の記録

#[derive(Debug, Clone)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn bin_dir(&self, build: &str) -> PathBuf {
        self.root.join("bin").join(build)
    }
    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }
    pub fn downloads_dir(&self) -> PathBuf {
        self.root.join("downloads")
    }
    pub fn marker(&self) -> PathBuf {
        self.root.join(".gazooo-llm.json")
    }
    pub fn load_record(&self) -> Option<InstallRecord> {
        let raw = std::fs::read_to_string(self.marker()).ok()?;
        serde_json::from_str(&raw).ok()
    }
}

/// 導入の記録(`<root>/.gazooo-llm.json`)。検証まで完了したときだけ書く。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRecord {
    pub manifest_version: String,
    pub runtime_build: String,
    pub runtime_sha256: String,
    pub model_file: String,
    pub model_sha256: String,
}

// ---------------------------------------------------------------- 起動の引数

pub struct LaunchParams {
    pub exe: PathBuf,
    pub model: PathBuf,
    pub port: u16,
    /// GPU に載せる層数。`None` は `llama-server` の自動調整(`--fit`)に任せる。
    pub gpu_layers: Option<u32>,
}

/// 起動のコマンド。引数は検証済みの値だけで組み立てる。
pub fn default_spec(p: &LaunchParams) -> LaunchSpec {
    let mut args: Vec<OsString> = vec![
        "-m".into(),
        p.model.clone().into_os_string(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        p.port.to_string().into(),
        "--alias".into(),
        MODEL_ALIAS.into(),
        "--jinja".into(),
        "-c".into(),
        CONTEXT_SIZE.to_string().into(),
        // 同時に 1 件だけ処理する(スロットを増やすとメモリが増える)
        "-np".into(),
        "1".into(),
        // プロンプトのキャッシュをメモリに溜めない(既定は最大 8GB。メモリ解放の方針に反する)
        "-cram".into(),
        "0".into(),
        // Qwen3.5 の思考を無効化(11 §13.3)
        "--chat-template-kwargs".into(),
        r#"{"enable_thinking":false}"#.into(),
    ];
    if let Some(n) = p.gpu_layers {
        args.push("-ngl".into());
        args.push(n.to_string().into());
    }
    LaunchSpec {
        program: p.exe.clone(),
        args,
        cwd: p.exe.parent().map(PathBuf::from),
    }
}

/// 翻訳専用モデル(PLaMo 3 Translate)の起動のコマンド(docs/34 §9.13)。
/// 翻訳の向き(`target_lang` / `source_lang`)は要求ごとに送る。`--no-prefill-assistant` は公式の推奨。
pub fn mt_spec(p: &LaunchParams) -> LaunchSpec {
    let mut args: Vec<OsString> = vec![
        "-m".into(),
        p.model.clone().into_os_string(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        p.port.to_string().into(),
        "--alias".into(),
        MT_MODEL_ALIAS.into(),
        "--jinja".into(),
        "--no-prefill-assistant".into(),
        "-c".into(),
        MT_CONTEXT_SIZE.to_string().into(),
        "-np".into(),
        "1".into(),
        "-cram".into(),
        "0".into(),
    ];
    if let Some(n) = p.gpu_layers {
        args.push("-ngl".into());
        args.push(n.to_string().into());
    }
    LaunchSpec {
        program: p.exe.clone(),
        args,
        cwd: p.exe.parent().map(PathBuf::from),
    }
}

type SpecBuilder = Box<dyn Fn(&LaunchParams) -> LaunchSpec + Send + Sync>;

// ---------------------------------------------------------------- サービス

struct Inner {
    port: Option<u16>,
    last_activity: Instant,
    /// 相談(ストリーミング)の実行中の数。0 でないあいだは自動停止しない。
    busy: u32,
    install: InstallState,
    install_cancel: Option<installer::CancelHandle>,
}

pub struct LlmService {
    db: Db,
    layout: Layout,
    manifest: Manifest,
    platform: Option<Platform>,
    /// 取得してよい取得元(本番はマニフェストの許可リスト。テストで差し替える)。
    policy: DownloadPolicy,
    proc: Arc<EngineProcess>,
    sink: Arc<dyn LlmSink>,
    inner: Mutex<Inner>,
    /// 起動の確認(`ensure_ready`)を 1 つにする。
    start_lock: tokio::sync::Mutex<()>,
    spec_builder: SpecBuilder,
    /// API で使うモデル名。
    alias: &'static str,
    /// もう一方のローカル LLM(本体 と 翻訳専用モデル)。同時に 2 つは起動しない(docs/34 §9.4)。
    peer: std::sync::OnceLock<std::sync::Weak<LlmService>>,
}

/// `GET /health` が 200 を返すか(外部クレートを使わない最小の HTTP)。
fn health_probe() -> Probe {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    Arc::new(|port| {
        Box::pin(async move {
            let attempt = async {
                let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
                    .await
                    .ok()?;
                s.write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
                    .await
                    .ok()?;
                let mut head = [0u8; 12];
                s.read_exact(&mut head).await.ok()?;
                Some(&head[9..12] == b"200")
            };
            tokio::time::timeout(Duration::from_secs(2), attempt)
                .await
                .ok()
                .flatten()
                .unwrap_or(false)
        })
    })
}

/// 空いているポートを 1 つ得る(ループバック)。
fn free_port() -> AppResult<u16> {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| {
        llm_error(
            "LLM_START_FAILED",
            format!("空きポートを確保できません: {e}"),
        )
    })?;
    l.local_addr()
        .map(|a| a.port())
        .map_err(|e| llm_error("LLM_START_FAILED", e.to_string()))
}

impl LlmService {
    pub fn new(
        db: Db,
        root: PathBuf,
        manifest: Manifest,
        sink: Arc<dyn LlmSink>,
        log_path: Option<PathBuf>,
    ) -> Arc<Self> {
        Self::with_options(
            db,
            root,
            manifest,
            Platform::current(),
            sink,
            log_path,
            health_probe(),
            READY_TIMEOUT,
            Box::new(default_spec),
            None,
        )
    }

    /// 翻訳専用モデル用(docs/34 §9.13)。導入先・マニフェスト・起動引数だけが本体と違う。
    pub fn new_mt(
        db: Db,
        root: PathBuf,
        manifest: Manifest,
        sink: Arc<dyn LlmSink>,
        log_path: Option<PathBuf>,
    ) -> Arc<Self> {
        Self::build(
            db,
            root,
            manifest,
            Platform::current(),
            sink,
            log_path,
            health_probe(),
            READY_TIMEOUT,
            Box::new(mt_spec),
            None,
            MT_MODEL_ALIAS,
            NAME_MT,
        )
    }

    /// 本体と翻訳専用モデルを互いに結び付ける(起動時に 1 回)。
    pub fn link_peers(a: &Arc<Self>, b: &Arc<Self>) {
        let _ = a.peer.set(Arc::downgrade(b));
        let _ = b.peer.set(Arc::downgrade(a));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_options(
        db: Db,
        root: PathBuf,
        manifest: Manifest,
        platform: Option<Platform>,
        sink: Arc<dyn LlmSink>,
        log_path: Option<PathBuf>,
        probe: Probe,
        ready_timeout: Duration,
        spec_builder: SpecBuilder,
        policy: Option<DownloadPolicy>,
    ) -> Arc<Self> {
        Self::build(
            db,
            root,
            manifest,
            platform,
            sink,
            log_path,
            probe,
            ready_timeout,
            spec_builder,
            policy,
            MODEL_ALIAS,
            NAME_LLM,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        db: Db,
        root: PathBuf,
        manifest: Manifest,
        platform: Option<Platform>,
        sink: Arc<dyn LlmSink>,
        log_path: Option<PathBuf>,
        probe: Probe,
        ready_timeout: Duration,
        spec_builder: SpecBuilder,
        policy: Option<DownloadPolicy>,
        alias: &'static str,
        proc_name: &'static str,
    ) -> Arc<Self> {
        let policy = policy.unwrap_or_else(|| manifest.policy());
        let proc = EngineProcess::with_name(
            Arc::new(ProcBridge(Arc::clone(&sink))),
            log_path,
            probe,
            ready_timeout,
            proc_name,
        );
        Arc::new(Self {
            db,
            layout: Layout { root },
            manifest,
            platform,
            policy,
            proc,
            sink,
            inner: Mutex::new(Inner {
                port: None,
                last_activity: Instant::now(),
                busy: 0,
                install: InstallState::idle(),
                install_cancel: None,
            }),
            start_lock: tokio::sync::Mutex::new(()),
            spec_builder,
            alias,
            peer: std::sync::OnceLock::new(),
        })
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, Inner>> {
        self.inner
            .lock()
            .map_err(|_| AppError::internal("llm lock poisoned"))
    }

    fn exe_path(&self) -> Option<PathBuf> {
        let rec = self.layout.load_record()?;
        let platform = self.platform?;
        let asset = self.manifest.runtime_asset(platform);
        Some(self.layout.bin_dir(&rec.runtime_build).join(&asset.exe))
    }

    fn model_path(&self) -> Option<PathBuf> {
        let rec = self.layout.load_record()?;
        Some(self.layout.models_dir().join(rec.model_file))
    }

    /// 導入済みで、起動に必要なファイルがそろっているか。
    pub fn is_installed(&self) -> bool {
        self.platform.is_some()
            && self.exe_path().is_some_and(|p| p.is_file())
            && self.model_path().is_some_and(|p| p.is_file())
    }

    pub fn logs(&self, lines: usize) -> Vec<String> {
        self.proc.logs(lines)
    }

    async fn settings(&self) -> AppResult<LlmSettings> {
        self.db.call(|c| store::llm_settings(c)).await
    }

    pub async fn status(&self) -> AppResult<LlmStatus> {
        let settings = self.settings().await?;
        Ok(self.status_with(settings))
    }

    fn status_with(&self, settings: LlmSettings) -> LlmStatus {
        let installed = self.is_installed();
        let rec = self.layout.load_record();
        let proc = self.proc.state();
        let state = if installed {
            map_state(proc.status)
        } else {
            LlmState::NotInstalled
        };
        let update_available = installed
            && rec.as_ref().is_some_and(|r| {
                r.runtime_build != self.manifest.runtime.build
                    || r.model_sha256 != self.manifest.model.source.sha256
            });
        let model_mb = self.manifest.model.source.size_bytes / (1024 * 1024);
        let runtime_mb = self
            .platform
            .map(|p| self.manifest.runtime_asset(p).source.size_bytes / (1024 * 1024))
            .unwrap_or(0);
        LlmStatus {
            supported: self.platform.is_some(),
            installed,
            update_available,
            runtime: rec.as_ref().map(|r| r.runtime_build.clone()),
            model: installed.then(|| self.manifest.model_label()),
            runtime_license: self.manifest.runtime.license.clone(),
            model_license: self.manifest.model.license.clone(),
            license_files: self
                .manifest
                .model
                .extra_files
                .iter()
                .filter(|f| self.layout.models_dir().join(&f.file_name).is_file())
                .map(|f| f.file_name.clone())
                .collect(),
            download_mb: model_mb + runtime_mb,
            state,
            message: proc.message,
            idle_minutes: settings.idle_minutes,
            release_before_generation: settings.release_before_generation,
            memory_hint_mb: matches!(proc.status, ProcStatus::Starting | ProcStatus::Running)
                .then_some(model_mb + MEMORY_OVERHEAD_MB),
            install: self
                .lock()
                .map(|g| g.install.clone())
                .unwrap_or_else(|_| InstallState::idle()),
        }
    }

    pub async fn update_settings(
        &self,
        idle_minutes: Option<u32>,
        release_before_generation: Option<bool>,
    ) -> AppResult<LlmStatus> {
        self.db
            .call(move |c| store::set_llm_settings(c, idle_minutes, release_before_generation))
            .await?;
        // 自動停止の時間を短くしたときに、すぐ反映されるようにする
        self.status().await
    }

    // ---- 起動・停止 ----

    fn touch(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.last_activity = Instant::now();
        }
    }

    /// 1 回起動する(準備完了は待たない)。
    async fn launch(&self, gpu_layers: Option<u32>) -> AppResult<()> {
        let (exe, model) = match (self.exe_path(), self.model_path()) {
            (Some(e), Some(m)) if e.is_file() && m.is_file() => (e, m),
            _ => {
                return Err(llm_error(
                    "LLM_NOT_INSTALLED",
                    "ローカル LLM が導入されていません。設定の「AI サポート」から導入してください",
                ));
            }
        };
        let port = free_port()?;
        let spec = (self.spec_builder)(&LaunchParams {
            exe,
            model,
            port,
            gpu_layers,
        });
        self.lock()?.port = Some(port);
        self.touch();
        self.proc.start_spec(spec, port).await.map_err(|e| {
            llm_error(
                "LLM_START_FAILED",
                format!("ローカル LLM を起動できません: {e}"),
            )
        })?;
        Ok(())
    }

    /// 起動を始める(設定の「起動」ボタン用)。準備完了は待たず、状態はイベントで通知する。
    pub async fn start(self: &Arc<Self>) -> AppResult<LlmStatus> {
        if self.lock()?.install.status == InstallStatus::Running {
            return Err(llm_error("LLM_BUSY", "導入中は起動できません"));
        }
        match self.proc.state().status {
            ProcStatus::Starting | ProcStatus::Running => {}
            _ => {
                let me = Arc::clone(self);
                tokio::spawn(async move {
                    if let Err(e) = me.ensure_ready().await {
                        tracing::warn!(code = e.code(), "llm start failed");
                    }
                });
                // 状態が Starting になるのを少し待ってから返す(画面にすぐ反映するため)
                for _ in 0..20 {
                    if self.proc.state().status != ProcStatus::Stopped {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
        self.status().await
    }

    /// 停止してメモリを解放する。
    pub async fn stop(&self) -> AppResult<LlmStatus> {
        self.proc.stop().await?;
        self.status().await
    }

    /// 準備完了まで待つ。止まっていれば起動する。VRAM 不足などで落ちたら、GPU を使わずに 1 回だけ再試行する。
    pub async fn ensure_ready_endpoint(&self) -> AppResult<LocalEndpoint> {
        let _guard = self.start_lock.lock().await;
        self.stop_peer().await?;
        if !self.is_installed() {
            return Err(llm_error(
                "LLM_NOT_INSTALLED",
                "ローカル LLM が導入されていません。設定の「AI サポート」から導入してください",
            ));
        }
        // 起動した回数(0 = まだ起動していない。1 = 通常、2 = GPU なしの再試行)
        let mut launched = 0;
        let deadline = Instant::now() + READY_TIMEOUT + Duration::from_secs(60);
        loop {
            let state = self.proc.state();
            match state.status {
                ProcStatus::Running => {
                    self.touch();
                    let port = self
                        .lock()?
                        .port
                        .ok_or_else(|| AppError::internal("llm port unknown"))?;
                    return Ok(LocalEndpoint {
                        base_url: format!("http://127.0.0.1:{port}/v1"),
                        model: self.alias.to_string(),
                    });
                }
                ProcStatus::Stopped => {
                    launched += 1;
                    self.launch(None).await?;
                }
                ProcStatus::Crashed => {
                    // 異常終了の表示を消して Stopped に戻す(前回の異常終了が残っていただけのときもある)
                    self.proc.stop().await?;
                    match launched {
                        0 => {
                            launched = 1;
                            self.launch(None).await?;
                        }
                        1 => {
                            tracing::warn!("llm crashed on start; retrying without GPU offload");
                            launched = 2;
                            self.launch(Some(0)).await?;
                        }
                        _ => return Err(self.start_failed(state.message)),
                    }
                }
                ProcStatus::Starting | ProcStatus::Stopping => {}
            }
            if Instant::now() >= deadline {
                return Err(self.start_failed(Some("起動の確認がタイムアウトしました".into())));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// もう一方のローカル LLM が起動中なら止める(VRAM の競合を避ける)。使用中なら断る。
    async fn stop_peer(&self) -> AppResult<()> {
        let Some(peer) = self.peer.get().and_then(std::sync::Weak::upgrade) else {
            return Ok(());
        };
        if !peer.proc.has_child() {
            return Ok(());
        }
        if peer.inner.lock().map(|g| g.busy > 0).unwrap_or(false) {
            return Err(llm_error(
                "LLM_BUSY",
                "もう一方のローカル LLM が使用中です。完了してからやり直してください",
            ));
        }
        tracing::info!("stopping the other local llm before starting this one");
        peer.proc.stop().await?;
        Ok(())
    }

    pub async fn ensure_ready(&self) -> AppResult<()> {
        self.ensure_ready_endpoint().await.map(|_| ())
    }

    fn start_failed(&self, message: Option<String>) -> AppError {
        let tail = self
            .proc
            .logs(200)
            .into_iter()
            .rev()
            .find(|l| {
                let l = l.to_ascii_lowercase();
                l.contains("error") || l.contains("failed") || l.contains("out of memory")
            })
            .map(|l| format!("({})", l.chars().take(160).collect::<String>()))
            .unwrap_or_default();
        llm_error(
            "LLM_START_FAILED",
            format!(
                "ローカル LLM を起動できませんでした。{}{tail}",
                message.map(|m| format!("{m} ")).unwrap_or_default()
            ),
        )
    }

    // ---- メモリの解放(docs/30 §5) ----

    pub fn use_started(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.busy += 1;
        }
    }

    pub fn use_finished(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.busy = g.busy.saturating_sub(1);
            g.last_activity = Instant::now();
        }
    }

    /// アイドルのタイムアウトを確認し、超えていれば停止する。停止したら `true`。
    pub async fn idle_tick(&self, now: Instant) -> bool {
        if self.proc.state().status != ProcStatus::Running {
            return false;
        }
        let Ok(settings) = self.settings().await else {
            return false;
        };
        let due = {
            let Ok(g) = self.inner.lock() else {
                return false;
            };
            settings.idle_minutes > 0
                && g.busy == 0
                && now.saturating_duration_since(g.last_activity)
                    >= Duration::from_secs(u64::from(settings.idle_minutes) * 60)
        };
        if !due {
            return false;
        }
        tracing::info!("llm idle timeout: stopping to release memory");
        let _ = self.proc.stop().await;
        true
    }

    /// 一定間隔でアイドルを確認する(アプリの寿命と同じ)。
    pub async fn run_idle_monitor(self: Arc<Self>) {
        loop {
            tokio::time::sleep(IDLE_CHECK_INTERVAL).await;
            self.idle_tick(Instant::now()).await;
        }
    }

    /// 画像生成の開始前に、設定に応じて停止して VRAM を空ける。停止したら `true`。
    pub async fn release_for_generation(&self) -> bool {
        let Ok(settings) = self.settings().await else {
            return false;
        };
        if !settings.release_before_generation {
            return false;
        }
        if !matches!(
            self.proc.state().status,
            ProcStatus::Starting | ProcStatus::Running
        ) {
            return false;
        }
        if self.inner.lock().map(|g| g.busy > 0).unwrap_or(false) {
            // 回答の生成中は止めない(終わった後のアイドル停止か、次の生成で解放される)
            return false;
        }
        tracing::info!("llm stopping before image generation");
        let _ = self.proc.stop().await;
        true
    }

    /// アプリ終了時の停止。
    pub async fn shutdown(&self) {
        let _ = self.proc.stop().await;
        if let Some(c) = self.lock().ok().and_then(|mut g| g.install_cancel.take()) {
            c.cancel(CancelMode::Cancel);
        }
    }

    /// 導入物を削除する(停止中のみ)。
    pub async fn remove(&self) -> AppResult<LlmStatus> {
        if self.proc.has_child() || self.lock()?.install.status == InstallStatus::Running {
            return Err(llm_error(
                "LLM_BUSY",
                "起動中または導入中は削除できません。停止してからやり直してください",
            ));
        }
        let root = self.layout.root.clone();
        tokio::task::spawn_blocking(move || match std::fs::remove_dir_all(&root) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(AppError::Storage(e.to_string())),
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;
        if let Ok(mut g) = self.lock() {
            g.install = InstallState::idle();
            g.port = None;
        }
        self.sink.state(LlmState::NotInstalled, None);
        self.status().await
    }
}

#[async_trait::async_trait]
impl LocalRuntime for LlmService {
    fn installed(&self) -> bool {
        self.is_installed()
    }

    async fn ensure_ready(&self) -> AppResult<LocalEndpoint> {
        self.ensure_ready_endpoint().await
    }

    fn use_started(&self) {
        LlmService::use_started(self);
    }

    fn use_finished(&self) {
        LlmService::use_finished(self);
    }
}

#[async_trait::async_trait]
impl crate::services::jobs::GenerationHook for LlmService {
    async fn before_generation(&self) {
        self.release_for_generation().await;
        if let Some(peer) = self.peer.get().and_then(std::sync::Weak::upgrade) {
            peer.release_for_generation().await;
        }
    }
}

#[cfg(test)]
mod tests;
