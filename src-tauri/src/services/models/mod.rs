//! モデル取得サービス(docs/12-model-downloader.md)。
//!
//! 同梱カタログのモデルを、ユーザーの明示操作ごとに ComfyUI の `models/` へ取得する。
//! 通信はダウンロード開始後に Rust からのみ行い、許可リストのホストに限る。

pub mod catalog;
pub mod compat;
pub mod download;
pub mod engine_paths;
#[cfg(test)]
pub(crate) mod testserver;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::{Notify, watch};

use self::catalog::{Catalog, Variant};
use self::download::{CancelMode, DownloadPolicy, Outcome, Phase, Progress, Request};
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::services::engine::EngineService;
use crate::services::settings;

/// 完了・失敗した履歴を保持する件数の上限。
const MAX_FINISHED: usize = 50;
/// 空き容量の確認で、ファイルサイズに上乗せする余裕(1.1 倍)。
const SPACE_MARGIN_NUM: u64 = 11;
const SPACE_MARGIN_DEN: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DirSource {
    /// ユーザーが設定で指定した。
    Setting,
    /// ComfyUI の `/system_stats`(起動引数)から検出した。
    Detected,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledStatus {
    pub family_id: String,
    pub variant_id: String,
    pub installed: bool,
    /// ファイルサイズがカタログと一致するか(一致しなければ破損・別物の可能性)。
    pub size_matches: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogView {
    pub catalog: Catalog,
    pub installed: Vec<InstalledStatus>,
    pub models_dir: Option<String>,
    pub models_dir_source: DirSource,
    pub free_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirView {
    pub models_dir: Option<String>,
    pub source: DirSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Queued,
    Downloading,
    Verifying,
    Done,
    Failed,
    Cancelled,
}

impl Status {
    fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Downloading | Self::Verifying)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadState {
    pub download_id: String,
    pub family_id: String,
    pub variant_id: String,
    pub status: Status,
    pub received_bytes: u64,
    pub total_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_per_sec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

/// フロントエンドへの通知(`models://progress` / `models://done`)。テストで差し替える。
pub trait ModelSink: Send + Sync {
    fn progress(&self, state: &DownloadState);
    fn done(&self, state: &DownloadState);
}

struct Entry {
    state: DownloadState,
    cancel: watch::Sender<CancelMode>,
    url: String,
    sha256: String,
    final_path: PathBuf,
}

/// ワーカーが 1 件を実行するための情報。
struct Job {
    id: String,
    url: String,
    sha256: String,
    size: u64,
    final_path: PathBuf,
    part_path: PathBuf,
    cancel: watch::Receiver<CancelMode>,
}

pub struct ModelService {
    db: Db,
    engine: Arc<EngineService>,
    catalog: Arc<Catalog>,
    policy: DownloadPolicy,
    http: reqwest::Client,
    sink: Arc<dyn ModelSink>,
    entries: Mutex<Vec<Entry>>,
    wake: Notify,
}

fn dir_unknown(message: impl Into<String>) -> AppError {
    AppError::models("MODELS_DIR_UNKNOWN", message)
}

fn not_found() -> AppError {
    AppError::models("MODELS_NOT_FOUND", "カタログにないモデルです")
}

fn part_path_of(final_path: &Path) -> PathBuf {
    let mut name = final_path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    final_path.with_file_name(name)
}

/// ComfyUI の起動引数から `models` フォルダを求める(docs/12 §3)。
/// `--base-directory <dir>` があれば `<dir>/models`、なければ `main.py` と同じ場所の `models`。
/// 絶対パスにならないもの(相対パス指定)は特定できないものとして `None`。
pub fn models_dir_from_argv(argv: &[String]) -> Option<PathBuf> {
    let mut base: Option<String> = None;
    let mut it = argv.iter().skip(1);
    while let Some(arg) = it.next() {
        if arg == "--base-directory" {
            base = it.next().cloned();
        } else if let Some(v) = arg.strip_prefix("--base-directory=") {
            base = Some(v.to_string());
        }
    }
    let dir = match base {
        Some(b) => PathBuf::from(b).join("models"),
        None => Path::new(argv.first()?).parent()?.join("models"),
    };
    dir.is_absolute().then_some(dir)
}

/// 反映できない理由(08 §6.2)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyBlock {
    /// 接続先がこの PC の ComfyUI ではない。
    Remote,
    /// 起動引数から `main.py` の場所を特定できない。
    EngineUnknown,
    /// 保存先が設定されていない(自動検出のとき)。
    NoSetting,
    /// ユーザー自身の `extra_model_paths.yaml` が既にある。
    Foreign,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineDirStatus {
    /// ComfyUI の起動引数から求めた `models`(特定できなければ null)。
    pub engine_models_dir: Option<String>,
    /// アプリが書いた設定ファイルが、現在の保存先と一致している。
    pub applied: bool,
    /// 保存先が ComfyUI の参照先と異なり、未反映(反映を促す)。
    pub mismatch: bool,
    /// 起動引数に `--extra-model-paths-config` がある(アプリからは断定できない)。
    pub has_custom_config: bool,
    pub can_apply: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<ApplyBlock>,
}

/// 保存先設定・起動引数・ローカルかどうかから、反映状況を求める(純粋関数)。
pub fn engine_dir_status_of(
    setting: Option<&Path>,
    argv: Option<&[String]>,
    local: bool,
) -> EngineDirStatus {
    let engine_dir = argv.and_then(models_dir_from_argv);
    let file = argv.and_then(engine_paths::file_for_argv);
    let has_custom_config = argv.is_some_and(|a| {
        a.iter().any(|x| {
            x == "--extra-model-paths-config" || x.starts_with("--extra-model-paths-config=")
        })
    });
    let applied = matches!((setting, &file), (Some(s), Some(f)) if engine_paths::is_applied(f, s));
    let mismatch = !applied
        && !has_custom_config
        && matches!((setting, &engine_dir), (Some(s), Some(e)) if !engine_paths::same_dir(s, e));
    let reason = if !local {
        Some(ApplyBlock::Remote)
    } else if file.is_none() {
        Some(ApplyBlock::EngineUnknown)
    } else if setting.is_none() {
        Some(ApplyBlock::NoSetting)
    } else if file
        .as_deref()
        .is_some_and(|f| matches!(engine_paths::is_managed(f), Ok(Some(false))))
    {
        Some(ApplyBlock::Foreign)
    } else {
        None
    };
    EngineDirStatus {
        engine_models_dir: engine_dir.map(|d| d.to_string_lossy().into_owned()),
        applied,
        mismatch,
        has_custom_config,
        can_apply: reason.is_none(),
        reason,
    }
}

/// 接続先がこの PC の ComfyUI か。リモートの ComfyUI のフォルダには書けない。
pub fn is_local_engine(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|h| matches!(h.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "::1"))
}

impl ModelService {
    pub fn new(
        db: Db,
        engine: Arc<EngineService>,
        catalog: Arc<Catalog>,
        policy: DownloadPolicy,
        sink: Arc<dyn ModelSink>,
    ) -> AppResult<Arc<Self>> {
        let http = download::build_client(&policy)?;
        Ok(Arc::new(Self {
            db,
            engine,
            catalog,
            policy,
            http,
            sink,
            entries: Mutex::new(Vec::new()),
            wake: Notify::new(),
        }))
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, Vec<Entry>>> {
        self.entries
            .lock()
            .map_err(|_| AppError::internal("models state lock poisoned"))
    }

    /// `models` フォルダと、その特定方法(docs/12 §3)。
    pub async fn resolve_dir(&self) -> AppResult<(Option<PathBuf>, DirSource)> {
        if let Some(p) = self.db.call(|c| settings::get_models_dir(c)).await? {
            return Ok((Some(PathBuf::from(p)), DirSource::Setting));
        }
        let engine = self.engine.engine()?;
        if is_local_engine(&engine.url())
            && let Ok(argv) = engine.launch_args().await
            && let Some(dir) = models_dir_from_argv(&argv)
            && dir.is_dir()
        {
            return Ok((Some(dir), DirSource::Detected));
        }
        Ok((None, DirSource::Unknown))
    }

    fn target_path(dir: &Path, v: &Variant) -> PathBuf {
        let mut p = dir.to_path_buf();
        for c in v.dest_dir.components() {
            p.push(c);
        }
        p.push(&v.file_name);
        p
    }

    /// カタログ・導入状況・保存先。外部へは通信しない(ComfyUI へ起動引数を尋ねることはある)。
    pub async fn catalog_view(&self) -> AppResult<CatalogView> {
        let (dir, source) = self.resolve_dir().await?;
        let mut installed = Vec::new();
        for f in &self.catalog.families {
            for v in &f.variants {
                let meta = dir
                    .as_deref()
                    .and_then(|d| std::fs::metadata(Self::target_path(d, v)).ok())
                    .filter(|m| m.is_file());
                installed.push(InstalledStatus {
                    family_id: f.id.clone(),
                    variant_id: v.id.clone(),
                    installed: meta.is_some(),
                    size_matches: meta.is_some_and(|m| m.len() == v.size_bytes),
                });
            }
        }
        let free_bytes = dir
            .as_deref()
            .filter(|d| d.is_dir())
            .and_then(|d| fs4::available_space(d).ok());
        Ok(CatalogView {
            catalog: (*self.catalog).clone(),
            installed,
            models_dir: dir.map(|d| d.to_string_lossy().into_owned()),
            models_dir_source: source,
            free_bytes,
        })
    }

    /// モデルフォルダを設定する(`None` で自動検出に戻す)。存在と書き込みを確認する。
    pub async fn set_dir(&self, path: Option<String>) -> AppResult<DirView> {
        if let Some(p) = &path {
            let dir = Path::new(p);
            if !dir.is_absolute() {
                return Err(AppError::validation("path", "絶対パスで指定してください"));
            }
            if !dir.is_dir() {
                return Err(AppError::validation("path", "フォルダが見つかりません"));
            }
            let probe = dir.join(format!(".gazooo-write-test-{}", uuid::Uuid::now_v7()));
            std::fs::write(&probe, b"")
                .and_then(|()| std::fs::remove_file(&probe))
                .map_err(|_| AppError::validation("path", "このフォルダには書き込めません"))?;
        }
        self.db
            .call(move |c| settings::set_models_dir(c, path.as_deref()))
            .await?;
        let (dir, source) = self.resolve_dir().await?;
        Ok(DirView {
            models_dir: dir.map(|d| d.to_string_lossy().into_owned()),
            source,
        })
    }

    /// ローカルの ComfyUI の起動引数(リモート・取得失敗は `None`)と、ローカルかどうか。
    async fn local_argv(&self) -> AppResult<(bool, Option<Vec<String>>)> {
        let engine = self.engine.engine()?;
        let local = is_local_engine(&engine.url());
        let argv = if local {
            engine.launch_args().await.ok().filter(|a| !a.is_empty())
        } else {
            None
        };
        Ok((local, argv))
    }

    /// ComfyUI が参照するフォルダと、保存先の反映状況(docs/08 §6.2)。
    pub async fn engine_dir_status(&self) -> AppResult<EngineDirStatus> {
        let setting = self
            .db
            .call(|c| settings::get_models_dir(c))
            .await?
            .map(PathBuf::from);
        let (local, argv) = self.local_argv().await?;
        Ok(engine_dir_status_of(
            setting.as_deref(),
            argv.as_deref(),
            local,
        ))
    }

    /// 保存先を ComfyUI に反映する(`clear` で解除)。ComfyUI の再起動は別途必要。
    pub async fn apply_dir_to_engine(&self, clear: bool) -> AppResult<EngineDirStatus> {
        let (local, argv) = self.local_argv().await?;
        let file = argv
            .as_deref()
            .and_then(engine_paths::file_for_argv)
            .filter(|_| local)
            .ok_or_else(|| dir_unknown("ComfyUI の場所を特定できないため、反映できません"))?;
        if clear {
            engine_paths::clear(&file)?;
        } else {
            let dir = self
                .db
                .call(|c| settings::get_models_dir(c))
                .await?
                .ok_or_else(|| dir_unknown("保存先が設定されていません"))?;
            engine_paths::apply(&file, Path::new(&dir))?;
        }
        self.engine_dir_status().await
    }

    /// ダウンロードをキューに追加する。同じモデルが取得中・待機中なら、その ID を返す。
    pub async fn start(
        &self,
        family_id: &str,
        variant_id: &str,
        consent: bool,
    ) -> AppResult<String> {
        let (_, variant) = self
            .catalog
            .find(family_id, variant_id)
            .ok_or_else(not_found)?;
        if let Some(e) = self.lock()?.iter().find(|e| {
            e.state.status.is_active()
                && e.state.family_id == family_id
                && e.state.variant_id == variant_id
        }) {
            return Ok(e.state.download_id.clone());
        }

        let (dir, _) = self.resolve_dir().await?;
        let dir = dir.ok_or_else(|| {
            dir_unknown(
                "ComfyUI の models フォルダを特定できません。設定でフォルダを指定してください",
            )
        })?;
        let final_path = Self::target_path(&dir, variant);
        if final_path.exists() {
            return Err(AppError::models(
                "MODELS_ALREADY_INSTALLED",
                "このモデルはすでに導入されています",
            ));
        }

        if variant.requires_consent {
            let key = format!("{family_id}/{variant_id}");
            let consented = {
                let key = key.clone();
                self.db
                    .call(move |c| Ok(settings::get_model_consents(c)?.contains(&key)))
                    .await?
            };
            if !consented {
                if !consent {
                    return Err(AppError::models(
                        "MODELS_NO_CONSENT",
                        "ライセンスの確認が必要です",
                    ));
                }
                self.db
                    .call(move |c| settings::add_model_consent(c, &key))
                    .await?;
            }
        }

        let dest_dir = final_path
            .parent()
            .ok_or_else(|| dir_unknown("保存先を決められません"))?
            .to_path_buf();
        std::fs::create_dir_all(&dest_dir)
            .map_err(|e| dir_unknown(format!("保存先フォルダを作成できません: {e}")))?;

        let part_len = std::fs::metadata(part_path_of(&final_path))
            .map(|m| m.len())
            .unwrap_or(0);
        let need =
            variant.size_bytes.saturating_sub(part_len) / SPACE_MARGIN_DEN * SPACE_MARGIN_NUM;
        if let Ok(free) = fs4::available_space(&dest_dir)
            && free < need
        {
            return Err(AppError::models_with(
                "MODELS_DISK_FULL",
                "ディスクの空き容量が足りません",
                serde_json::json!({ "needBytes": need, "freeBytes": free }),
            ));
        }

        let download_id = uuid::Uuid::now_v7().to_string();
        let (cancel, _) = watch::channel(CancelMode::Run);
        let state = DownloadState {
            download_id: download_id.clone(),
            family_id: family_id.to_string(),
            variant_id: variant_id.to_string(),
            status: Status::Queued,
            received_bytes: 0,
            total_bytes: variant.size_bytes,
            bytes_per_sec: None,
            error: None,
        };
        {
            let mut entries = self.lock()?;
            // 同じモデルの過去の結果(失敗・取消)は、再取得で置き換える
            entries.retain(|e| {
                e.state.status.is_active()
                    || !(e.state.family_id == family_id && e.state.variant_id == variant_id)
            });
            let finished = entries
                .iter()
                .filter(|e| !e.state.status.is_active())
                .count();
            for _ in 0..finished.saturating_sub(MAX_FINISHED - 1) {
                if let Some(i) = entries.iter().position(|e| !e.state.status.is_active()) {
                    entries.remove(i);
                }
            }
            entries.push(Entry {
                state: state.clone(),
                cancel,
                url: variant.source.url.clone(),
                sha256: variant.sha256.clone(),
                final_path,
            });
        }
        self.sink.progress(&state);
        self.wake.notify_one();
        Ok(download_id)
    }

    pub fn list(&self) -> AppResult<Vec<DownloadState>> {
        Ok(self.lock()?.iter().map(|e| e.state.clone()).collect())
    }

    /// 取得を中止する。`discard` で `.part`(途中のデータ)も削除する。
    pub fn cancel(&self, download_id: &str, discard: bool) -> AppResult<()> {
        let mut finished = None;
        {
            let mut entries = self.lock()?;
            let e = entries
                .iter_mut()
                .find(|e| e.state.download_id == download_id)
                .ok_or_else(not_found)?;
            match e.state.status {
                Status::Queued => {
                    e.state.status = Status::Cancelled;
                    if discard {
                        let _ = std::fs::remove_file(part_path_of(&e.final_path));
                    }
                    finished = Some(e.state.clone());
                }
                Status::Downloading | Status::Verifying => {
                    let _ = e.cancel.send(if discard {
                        CancelMode::Discard
                    } else {
                        CancelMode::Cancel
                    });
                }
                _ => {}
            }
        }
        if let Some(s) = finished {
            self.sink.done(&s);
        }
        Ok(())
    }

    /// 導入済みファイルの SHA-256 をカタログと照合する(通信しない)。
    pub async fn verify(&self, family_id: &str, variant_id: &str) -> AppResult<bool> {
        let (_, variant) = self
            .catalog
            .find(family_id, variant_id)
            .ok_or_else(not_found)?;
        let (dir, _) = self.resolve_dir().await?;
        let dir = dir.ok_or_else(|| dir_unknown("ComfyUI の models フォルダを特定できません"))?;
        let path = Self::target_path(&dir, variant);
        if !path.is_file() {
            return Err(not_found_installed());
        }
        Ok(download::sha256_file(&path).await? == variant.sha256)
    }

    /// 待機中のダウンロードを 1 件ずつ実行するワーカー(アプリ起動時に 1 つだけ動かす)。
    pub async fn run_worker(self: Arc<Self>) {
        loop {
            match self.take_next() {
                Ok(Some(job)) => self.run_one(job).await,
                Ok(None) => self.wake.notified().await,
                Err(e) => {
                    tracing::error!(error = %e, "model download worker stopped");
                    return;
                }
            }
        }
    }

    fn take_next(&self) -> AppResult<Option<Job>> {
        let mut entries = self.lock()?;
        let Some(e) = entries
            .iter_mut()
            .find(|e| e.state.status == Status::Queued)
        else {
            return Ok(None);
        };
        e.state.status = Status::Downloading;
        let job = Job {
            id: e.state.download_id.clone(),
            url: e.url.clone(),
            sha256: e.sha256.clone(),
            size: e.state.total_bytes,
            part_path: part_path_of(&e.final_path),
            final_path: e.final_path.clone(),
            cancel: e.cancel.subscribe(),
        };
        let state = e.state.clone();
        drop(entries);
        self.sink.progress(&state);
        Ok(Some(job))
    }

    /// 状態を更新して、更新後の状態を返す。
    fn update(&self, id: &str, f: impl FnOnce(&mut DownloadState)) -> Option<DownloadState> {
        let mut entries = self.entries.lock().ok()?;
        let e = entries.iter_mut().find(|e| e.state.download_id == id)?;
        f(&mut e.state);
        Some(e.state.clone())
    }

    async fn run_one(&self, mut job: Job) {
        tracing::info!(download = %job.id, "model download started");
        let req = Request {
            url: &job.url,
            part_path: &job.part_path,
            final_path: &job.final_path,
            size_bytes: job.size,
            sha256: &job.sha256,
        };
        let id = job.id.clone();
        let mut on_progress = |p: Progress| {
            let state = self.update(&id, |s| {
                s.status = match p.phase {
                    Phase::Downloading => Status::Downloading,
                    Phase::Verifying => Status::Verifying,
                };
                s.received_bytes = p.received;
                s.bytes_per_sec = p.bytes_per_sec;
            });
            if let Some(s) = state {
                self.sink.progress(&s);
            }
        };
        let result = download::download(
            &self.http,
            &self.policy,
            &req,
            &mut job.cancel,
            &mut on_progress,
        )
        .await;

        let finished = match result {
            Ok(Outcome::Done) => {
                tracing::info!(download = %job.id, "model download finished");
                self.update(&job.id, |s| {
                    s.status = Status::Done;
                    s.received_bytes = s.total_bytes;
                    s.bytes_per_sec = None;
                })
            }
            Ok(Outcome::Cancelled(mode)) => {
                if mode == CancelMode::Discard {
                    let _ = tokio::fs::remove_file(&job.part_path).await;
                }
                self.update(&job.id, |s| {
                    s.status = Status::Cancelled;
                    s.bytes_per_sec = None;
                })
            }
            Err(e) => {
                tracing::warn!(download = %job.id, code = e.code(), "model download failed");
                self.update(&job.id, |s| {
                    s.status = Status::Failed;
                    s.bytes_per_sec = None;
                    s.error = Some(ErrorInfo {
                        code: e.code().to_string(),
                        message: e.to_string(),
                    });
                })
            }
        };
        if let Some(s) = finished {
            self.sink.done(&s);
        }
    }
}

fn not_found_installed() -> AppError {
    AppError::models("MODELS_NOT_FOUND", "このモデルは導入されていません")
}

#[cfg(test)]
mod tests;
