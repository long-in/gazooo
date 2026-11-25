//! ComfyUI の起動管理(docs/15)。
//!
//! アプリが起動した子プロセスだけを管理する(他のプロセスは停止・再起動しない)。
//! 子プロセスは Windows では Job Object、Unix ではプロセスグループに入れ、アプリの終了時に残らないようにする。

pub mod config;
pub mod known;

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use command_group::{AsyncCommandGroup, AsyncGroupChild};
use futures_util::future::BoxFuture;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::sync::{Notify, oneshot};

use self::config::{LaunchSpec, ManagedEngineConfig};
use crate::error::{AppError, AppResult};

/// UI に保持するログの行数。
const LOG_LINES: usize = 500;
/// 1 行の最大文字数(進捗バーなどで極端に長くならないように)。
const LOG_LINE_MAX: usize = 2000;
/// ログファイルの上限(超えたら書かない。UI のリングバッファは続く)。
const LOG_FILE_MAX: u64 = 20 * 1024 * 1024;
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const READY_INTERVAL: Duration = Duration::from_millis(500);
/// Unix で SIGTERM を送ってから強制終了するまでの猶予。
const STOP_GRACE: Duration = Duration::from_secs(5);
const PORT_FREE_TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineProcState {
    pub status: ProcStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// 異常終了のときの終了コード。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// 失敗の理由(短文)。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// アプリが起動した子プロセスが存在するか。
    pub managed_by_app: bool,
}

impl EngineProcState {
    fn stopped() -> Self {
        Self {
            status: ProcStatus::Stopped,
            pid: None,
            exit_code: None,
            message: None,
            managed_by_app: false,
        }
    }

    fn crashed(message: impl Into<String>, exit_code: Option<i32>) -> Self {
        Self {
            status: ProcStatus::Crashed,
            pid: None,
            exit_code,
            message: Some(message.into()),
            managed_by_app: false,
        }
    }

    fn with_child(status: ProcStatus, pid: Option<u32>) -> Self {
        Self {
            status,
            pid,
            exit_code: None,
            message: None,
            managed_by_app: true,
        }
    }
}

/// フロントエンドへの通知(`engine://proc-state` / `engine://log`)。テストで差し替える。
pub trait ProcSink: Send + Sync {
    fn state(&self, state: &EngineProcState);
    fn log(&self, line: &str);
}

/// ComfyUI が応答しているか(ポート番号を受け取る)。テストで差し替える。
pub type Probe = Arc<dyn Fn(u16) -> BoxFuture<'static, bool> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopReason {
    /// ユーザーの操作(停止・再起動・アプリ終了)。
    Requested,
    /// 起動の確認がタイムアウトした。
    StartTimeout,
}

struct Inner {
    state: EngineProcState,
    /// 起動のたびに増える。古い監視タスクの結果を捨てるため。
    epoch: u64,
    stop_tx: Option<oneshot::Sender<StopReason>>,
    logs: VecDeque<String>,
    log_file: Option<(std::fs::File, u64)>,
}

pub struct EngineProcess {
    inner: Mutex<Inner>,
    changed: Notify,
    sink: Arc<dyn ProcSink>,
    log_path: Option<PathBuf>,
    probe: Probe,
    ready_timeout: Duration,
    /// メッセージに出す名前(既定は「ComfyUI」。ローカル LLM の管理でも再利用する。docs/30 §4)。
    name: &'static str,
}

/// ローカル LLM(docs/30)の管理で使う、`llama-server` 用の起動管理。
pub const NAME_LLM: &str = "ローカル LLM";
/// 翻訳専用モデル(docs/34 §9.13)の `llama-server` 用。
pub const NAME_MT: &str = "翻訳専用モデル";

fn launch_failed(message: impl Into<String>) -> AppError {
    AppError::engine_proc("ENGINE_LAUNCH_FAILED", message)
}

/// 127.0.0.1 のそのポートで、誰かが待ち受けているか。
async fn port_in_use(port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_millis(500),
            TcpStream::connect(("127.0.0.1", port))
        )
        .await,
        Ok(Ok(_))
    )
}

async fn wait_port_free(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while port_in_use(port).await {
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    true
}

/// 本番の応答確認: `GET /system_stats` が 200 を返すか(外部クレートを使わない最小の HTTP)。
fn http_probe() -> Probe {
    Arc::new(|port| {
        Box::pin(async move {
            let attempt = async {
                let mut s = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
                s.write_all(b"GET /system_stats HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
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

impl EngineProcess {
    pub fn new(sink: Arc<dyn ProcSink>, log_path: Option<PathBuf>) -> Arc<Self> {
        Self::with_options(sink, log_path, http_probe(), READY_TIMEOUT)
    }

    pub fn with_options(
        sink: Arc<dyn ProcSink>,
        log_path: Option<PathBuf>,
        probe: Probe,
        ready_timeout: Duration,
    ) -> Arc<Self> {
        Self::with_name(sink, log_path, probe, ready_timeout, "ComfyUI")
    }

    /// 管理する対象の名前を指定して作る(メッセージ用)。
    pub fn with_name(
        sink: Arc<dyn ProcSink>,
        log_path: Option<PathBuf>,
        probe: Probe,
        ready_timeout: Duration,
        name: &'static str,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                state: EngineProcState::stopped(),
                epoch: 0,
                stop_tx: None,
                logs: VecDeque::new(),
                log_file: None,
            }),
            changed: Notify::new(),
            sink,
            log_path,
            probe,
            ready_timeout,
            name,
        })
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, Inner>> {
        self.inner
            .lock()
            .map_err(|_| AppError::internal("engine process lock poisoned"))
    }

    pub fn state(&self) -> EngineProcState {
        self.lock()
            .map(|g| g.state.clone())
            .unwrap_or_else(|_| EngineProcState::stopped())
    }

    /// アプリが起動した子プロセスがあるか(起動中・稼働中・停止処理中)。
    pub fn has_child(&self) -> bool {
        self.state().managed_by_app
    }

    pub fn logs(&self, lines: usize) -> Vec<String> {
        let Ok(g) = self.lock() else {
            return Vec::new();
        };
        let skip = g.logs.len().saturating_sub(lines);
        g.logs.iter().skip(skip).cloned().collect()
    }

    /// 現在の `epoch` のときだけ状態を更新して通知する。
    fn set_state(&self, epoch: u64, new: EngineProcState) {
        let changed = {
            let Ok(mut g) = self.inner.lock() else { return };
            if g.epoch != epoch || g.state == new {
                false
            } else {
                g.state = new.clone();
                true
            }
        };
        if changed {
            self.sink.state(&new);
        }
        self.changed.notify_waiters();
    }

    fn push_log(&self, line: &str) {
        let line: String = strip_ansi(line).chars().take(LOG_LINE_MAX).collect();
        if let Ok(mut g) = self.inner.lock() {
            if g.logs.len() >= LOG_LINES {
                g.logs.pop_front();
            }
            g.logs.push_back(line.clone());
            if let Some((file, written)) = g.log_file.as_mut()
                && *written < LOG_FILE_MAX
                && writeln!(file, "{line}").is_ok()
            {
                *written += line.len() as u64 + 1;
            }
        }
        self.sink.log(&line);
    }

    /// 設定どおりに起動する。
    pub async fn start(self: &Arc<Self>, cfg: &ManagedEngineConfig) -> AppResult<EngineProcState> {
        self.start_spec(cfg.launch_spec()?, cfg.port).await
    }

    /// 設定どおりに再起動する。
    pub async fn restart(
        self: &Arc<Self>,
        cfg: &ManagedEngineConfig,
    ) -> AppResult<EngineProcState> {
        self.restart_spec(cfg.launch_spec()?, cfg.port).await
    }

    pub async fn restart_spec(
        self: &Arc<Self>,
        spec: LaunchSpec,
        port: u16,
    ) -> AppResult<EngineProcState> {
        self.stop().await?;
        if !wait_port_free(port, PORT_FREE_TIMEOUT).await {
            return Err(port_in_use_error(self.name, port));
        }
        self.start_spec(spec, port).await
    }

    pub async fn start_spec(
        self: &Arc<Self>,
        spec: LaunchSpec,
        port: u16,
    ) -> AppResult<EngineProcState> {
        // 先に「起動中」として確保する(同時の起動要求を 1 つにする)
        let epoch = {
            let mut g = self.lock()?;
            match g.state.status {
                ProcStatus::Starting | ProcStatus::Running => return Ok(g.state.clone()),
                ProcStatus::Stopping => {
                    return Err(AppError::engine_proc(
                        "ENGINE_BUSY",
                        format!(
                            "{} を停止しています。完了してからやり直してください",
                            self.name
                        ),
                    ));
                }
                ProcStatus::Stopped | ProcStatus::Crashed => {}
            }
            g.epoch += 1;
            g.logs.clear();
            g.log_file = self
                .log_path
                .as_ref()
                .and_then(|p| std::fs::File::create(p).ok())
                .map(|f| (f, 0));
            g.state = EngineProcState::with_child(ProcStatus::Starting, None);
            g.epoch
        };
        self.sink.state(&self.state());
        self.changed.notify_waiters();

        if port_in_use(port).await {
            let err = port_in_use_error(self.name, port);
            self.set_state(epoch, EngineProcState::crashed(err.to_string(), None));
            return Err(err);
        }

        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("PYTHONUNBUFFERED", "1")
            .env("PYTHONIOENCODING", "utf-8");
        #[cfg(target_os = "macos")]
        cmd.env("PYTORCH_ENABLE_MPS_FALLBACK", "1");
        if let Some(cwd) = &spec.cwd {
            cmd.current_dir(cwd);
        }
        let mut builder = cmd.group();
        #[cfg(windows)]
        builder.creation_flags(CREATE_NO_WINDOW);
        let mut child = match builder.kill_on_drop(true).spawn() {
            Ok(c) => c,
            Err(e) => {
                let err = launch_failed(format!("{} を起動できません: {e}", self.name));
                self.set_state(epoch, EngineProcState::crashed(err.to_string(), None));
                return Err(err);
            }
        };

        let pid = child.id();
        if let Some(out) = child.inner().stdout.take() {
            tokio::spawn(read_lines(Arc::clone(self), out));
        }
        if let Some(err) = child.inner().stderr.take() {
            tokio::spawn(read_lines(Arc::clone(self), err));
        }
        let (stop_tx, stop_rx) = oneshot::channel();
        // 起動の途中で停止が要求されていたら、起動した直後に終了させる
        let early_stop = {
            let mut g = self.lock()?;
            if g.epoch == epoch && g.state.status == ProcStatus::Stopping {
                Some(stop_tx)
            } else {
                if g.epoch == epoch {
                    g.stop_tx = Some(stop_tx);
                }
                None
            }
        };
        match early_stop {
            Some(tx) => {
                let _ = tx.send(StopReason::Requested);
            }
            None => self.set_state(
                epoch,
                EngineProcState::with_child(ProcStatus::Starting, pid),
            ),
        }
        tracing::info!(?pid, port, "comfyui process started");

        tokio::spawn(Arc::clone(self).supervise(epoch, child, stop_rx));
        tokio::spawn(Arc::clone(self).watch_ready(epoch, port));
        Ok(self.state())
    }

    /// 子プロセスの終了を見張る。停止要求が来たら終了させる。
    async fn supervise(
        self: Arc<Self>,
        epoch: u64,
        mut child: AsyncGroupChild,
        mut stop_rx: oneshot::Receiver<StopReason>,
    ) {
        let (reason, status) = tokio::select! {
            r = child.wait() => (None, r),
            r = &mut stop_rx => {
                let reason = r.unwrap_or(StopReason::Requested);
                terminate(&mut child).await;
                (Some(reason), child.wait().await)
            }
        };
        // 本体が終了しても孫プロセスが残らないよう、グループごと終了させる(失敗は無視)
        let _ = child.kill().await;
        let code = status.ok().and_then(|s| s.code());
        let new = match reason {
            Some(StopReason::Requested) => EngineProcState::stopped(),
            Some(StopReason::StartTimeout) => EngineProcState::crashed(
                format!(
                    "起動の確認がタイムアウトしました({} の準備に時間がかかっています)",
                    self.name
                ),
                code,
            ),
            None => EngineProcState::crashed(
                match code {
                    Some(c) => format!("{} が予期せず終了しました(終了コード {c})", self.name),
                    None => format!("{} が予期せず終了しました", self.name),
                },
                code,
            ),
        };
        if let Ok(mut g) = self.inner.lock()
            && g.epoch == epoch
        {
            g.stop_tx = None;
        }
        tracing::info!(?reason, ?code, "comfyui process ended");
        self.set_state(epoch, new);
    }

    /// 応答が返るまで待ち、`running` にする。時間内に返らなければ終了させる。
    async fn watch_ready(self: Arc<Self>, epoch: u64, port: u16) {
        let deadline = Instant::now() + self.ready_timeout;
        loop {
            let pid = {
                let Ok(g) = self.inner.lock() else { return };
                if g.epoch != epoch || g.state.status != ProcStatus::Starting {
                    return;
                }
                g.state.pid
            };
            if (self.probe)(port).await {
                self.set_state(epoch, EngineProcState::with_child(ProcStatus::Running, pid));
                return;
            }
            if Instant::now() >= deadline {
                let tx = self
                    .inner
                    .lock()
                    .ok()
                    .and_then(|mut g| (g.epoch == epoch).then(|| g.stop_tx.take()).flatten());
                if let Some(tx) = tx {
                    let _ = tx.send(StopReason::StartTimeout);
                }
                return;
            }
            tokio::time::sleep(READY_INTERVAL).await;
        }
    }

    /// 停止する。アプリが起動した子プロセスがなければ何もしない(異常終了の表示は消す)。
    pub async fn stop(self: &Arc<Self>) -> AppResult<EngineProcState> {
        let tx = {
            let mut g = self.lock()?;
            match g.state.status {
                ProcStatus::Stopped => return Ok(g.state.clone()),
                ProcStatus::Crashed => {
                    g.state = EngineProcState::stopped();
                    let s = g.state.clone();
                    drop(g);
                    self.sink.state(&s);
                    self.changed.notify_waiters();
                    return Ok(s);
                }
                ProcStatus::Starting | ProcStatus::Running => {
                    let pid = g.state.pid;
                    g.state = EngineProcState::with_child(ProcStatus::Stopping, pid);
                    g.stop_tx.take()
                }
                ProcStatus::Stopping => None,
            }
        };
        self.sink.state(&self.state());
        if let Some(tx) = tx {
            let _ = tx.send(StopReason::Requested);
        }
        self.wait_until_idle(STOP_GRACE + Duration::from_secs(10))
            .await;
        Ok(self.state())
    }

    async fn wait_until_idle(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let notified = self.changed.notified();
            if matches!(
                self.state().status,
                ProcStatus::Stopped | ProcStatus::Crashed
            ) {
                return;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || tokio::time::timeout(left, notified).await.is_err() {
                return;
            }
        }
    }
}

/// 端末の色指定(`ESC [ ... m` など)を取り除く。
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

fn port_in_use_error(name: &str, port: u16) -> AppError {
    AppError::engine_proc(
        "ENGINE_PORT_IN_USE",
        format!(
            "ポート {port} は既に使われています。別の {name} が起動している可能性があります(他のプロセスは終了しません)"
        ),
    )
}

/// 子プロセスを終了させる。Unix では SIGTERM → 猶予 → 強制終了。Windows は Job Object を終了する。
async fn terminate(child: &mut AsyncGroupChild) {
    #[cfg(unix)]
    {
        use command_group::{Signal, UnixChildExt};
        if child.signal(Signal::SIGTERM).is_ok()
            && tokio::time::timeout(STOP_GRACE, child.wait()).await.is_ok()
        {
            return;
        }
    }
    let _ = child.kill().await;
}

/// 出力を行(`\n` と `\r` で区切る)ごとに読み、ログへ流す。
async fn read_lines<R: tokio::io::AsyncRead + Unpin>(svc: Arc<EngineProcess>, reader: R) {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                let text = String::from_utf8_lossy(&buf);
                for seg in text.split(['\r', '\n']).map(str::trim_end) {
                    if !seg.is_empty() {
                        svc.push_log(seg);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
