//! 保存先のデータ移行(docs/51-storage-migration.md)。
//!
//! 事前確認 → コピー → 保存先の切り替え → 元のファイルを OS のゴミ箱へ、の順で行う。
//! 切り替えより前の失敗・中止では、保存先の設定と元のファイルは変わらない。

pub mod files;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use self::files::{Analysis, Relation, Scan};
use crate::AppPaths;
use crate::db::Db;
use crate::db::jobs::ImageRow;
use crate::db::scopes::{self, Scope};
use crate::domain::params::is_valid_input_path;
use crate::error::{AppError, AppResult};
use crate::services::image_locations::{self as locations, Rejected};
use crate::services::images::Trasher;
use crate::services::models::engine_paths;
use crate::services::settings::{self, SettingsPatch};

/// コピー中の進捗イベントの最小間隔(最大 4 回/秒)。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// 画面に列挙する競合の上限。
const MAX_CONFLICTS: usize = 20;
/// 空き容量の確認で上乗せする余裕(1.1 倍)。
const SPACE_MARGIN_PERCENT: u64 = 110;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Target {
    /// 全体の画像の保存先(全体設定を使う全ユーザー。docs/52 §4.2)。
    ImagesGlobal,
    /// ログイン中のユーザーの画像の保存先(個別設定。docs/52 §4.3)。
    ImagesUser,
    Models,
}

/// 開始できない理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Blocker {
    SourceUnknown,
    SourceMissing,
    DestNotAbsolute,
    DestNotWritable,
    DestInAppData,
    /// ほかのユーザーの画像の場所・全体の保存先と重なる。
    DestInUse,
    /// 対象のユーザーのデータを読めない。
    UserUnreadable,
    SameDir,
    Nested,
    NoSpace,
    Conflict,
    JobRunning,
    DownloadActive,
    Busy,
}

impl Blocker {
    fn error(self) -> AppError {
        let m = AppError::migration;
        match self {
            Self::NoSpace => m("STORAGE_NO_SPACE", "移行先の空き容量が足りません"),
            Self::Conflict => m(
                "STORAGE_CONFLICT",
                "移行先に内容の違う同名のファイルがあります",
            ),
            Self::JobRunning => m("STORAGE_BUSY", "生成中は移行できません"),
            Self::DownloadActive => m("STORAGE_BUSY", "モデルのダウンロード中は移行できません"),
            Self::Busy => m("STORAGE_BUSY", "別の処理を実行中です"),
            Self::SourceUnknown => AppError::validation("newDir", "移行元を特定できません"),
            Self::SourceMissing => {
                AppError::validation("newDir", "移行元のフォルダが見つかりません")
            }
            Self::DestNotAbsolute => {
                AppError::validation("newDir", "移行先は絶対パスで指定してください")
            }
            Self::DestNotWritable => AppError::validation("newDir", "この場所には書き込めません"),
            Self::DestInAppData => {
                AppError::validation("newDir", "アプリのデータ領域内は指定できません")
            }
            Self::DestInUse => Rejected::InUse.error(),
            Self::UserUnreadable => {
                AppError::validation("newDir", "ユーザーのデータを読めないため、移行できません")
            }
            Self::SameDir => AppError::validation("newDir", "現在の保存先と同じフォルダです"),
            Self::Nested => {
                AppError::validation("newDir", "移行先が現在の保存先と入れ子になっています")
            }
        }
    }
}

/// ComfyUI の参照先を切り替えられない理由(docs/51 §6)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineBlock {
    Remote,
    EngineUnknown,
    Foreign,
    CustomConfig,
}

#[derive(Debug, Clone, Default)]
pub struct EngineProbe {
    /// アプリが起動した ComfyUI が動いている。
    pub managed_running: bool,
    /// アプリの外で起動した ComfyUI が応答している。
    pub external_running: bool,
    /// 書き込む `extra_model_paths.yaml`(特定できないとき `None`)。
    pub config_file: Option<PathBuf>,
    pub block: Option<EngineBlock>,
}

/// 起動引数から、反映先と反映できない理由を求める。
pub fn engine_target(
    argv: Option<&[String]>,
    local: bool,
) -> (Option<PathBuf>, Option<EngineBlock>) {
    if !local {
        return (None, Some(EngineBlock::Remote));
    }
    let Some(argv) = argv else {
        return (None, Some(EngineBlock::EngineUnknown));
    };
    let custom = argv
        .iter()
        .any(|a| a == "--extra-model-paths-config" || a.starts_with("--extra-model-paths-config="));
    match engine_paths::file_for_argv(argv) {
        None => (None, Some(EngineBlock::EngineUnknown)),
        Some(_) if custom => (None, Some(EngineBlock::CustomConfig)),
        Some(f) if matches!(engine_paths::is_managed(&f), Ok(Some(false))) => {
            (None, Some(EngineBlock::Foreign))
        }
        Some(f) => (Some(f), None),
    }
}

/// 移行が、他のサービス(ジョブ・ダウンロード・ComfyUI)に頼む操作。テストで差し替える。
#[async_trait::async_trait]
pub trait MigrationHost: Send + Sync {
    /// 次のジョブの開始を止める / 再開する。止めるときは、開始処理が済むまで待つ。
    async fn hold_jobs(&self, hold: bool);
    /// ほかに実行中で、移行と同時にできない処理。
    fn busy(&self, target: Target) -> Vec<Blocker>;
    async fn models_source(&self) -> AppResult<Option<PathBuf>>;
    async fn engine_probe(&self) -> AppResult<EngineProbe>;
    async fn stop_engine(&self) -> AppResult<()>;
    async fn start_engine(&self) -> AppResult<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Copying,
    Switching,
    Cleaning,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationProgress {
    pub phase: Phase,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub done_files: usize,
    pub total_files: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_file: Option<String>,
}

pub trait MigrationSink: Send + Sync {
    fn progress(&self, p: &MigrationProgress);
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnginePlan {
    pub managed_running: bool,
    pub external_running: bool,
    pub can_apply: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<EngineBlock>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserPlan {
    pub name: String,
    pub file_count: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPlan {
    /// 空文字は「アプリのデータ内」(全体の移行の標準値)。
    pub source_dir: String,
    pub dest_dir: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub copy_bytes: u64,
    pub free_bytes: Option<u64>,
    pub same_size_count: usize,
    pub missing: usize,
    pub conflicts: Vec<String>,
    pub skipped_folders: Vec<String>,
    pub skipped_links: usize,
    pub blockers: Vec<Blocker>,
    /// 全体の移行の対象ユーザー。
    pub users: Vec<UserPlan>,
    /// 個別設定のため、全体の移行の対象外のユーザー数。
    pub excluded_users: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<EnginePlan>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    pub source_dir: String,
    pub dest_dir: String,
    pub moved: usize,
    pub missing: usize,
    pub kept_same_size: usize,
    pub left_in_source: usize,
    /// ファイルを残した元のフォルダ。
    pub left_dirs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_applied: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_restarted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_error: Option<String>,
}

/// 移行の 1 まとまり(画像はユーザーごと、モデルは全体で 1 つ)。
struct Part {
    /// 全体の移行のユーザー名(ほかは空)。
    name: String,
    source: PathBuf,
    dest: PathBuf,
    scan: Scan,
    analysis: Analysis,
    /// 移行元をほかのユーザーも使っている(参照画像は元を残す)。
    shared: bool,
}

/// これから走査する 1 まとまり。
struct Candidate {
    name: String,
    source: PathBuf,
    dest: PathBuf,
    /// 画像の記録。`None` はモデル。
    rows: Option<Vec<ImageRow>>,
    shared: bool,
}

/// 事前確認の結果。
struct Prepared {
    /// 画面に出す移行元(全体の移行で標準値なら空)。
    source: String,
    dest: PathBuf,
    /// 保存する設定値(`None` は全体設定・標準値・既定に戻す)。
    setting: Option<String>,
    dest_label: String,
    parts: Vec<Part>,
    excluded_users: usize,
    free: Option<u64>,
    blockers: Vec<Blocker>,
    engine: Option<EngineProbe>,
}

impl Prepared {
    fn sum(&self, f: impl Fn(&Part) -> u64) -> u64 {
        self.parts.iter().map(f).sum()
    }

    fn plan(&self, target: Target) -> MigrationPlan {
        let count = |f: fn(&Part) -> usize| self.parts.iter().map(f).sum::<usize>();
        MigrationPlan {
            source_dir: self.source.clone(),
            dest_dir: self.dest_label.clone(),
            file_count: count(|p| p.scan.items.len()),
            total_bytes: self.sum(|p| p.scan.items.iter().map(|i| i.size).sum()),
            copy_bytes: self.sum(|p| p.analysis.copy_bytes),
            free_bytes: self.free,
            same_size_count: count(|p| p.analysis.same_size.len()),
            missing: count(|p| p.scan.missing),
            conflicts: self
                .parts
                .iter()
                .flat_map(|p| p.analysis.conflicts.iter().cloned())
                .take(MAX_CONFLICTS)
                .collect(),
            skipped_folders: self
                .parts
                .iter()
                .flat_map(|p| p.scan.skipped_folders.iter().cloned())
                .collect(),
            skipped_links: count(|p| p.scan.skipped_links),
            blockers: self.blockers.clone(),
            users: if target == Target::ImagesGlobal {
                self.parts
                    .iter()
                    .map(|p| UserPlan {
                        name: p.name.clone(),
                        file_count: p.scan.items.len(),
                        total_bytes: p.scan.items.iter().map(|i| i.size).sum(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
            excluded_users: self.excluded_users,
            engine: self.engine.as_ref().map(|e| EnginePlan {
                managed_running: e.managed_running,
                external_running: e.external_running,
                can_apply: e.block.is_none(),
                reason: e.block,
            }),
        }
    }
}

/// 候補を走査して、移行先の状況を調べる。
fn examine(candidates: Vec<Candidate>, found: &mut Vec<Blocker>) -> AppResult<Vec<Part>> {
    let mut parts = Vec::new();
    for c in candidates {
        let images = c.rows.is_some();
        // 画像の記録がないユーザーは、フォルダがなくてもよい
        let needs_source = !images || c.rows.as_ref().is_some_and(|r| !r.is_empty());
        let exists = c.source.is_dir();
        if !exists && needs_source {
            found.push(Blocker::SourceMissing);
        }
        let mut usable = c.dest.is_absolute();
        if usable {
            match files::relation(&c.source, &c.dest) {
                Relation::Same => {
                    found.push(Blocker::SameDir);
                    usable = false;
                }
                Relation::Nested => {
                    found.push(Blocker::Nested);
                    usable = false;
                }
                Relation::Distinct => {}
            }
        }
        let scan = match (&c.rows, exists) {
            (_, false) => Scan::default(),
            (None, true) => files::scan_models(&c.source)?,
            (Some(rows), true) => files::scan_list(&c.source, &image_rels(rows, &c.source)),
        };
        let analysis = if usable {
            files::analyze(&scan.items, &c.dest)
        } else {
            Analysis::default()
        };
        if !analysis.conflicts.is_empty() {
            found.push(Blocker::Conflict);
        }
        parts.push(Part {
            name: c.name,
            source: c.source,
            dest: c.dest,
            scan,
            analysis,
            shared: c.shared,
        });
    }
    Ok(parts)
}

fn display(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".gazooo-write-test-{}", uuid::Uuid::now_v7()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal(e.to_string()))
}

pub struct MigrationService {
    db: Db,
    paths: AppPaths,
    trasher: Arc<dyn Trasher>,
    host: Arc<dyn MigrationHost>,
    sink: Arc<dyn MigrationSink>,
    active: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

/// 実行中の印を、どの経路で抜けても外す。
struct ActiveGuard(Arc<AtomicBool>);
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl MigrationService {
    /// `active` は、移行中にほかの操作を断る側(IPC の入口・自動復旧)と共有する。
    pub fn new(
        db: Db,
        paths: AppPaths,
        trasher: Arc<dyn Trasher>,
        host: Arc<dyn MigrationHost>,
        sink: Arc<dyn MigrationSink>,
        active: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            paths,
            trasher,
            host,
            sink,
            active,
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// コピー中なら中止する(切り替え以降は止めない)。
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// 内容を確認する。何も変更しない。
    pub async fn plan(&self, target: Target, new_dir: Option<String>) -> AppResult<MigrationPlan> {
        if self.is_active() {
            return Err(Blocker::Busy.error());
        }
        Ok(self.prepare(target, new_dir).await?.plan(target))
    }

    async fn prepare(&self, target: Target, new_dir: Option<String>) -> AppResult<Prepared> {
        let mut blockers = self.host.busy(target);
        let running: i64 = self
            .db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM jobs WHERE status = 'running'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await?;
        if running > 0 {
            blockers.push(Blocker::JobRunning);
        }
        let absolute = new_dir
            .as_deref()
            .is_none_or(|d| Path::new(d).is_absolute());
        if !absolute {
            blockers.push(Blocker::DestNotAbsolute);
        }
        match target {
            Target::ImagesGlobal => self.prepare_global(new_dir, blockers, absolute).await,
            Target::ImagesUser => self.prepare_user(new_dir, blockers, absolute).await,
            Target::Models => self.prepare_models(new_dir, blockers).await,
        }
    }

    async fn prepare_global(
        &self,
        new_dir: Option<String>,
        mut blockers: Vec<Blocker>,
        absolute: bool,
    ) -> AppResult<Prepared> {
        let all = locations::all(&self.db, &self.paths).await?;
        let data_dir = self.paths.data_dir.clone();
        let setting = new_dir.clone();
        let (parts, excluded, free, mut found) = blocking(move || -> AppResult<_> {
            let mut found = Vec::new();
            let root = setting.as_deref().filter(|_| absolute);
            if let Some(r) = root {
                match locations::check_root(&data_dir, &all, Path::new(r)) {
                    Some(Rejected::InAppData) => found.push(Blocker::DestInAppData),
                    Some(Rejected::InUse) => found.push(Blocker::DestInUse),
                    None => {}
                }
                if let Some(old) = all.root.as_deref() {
                    match files::relation(Path::new(old), Path::new(r)) {
                        Relation::Same => found.push(Blocker::SameDir),
                        Relation::Nested => found.push(Blocker::Nested),
                        Relation::Distinct => {}
                    }
                }
            }
            let mut candidates = Vec::new();
            let mut excluded = 0;
            for u in &all.users {
                if !u.readable {
                    found.push(Blocker::UserUnreadable);
                    continue;
                }
                if u.individual.is_some() {
                    excluded += 1;
                    continue;
                }
                let dest =
                    locations::location(&data_dir, Some(&u.id), None, setting.as_deref(), None);
                // 場所が変わらないユーザーは、移すものがない
                if files::relation(&u.dir, &dest) == Relation::Same {
                    continue;
                }
                let Ok(rows) = locations::read_image_rows(&u.db_path) else {
                    found.push(Blocker::UserUnreadable);
                    continue;
                };
                let shared = all
                    .users
                    .iter()
                    .any(|o| o.id != u.id && locations::overlaps(&o.dir, &u.dir));
                candidates.push(Candidate {
                    name: u.name.clone(),
                    source: u.dir.clone(),
                    dest,
                    rows: Some(rows),
                    shared,
                });
            }
            let changes_root = all.root != setting;
            if candidates.is_empty() && !changes_root && found.is_empty() {
                found.push(Blocker::SameDir);
            }
            let base = setting
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| data_dir.clone());
            let free = check_destination(&base, absolute, &mut found);
            let parts = examine(candidates, &mut found)?;
            Ok((parts, excluded, free, found))
        })
        .await??;
        found.dedup();
        blockers.append(&mut found);
        let old_root = self
            .db
            .call_device(|c| Ok(settings::global_images(c)?.0))
            .await?;
        let dest = new_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.paths.data_dir.clone());
        Ok(finish(Prepared {
            source: old_root.unwrap_or_default(),
            dest,
            dest_label: new_dir.clone().unwrap_or_default(),
            setting: new_dir,
            parts,
            excluded_users: excluded,
            free,
            blockers,
            engine: None,
        }))
    }

    async fn prepare_user(
        &self,
        new_dir: Option<String>,
        mut blockers: Vec<Blocker>,
        absolute: bool,
    ) -> AppResult<Prepared> {
        let current = self.db.call(|c| settings::load(c)).await?;
        let source = self.paths.resolve_images_dir(&current);
        let dest = match &new_dir {
            Some(d) => PathBuf::from(d),
            None => self.paths.standard_images_dir(&current),
        };
        let all = locations::all(&self.db, &self.paths).await?;
        let rows = self.db.call(|c| crate::db::images::all(c)).await?;
        let me = self.paths.current_user_id();
        let data_dir = self.paths.data_dir.clone();
        let individual = new_dir.is_some();
        let (src, dst) = (source.clone(), dest.clone());
        let (parts, free, mut found) = blocking(move || -> AppResult<_> {
            let mut found = Vec::new();
            if individual && absolute {
                match locations::check_individual(&data_dir, &all, me.as_deref(), &dst) {
                    Some(Rejected::InAppData) => found.push(Blocker::DestInAppData),
                    Some(Rejected::InUse) => found.push(Blocker::DestInUse),
                    None => {}
                }
            }
            let shared = all
                .users
                .iter()
                .any(|o| Some(o.id.as_str()) != me.as_deref() && locations::overlaps(&o.dir, &src));
            let free = check_destination(&dst, absolute, &mut found);
            let parts = examine(
                vec![Candidate {
                    name: String::new(),
                    source: src,
                    dest: dst,
                    rows: Some(rows),
                    shared,
                }],
                &mut found,
            )?;
            Ok((parts, free, found))
        })
        .await??;
        blockers.append(&mut found);
        Ok(finish(Prepared {
            source: display(&source),
            dest_label: display(&dest),
            dest,
            setting: new_dir,
            parts,
            excluded_users: 0,
            free,
            blockers,
            engine: None,
        }))
    }

    async fn prepare_models(
        &self,
        new_dir: Option<String>,
        mut blockers: Vec<Blocker>,
    ) -> AppResult<Prepared> {
        let Some(dir) = new_dir else {
            return Err(AppError::validation("newDir", "移行先を指定してください"));
        };
        let dest = PathBuf::from(&dir);
        let source = self.host.models_source().await?;
        let engine = self.host.engine_probe().await?;
        if source.is_none() {
            blockers.push(Blocker::SourceUnknown);
        }
        let (src, dst) = (source.clone(), dest.clone());
        let (parts, free, mut found) = blocking(move || -> AppResult<_> {
            let mut found = Vec::new();
            let free = check_destination(&dst, dst.is_absolute(), &mut found);
            let candidates = src
                .map(|source| Candidate {
                    name: String::new(),
                    source,
                    dest: dst,
                    rows: None,
                    shared: false,
                })
                .into_iter()
                .collect();
            let parts = examine(candidates, &mut found)?;
            Ok((parts, free, found))
        })
        .await??;
        blockers.append(&mut found);
        Ok(finish(Prepared {
            source: source.as_deref().map(display).unwrap_or_default(),
            dest_label: dir.clone(),
            dest,
            setting: Some(dir),
            parts,
            excluded_users: 0,
            free,
            blockers,
            engine: Some(engine),
        }))
    }

    /// 移行を実行する。成功すると、移行先が以後の保存先になる。
    pub async fn start(
        &self,
        target: Target,
        new_dir: Option<String>,
    ) -> AppResult<MigrationReport> {
        if self.active.swap(true, Ordering::SeqCst) {
            return Err(Blocker::Busy.error());
        }
        let _guard = ActiveGuard(Arc::clone(&self.active));
        self.cancel.store(false, Ordering::SeqCst);
        // 事前確認より先にジョブを止める(確認のあとで始まるのを防ぐ)
        self.host.hold_jobs(true).await;
        let result = self.run(target, new_dir).await;
        self.host.hold_jobs(false).await;
        result
    }

    async fn run(&self, target: Target, new_dir: Option<String>) -> AppResult<MigrationReport> {
        let prepared = self.prepare(target, new_dir).await?;
        if let Some(b) = prepared.blockers.first() {
            return Err(b.error());
        }
        let stopped = prepared.engine.as_ref().is_some_and(|e| e.managed_running);
        if stopped {
            self.host.stop_engine().await?;
        }
        let result = self.transfer(target, &prepared).await;
        // 停止した ComfyUI は、中止・失敗でも起動し直す
        let restart = if stopped {
            Some(self.host.start_engine().await)
        } else {
            None
        };
        let mut report = result?;
        if let Some(r) = restart {
            report.engine_restarted = Some(r.is_ok());
            if let Err(e) = r {
                report.engine_error.get_or_insert(e.to_string());
            }
        }
        Ok(report)
    }

    async fn transfer(&self, target: Target, p: &Prepared) -> AppResult<MigrationReport> {
        let total_bytes = p.sum(|part| part.analysis.copy_bytes);
        let total_files: usize = p.parts.iter().map(|part| part.analysis.copy.len()).sum();

        let jobs: Vec<_> = p
            .parts
            .iter()
            .map(|part| {
                (
                    part.analysis.copy.clone(),
                    part.source.clone(),
                    part.dest.clone(),
                )
            })
            .collect();
        let base = p.dest.clone();
        let (cancel, sink) = (Arc::clone(&self.cancel), Arc::clone(&self.sink));
        // まとまりごとのコピー結果。失敗・中止では、それまでの分もすべて消す
        let copied = blocking(move || -> AppResult<Vec<files::Copied>> {
            std::fs::create_dir_all(&base)?;
            let mut done: Vec<files::Copied> = Vec::new();
            let (mut base_bytes, mut base_files) = (0u64, 0usize);
            let mut last: Option<Instant> = None;
            for (items, src, dst) in jobs {
                let result = std::fs::create_dir_all(&dst)
                    .map_err(AppError::from)
                    .and_then(|()| {
                        files::copy_all(&items, &src, &dst, &cancel, &mut |c| {
                            let done_files = base_files + c.done_files;
                            let due = last.is_none_or(|t| t.elapsed() >= PROGRESS_INTERVAL);
                            if due || done_files == total_files {
                                last = Some(Instant::now());
                                sink.progress(&MigrationProgress {
                                    phase: Phase::Copying,
                                    done_bytes: base_bytes + c.done_bytes,
                                    total_bytes,
                                    done_files,
                                    total_files,
                                    current_file: Some(c.current.to_owned()),
                                });
                            }
                        })
                    });
                match result {
                    Ok(copied) => done.push(copied),
                    Err(e) => {
                        done.iter().for_each(|c| files::remove_created(&c.created));
                        return Err(e);
                    }
                }
                base_bytes += items.iter().map(|i| i.size).sum::<u64>();
                base_files += items.len();
            }
            Ok(done)
        })
        .await??;

        let remove_all = |copied: &[files::Copied]| {
            copied
                .iter()
                .for_each(|c| files::remove_created(&c.created));
        };
        let progress = |phase, done_files, total_files| MigrationProgress {
            phase,
            done_bytes: total_bytes,
            total_bytes,
            done_files,
            total_files,
            current_file: None,
        };
        if self.cancel.load(Ordering::SeqCst) {
            remove_all(&copied);
            return Err(AppError::migration(
                "STORAGE_CANCELLED",
                "移行を中止しました",
            ));
        }
        self.sink
            .progress(&progress(Phase::Switching, total_files, total_files));
        if let Err(e) = self.switch(target, p.setting.clone()).await {
            remove_all(&copied);
            return Err(e);
        }

        let mut report = MigrationReport {
            source_dir: p.source.clone(),
            dest_dir: p.dest_label.clone(),
            moved: copied.iter().map(|c| c.sources.len()).sum(),
            missing: p.parts.iter().map(|part| part.scan.missing).sum(),
            kept_same_size: p
                .parts
                .iter()
                .map(|part| part.analysis.same_size.len())
                .sum(),
            ..MigrationReport::default()
        };
        // ComfyUI の参照先を切り替えられないときは、元のファイルを残す(docs/51 §6)
        let mut keep_source = false;
        if let Some(engine) = &p.engine {
            let applied = match (&engine.config_file, engine.block) {
                (Some(file), None) => match engine_paths::apply(file, &p.dest) {
                    Ok(()) => true,
                    Err(e) => {
                        report.engine_error = Some(e.to_string());
                        false
                    }
                },
                _ => false,
            };
            report.engine_applied = Some(applied);
            keep_source = !applied;
        }

        let mut sources = Vec::new();
        for (part, c) in p.parts.iter().zip(copied) {
            let same_size = part.analysis.same_size.len();
            let mut left = same_size;
            if keep_source {
                left += c.sources.len();
            } else {
                // ほかのユーザーも使うフォルダの参照画像は、ほかの人の分が混ざるので元を残す
                let inputs = part.source.join("inputs");
                let (kept, trash): (Vec<_>, Vec<_>) = c
                    .sources
                    .into_iter()
                    .partition(|s| part.shared && s.starts_with(&inputs));
                left += kept.len();
                sources.push((part.source.clone(), trash));
            }
            report.left_in_source += left;
            if left > 0 {
                report.left_dirs.push(display(&part.source));
            }
        }

        let count: usize = sources.iter().map(|(_, s)| s.len()).sum();
        let (trasher, sink) = (Arc::clone(&self.trasher), Arc::clone(&self.sink));
        let failed = blocking(move || {
            let mut failed = Vec::new();
            let mut offset = 0;
            for (root, files) in sources {
                let n = files::trash_all(&files, trasher.as_ref(), &mut |done| {
                    sink.progress(&MigrationProgress {
                        phase: Phase::Cleaning,
                        done_bytes: total_bytes,
                        total_bytes,
                        done_files: offset + done,
                        total_files: count,
                        current_file: None,
                    });
                });
                offset += files.len();
                if n > 0 {
                    failed.push((root, n));
                }
            }
            failed
        })
        .await?;
        for (root, n) in failed {
            report.left_in_source += n;
            let dir = display(&root);
            if !report.left_dirs.contains(&dir) {
                report.left_dirs.push(dir);
            }
        }
        Ok(report)
    }

    /// 保存先の切り替え(1 回の書き込み)。
    async fn switch(&self, target: Target, setting: Option<String>) -> AppResult<()> {
        match target {
            Target::ImagesGlobal => {
                self.db
                    .call_device(move |c| {
                        let tx = c.transaction()?;
                        let value = setting.map(serde_json::Value::String);
                        scopes::write(&tx, Scope::Global, scopes::KEY_IMAGES_ROOT, value.as_ref())?;
                        // 移行すると、以前の版の場所を使う特例はなくなる
                        scopes::write(&tx, Scope::Global, scopes::KEY_LEGACY_IMAGES_USER, None)?;
                        tx.commit()?;
                        Ok(())
                    })
                    .await
            }
            Target::ImagesUser => {
                let patch = SettingsPatch {
                    images_dir: Some(setting),
                    ..SettingsPatch::default()
                };
                self.db
                    .call(move |c| settings::update(c, patch).map(|_| ()))
                    .await
            }
            Target::Models => {
                self.db
                    .call(move |c| settings::set_models_dir(c, setting.as_deref()))
                    .await
            }
        }
    }
}

/// 移行先に書けるかを調べ、空き容量を返す。何も作らない(確認用の一時ファイルは消す)。
fn check_destination(dest: &Path, absolute: bool, found: &mut Vec<Blocker>) -> Option<u64> {
    if !absolute {
        return None;
    }
    let ancestor = files::existing_ancestor(dest);
    if dest.is_file() || !ancestor.is_some_and(writable) {
        found.push(Blocker::DestNotWritable);
    }
    ancestor.and_then(|d| fs4::available_space(d).ok())
}

/// 全体の合計で空き容量を確認し、同じ理由の重複を除く。
fn finish(mut p: Prepared) -> Prepared {
    let need = p.sum(|part| part.analysis.copy_bytes);
    if p.free
        .is_some_and(|f| need > 0 && f < need / 100 * SPACE_MARGIN_PERCENT)
    {
        p.blockers.push(Blocker::NoSpace);
    }
    let mut seen = Vec::new();
    p.blockers.retain(|b| {
        let new = !seen.contains(b);
        seen.push(*b);
        new
    });
    p
}

/// 画像の移行対象: 各画像の原寸・サムネイルと、参照画像(`inputs/`)。
fn image_rels(rows: &[ImageRow], root: &Path) -> Vec<String> {
    let mut rels = Vec::new();
    for row in rows {
        rels.push(row.file_path.clone());
        if row.thumb_path != row.file_path {
            rels.push(row.thumb_path.clone());
        }
    }
    // 参照画像は DB に記録が無い(パラメータの中にだけある)ため、フォルダを走査する。
    // ファイル名は取り込み時に安全な形に限っているが、念のため検証を通ったものだけを移す
    if let Ok(entries) = std::fs::read_dir(root.join("inputs")) {
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.file_name().into_string().ok())
            .map(|name| format!("inputs/{name}"))
            .filter(|rel| is_valid_input_path(rel))
            .collect();
        names.sort();
        rels.extend(names);
    }
    rels
}
