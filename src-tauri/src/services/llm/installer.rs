//! ローカル LLM の導入(docs/30 §3)。ランタイム(llama.cpp)とモデル(GGUF)を取得・検証して配置する。
//!
//! ダウンロードは 12 の基盤(許可リスト・再開・SHA-256 検証)を再利用する。各手順は冪等で、
//! 検証済みのものは取り直さない。記録(`.gazooo-llm.json`)は、すべて検証できたときだけ書く。

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tokio::sync::watch;

use super::{InstallRecord, LlmService, llm_error};
use crate::error::{AppError, AppResult};
use crate::services::engine_install::archive;
use crate::services::models::download::{
    self, CancelMode, DownloadPolicy, Outcome, Phase, Progress, Request, sha256_file,
};

const TOTAL_STEPS: u32 = 3;
/// 空き容量の余裕(MB)。展開中は圧縮ファイルと展開後が同時にある。
const DISK_MARGIN_MB: u64 = 512;

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
    pub install_id: Option<String>,
    pub status: InstallStatus,
    /// 現在の手順(1 始まり)。実行中以外は 0。
    pub step: u32,
    pub total_steps: u32,
    /// `runtime` / `extract` / `model` / `license`
    pub step_id: Option<String>,
    pub received: Option<u64>,
    pub total: Option<u64>,
    pub error: Option<InstallErrorView>,
}

impl InstallState {
    pub fn idle() -> Self {
        Self {
            install_id: None,
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

/// 実行中の導入を中断するための部品。
pub(crate) struct CancelHandle {
    id: String,
    flag: Arc<AtomicBool>,
    tx: watch::Sender<CancelMode>,
}

impl CancelHandle {
    pub(crate) fn cancel(&self, mode: CancelMode) {
        self.flag.store(true, Ordering::Relaxed);
        let _ = self.tx.send(mode);
    }
}

fn failed(code: &'static str, message: impl Into<String>) -> AppError {
    llm_error(code, message)
}

fn cancelled_error() -> AppError {
    failed("LLM_INSTALL_CANCELLED", "導入を中止しました")
}

fn io_error(e: std::io::Error) -> AppError {
    if e.kind() == std::io::ErrorKind::StorageFull {
        failed("LLM_INSTALL_FAILED", "ディスクの空き容量が足りません")
    } else {
        AppError::Storage(e.to_string())
    }
}

/// ダウンロード基盤のエラーを、導入のエラーコードに読み替える。
fn map_download_error(e: AppError) -> AppError {
    match e.code() {
        "MODELS_HASH_MISMATCH" => failed(
            "LLM_INSTALL_HASH_MISMATCH",
            "ダウンロードしたファイルの検証に失敗しました。もう一度お試しください",
        ),
        "MODELS_DISK_FULL" => failed("LLM_INSTALL_FAILED", "ディスクの空き容量が足りません"),
        "MODELS_HOST_NOT_ALLOWED" | "MODELS_DOWNLOAD_FAILED" => {
            failed("LLM_INSTALL_FAILED", e.to_string())
        }
        _ => e,
    }
}

/// `path`(または存在する最も近い親)がある場所の空き容量(バイト)。
fn available_bytes(path: &Path) -> Option<u64> {
    let mut p = Some(path);
    while let Some(dir) = p {
        if dir.exists() {
            return fs4::available_space(dir).ok();
        }
        p = dir.parent();
    }
    None
}

impl LlmService {
    fn set_install(&self, state: InstallState) {
        if let Ok(mut g) = self.inner.lock() {
            g.install = state.clone();
        }
        self.sink.install(&state);
    }

    /// 導入を始める。進捗は `llm://install-progress` で通知する。
    pub async fn install_start(self: &Arc<Self>) -> AppResult<String> {
        let Some(platform) = self.platform else {
            return Err(failed(
                "LLM_INSTALL_FAILED",
                "この OS ではローカル LLM の導入に対応していません(Windows x64 / macOS Apple Silicon のみ)",
            ));
        };
        if self.proc.has_child() {
            return Err(llm_error(
                "LLM_BUSY",
                "ローカル LLM が起動中です。停止してから導入してください",
            ));
        }
        let install_id = uuid::Uuid::now_v7().to_string();
        let (tx, rx) = watch::channel(CancelMode::Run);
        let flag = Arc::new(AtomicBool::new(false));
        {
            let mut g = self.lock()?;
            if g.install.status == InstallStatus::Running {
                return Err(llm_error("LLM_BUSY", "導入はすでに実行中です"));
            }
            g.install_cancel = Some(CancelHandle {
                id: install_id.clone(),
                flag: Arc::clone(&flag),
                tx,
            });
        }
        let mut st = InstallState::idle();
        st.install_id = Some(install_id.clone());
        st.status = InstallStatus::Running;
        st.step = 1;
        self.set_install(st);

        let me = Arc::clone(self);
        let id = install_id.clone();
        tokio::spawn(async move {
            let result = me.run_install(platform, &id, rx, &flag).await;
            let mut st = InstallState::idle();
            st.install_id = Some(id.clone());
            match result {
                Ok(()) => {
                    st.status = InstallStatus::Done;
                    st.step = TOTAL_STEPS;
                }
                Err(e) if e.code() == "LLM_INSTALL_CANCELLED" => {
                    st.status = InstallStatus::Cancelled;
                }
                Err(e) => {
                    tracing::warn!(code = e.code(), "llm install failed");
                    st.status = InstallStatus::Failed;
                    st.error = Some(InstallErrorView {
                        code: e.code().to_string(),
                        message: e.to_string(),
                    });
                }
            }
            if let Ok(mut g) = me.inner.lock()
                && g.install_cancel.as_ref().is_some_and(|c| c.id == id)
            {
                g.install_cancel = None;
            }
            me.set_install(st);
        });
        Ok(install_id)
    }

    /// 導入を中断する(取得途中のファイルは再開用に残す)。
    pub fn install_cancel(&self, install_id: &str) {
        if let Ok(g) = self.inner.lock()
            && let Some(c) = g.install_cancel.as_ref()
            && c.id == install_id
        {
            c.cancel(CancelMode::Cancel);
        }
    }

    async fn run_install(
        &self,
        platform: crate::services::engine_install::manifest::Platform,
        install_id: &str,
        mut cancel: watch::Receiver<CancelMode>,
        flag: &AtomicBool,
    ) -> AppResult<()> {
        let policy = self.policy.clone();
        let client = download::build_client(&policy)?;
        let asset = self.manifest.runtime_asset(platform).clone();
        let model = self.manifest.model.clone();
        let build = self.manifest.runtime.build.clone();
        let layout = self.layout.clone();

        // 事前チェック: 空き容量(取得するファイル + 展開後 + 余裕)
        let need = (asset.source.size_bytes * 3)
            .saturating_add(model.source.size_bytes)
            .saturating_add(DISK_MARGIN_MB * 1024 * 1024);
        std::fs::create_dir_all(&layout.root).map_err(io_error)?;
        if let Some(free) = available_bytes(&layout.root)
            && free < need
        {
            return Err(failed(
                "LLM_INSTALL_FAILED",
                format!(
                    "ディスクの空き容量が足りません(必要の目安 {} GB、空き {} GB)",
                    need.div_ceil(1 << 30),
                    free >> 30
                ),
            ));
        }
        std::fs::create_dir_all(layout.downloads_dir()).map_err(io_error)?;
        std::fs::create_dir_all(layout.models_dir()).map_err(io_error)?;

        // 1. ランタイム
        let exe = layout.bin_dir(&build).join(&asset.exe);
        let mut runtime_sha = asset.source.sha256.clone();
        if exe.is_file() {
            // 展開済み(前回の導入)。そのまま使う
        } else {
            self.progress(install_id, 1, "runtime", 0, asset.source.size_bytes);
            let archive_name = asset
                .source
                .url
                .rsplit('/')
                .next()
                .unwrap_or("runtime.archive")
                .to_string();
            let final_path = layout.downloads_dir().join(&archive_name);
            let part = layout.downloads_dir().join(format!("{archive_name}.part"));
            self.fetch(
                install_id,
                1,
                "runtime",
                &client,
                &policy,
                &Request {
                    url: &asset.source.url,
                    part_path: &part,
                    final_path: &final_path,
                    size_bytes: asset.source.size_bytes,
                    sha256: &asset.source.sha256,
                },
                &mut cancel,
            )
            .await?;

            // 2. 展開(一時ディレクトリへ展開してから置き換える)
            self.progress(install_id, 2, "extract", 0, 0);
            let tmp = layout.root.join("bin").join(format!("{build}.tmp"));
            let dest = layout.bin_dir(&build);
            let (kind, archive_path, tmp2, dest2) =
                (asset.kind, final_path.clone(), tmp.clone(), dest.clone());
            let flag2 = Arc::new(AtomicBool::new(false));
            let flag_for_task = Arc::clone(&flag2);
            let extract = tokio::task::spawn_blocking(move || -> AppResult<()> {
                let _ = std::fs::remove_dir_all(&tmp2);
                match kind {
                    super::manifest::ArchiveKind::Zip => {
                        archive::extract_zip(&archive_path, &tmp2, 0, &flag_for_task)?
                    }
                    super::manifest::ArchiveKind::TarGz => {
                        archive::extract_tar_gz(&archive_path, &tmp2, &flag_for_task)?
                    }
                }
                let _ = std::fs::remove_dir_all(&dest2);
                std::fs::rename(&tmp2, &dest2).map_err(io_error)
            });
            tokio::pin!(extract);
            let r = loop {
                tokio::select! {
                    r = &mut extract => break r.map_err(|e| AppError::internal(e.to_string()))?,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                        if flag.load(Ordering::Relaxed) {
                            flag2.store(true, Ordering::Relaxed);
                        }
                    }
                }
            };
            if let Err(e) = r {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(if flag.load(Ordering::Relaxed) {
                    cancelled_error()
                } else {
                    e
                });
            }
            let _ = std::fs::remove_file(&final_path);
            if !exe.is_file() {
                return Err(failed(
                    "LLM_INSTALL_FAILED",
                    "展開したランタイムに llama-server が見つかりません",
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // 展開で実行権限が付かなかった場合に備える
                if let Ok(meta) = std::fs::metadata(&exe) {
                    let mut perm = meta.permissions();
                    perm.set_mode(perm.mode() | 0o755);
                    let _ = std::fs::set_permissions(&exe, perm);
                }
            }
        }
        if flag.load(Ordering::Relaxed) {
            return Err(cancelled_error());
        }
        runtime_sha = runtime_sha.to_ascii_lowercase();

        // 3. モデル
        let final_path = layout.models_dir().join(&model.file_name);
        let mut have_model = false;
        if final_path.is_file() {
            self.progress(install_id, 3, "model", 0, model.source.size_bytes);
            // 前回の導入で配置済み: 内容が固定値と一致すれば取り直さない
            if sha256_file(&final_path).await? == model.source.sha256 {
                have_model = true;
            } else {
                let _ = std::fs::remove_file(&final_path);
            }
        }
        if !have_model {
            let part = layout
                .downloads_dir()
                .join(format!("{}.part", model.file_name));
            self.progress(install_id, 3, "model", 0, model.source.size_bytes);
            self.fetch(
                install_id,
                3,
                "model",
                &client,
                &policy,
                &Request {
                    url: &model.source.url,
                    part_path: &part,
                    final_path: &final_path,
                    size_bytes: model.source.size_bytes,
                    sha256: &model.source.sha256,
                },
                &mut cancel,
            )
            .await?;
        }

        // 付属ファイル(ライセンス全文。docs/34 §9.13)。検証できたものだけ置く
        for extra in &model.extra_files {
            let path = layout.models_dir().join(&extra.file_name);
            if path.is_file() && sha256_file(&path).await? == extra.source.sha256 {
                continue;
            }
            let part = layout
                .downloads_dir()
                .join(format!("{}.part", extra.file_name));
            self.fetch(
                install_id,
                3,
                "license",
                &client,
                &policy,
                &Request {
                    url: &extra.source.url,
                    part_path: &part,
                    final_path: &path,
                    size_bytes: extra.source.size_bytes,
                    sha256: &extra.source.sha256,
                },
                &mut cancel,
            )
            .await?;
        }

        // 記録(検証まで終わったときだけ)。古いビルドは消す
        let record = InstallRecord {
            manifest_version: self.manifest.manifest_version.clone(),
            runtime_build: build.clone(),
            runtime_sha256: runtime_sha,
            model_file: model.file_name.clone(),
            model_sha256: model.source.sha256.clone(),
        };
        let json = serde_json::to_string_pretty(&record)?;
        std::fs::write(layout.marker(), json).map_err(io_error)?;
        if let Ok(rd) = std::fs::read_dir(layout.root.join("bin")) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy() != build {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        }
        if let Ok(rd) = std::fs::read_dir(layout.models_dir()) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name != model.file_name && !model.extra_files.iter().any(|f| f.file_name == name)
                {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        Ok(())
    }

    fn progress(&self, install_id: &str, step: u32, step_id: &str, received: u64, total: u64) {
        let mut st = InstallState::idle();
        st.install_id = Some(install_id.to_string());
        st.status = InstallStatus::Running;
        st.step = step;
        st.step_id = Some(step_id.to_string());
        st.received = Some(received);
        st.total = Some(total);
        self.set_install(st);
    }

    #[allow(clippy::too_many_arguments)]
    async fn fetch(
        &self,
        install_id: &str,
        step: u32,
        step_id: &str,
        client: &reqwest::Client,
        policy: &DownloadPolicy,
        req: &Request<'_>,
        cancel: &mut watch::Receiver<CancelMode>,
    ) -> AppResult<()> {
        let total = req.size_bytes;
        let mut on_progress = |p: Progress| {
            // 検証中は「全部受信済み」として表示する
            let received = match p.phase {
                Phase::Verifying => total,
                Phase::Downloading => p.received,
            };
            self.progress(install_id, step, step_id, received, total);
        };
        match download::download(client, policy, req, cancel, &mut on_progress)
            .await
            .map_err(map_download_error)?
        {
            Outcome::Done => Ok(()),
            Outcome::Cancelled(_) => Err(cancelled_error()),
        }
    }
}
