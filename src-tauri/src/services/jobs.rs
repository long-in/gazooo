//! ジョブキューと実行ワーカー(docs/02-architecture.md §3.5)。
//!
//! - ジョブは DB に永続化し、ワーカー(1 タスク)が待機中の先頭から 1 件ずつ実行する。
//! - 状態遷移: queued → running → completed | failed | cancelled。
//! - 起動時、前回 running のまま残っていたジョブは failed(`INTERRUPTED`)にする。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use crate::AppPaths;
use crate::db::Db;
use crate::db::jobs::{self as repo, ImageRow, JobRow};
use crate::domain::job::{ImageItem, Job, JobError, JobStatus, Page, ProgressEvent};
use crate::domain::params::{GenerationParams, MAX_SEED};
use crate::engine::{ImageData, Progress};
use crate::error::{AppError, AppResult};
use crate::services::assistant::capabilities::catalog;
use crate::services::engine::EngineService;
use crate::services::events::EventSink;
use crate::services::inputs::{check_inpaint, input_abs, read_input};
use crate::services::models::compat::check_params;
use crate::services::settings;
use crate::services::thumbs::{make_thumbnail, thumb_rel_path};
use crate::util::{now_ms, year_month};

/// 一度に投入できるジョブ数の上限。
pub const MAX_ENQUEUE_COUNT: u32 = 20;
/// 接続断で失敗したジョブの自動再実行の上限(docs/47 §4.3)。
const MAX_REQUEUES: u32 = 2;
/// 進捗イベントの最小間隔(最大 10 回/秒)。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

struct Active {
    job_id: String,
    cancel: Arc<Notify>,
}

enum RunError {
    Cancelled,
    Failed(AppError),
}

impl From<AppError> for RunError {
    fn from(e: AppError) -> Self {
        Self::Failed(e)
    }
}

/// 画像生成の開始前に呼ばれる(ローカル LLM のメモリ解放。docs/30 §5)。
#[async_trait::async_trait]
pub trait GenerationHook: Send + Sync {
    async fn before_generation(&self);
}

pub struct JobService {
    db: Db,
    engine: Arc<EngineService>,
    sink: Arc<dyn EventSink>,
    paths: AppPaths,
    wake: Notify,
    active: Mutex<Option<Active>>,
    /// 一時停止中は、次のジョブを始めない(実行中のものは完了まで続く。docs/20 §3.3)。アプリ起動中のみ保持。
    paused: AtomicBool,
    hook: std::sync::OnceLock<Arc<dyn GenerationHook>>,
    /// ComfyUI の自動復旧中は、次のジョブを始めない(docs/47 §4.3)。
    recovery_hold: AtomicBool,
    /// 接続断で止まった実行中ジョブを、失敗にせず待機の先頭へ戻す(自動復旧が有効なとき)。
    recovery_requeue: AtomicBool,
    /// 保存先のデータ移行中は、次のジョブを始めない(docs/51 §5)。
    storage_hold: AtomicBool,
    /// ジョブの開始(取り出しから running への変更まで)と、移行の開始がすれ違わないようにする。
    start_gate: tokio::sync::Mutex<()>,
    /// ジョブごとの自動再実行の回数(メモリ上。アプリ再起動で 0 に戻る)。
    requeue_counts: Mutex<std::collections::HashMap<String, u32>>,
}

impl JobService {
    pub fn new(
        db: Db,
        engine: Arc<EngineService>,
        sink: Arc<dyn EventSink>,
        paths: AppPaths,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            engine,
            sink,
            paths,
            wake: Notify::new(),
            active: Mutex::new(None),
            paused: AtomicBool::new(false),
            hook: std::sync::OnceLock::new(),
            recovery_hold: AtomicBool::new(false),
            recovery_requeue: AtomicBool::new(false),
            storage_hold: AtomicBool::new(false),
            start_gate: tokio::sync::Mutex::new(()),
            requeue_counts: Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// 自動復旧の状態を反映する。`hold` の間は次のジョブを始めない。`requeue` が真なら、接続断の失敗を待機へ戻す。
    pub fn set_recovery(&self, hold: bool, requeue: bool) {
        self.recovery_requeue.store(requeue, Ordering::SeqCst);
        let was = self.recovery_hold.swap(hold, Ordering::SeqCst);
        if was && !hold {
            self.wake.notify_one();
        }
    }

    /// 次のジョブの開始を止める / 再開する(保存先のデータ移行中)。止めるときは、
    /// 開始処理の途中のジョブが running になるまで待つ(呼び出し側は、そのあと実行中を確認できる)。
    pub async fn set_storage_hold(&self, hold: bool) {
        self.storage_hold.store(hold, Ordering::SeqCst);
        if hold {
            drop(self.start_gate.lock().await);
        } else {
            self.wake.notify_one();
        }
    }

    /// 接続断で失敗したジョブを待機へ戻してよいか(1 ジョブにつき最大 `MAX_REQUEUES` 回)。
    fn take_requeue_slot(&self, id: &str) -> bool {
        if !self.recovery_requeue.load(Ordering::SeqCst) {
            return false;
        }
        let Ok(mut m) = self.requeue_counts.lock() else {
            return false;
        };
        let n = m.entry(id.to_owned()).or_insert(0);
        if *n >= MAX_REQUEUES {
            return false;
        }
        *n += 1;
        true
    }

    /// 生成の開始前に呼ぶ処理を設定する(起動時に 1 回)。
    pub fn set_generation_hook(&self, hook: Arc<dyn GenerationHook>) {
        let _ = self.hook.set(hook);
    }

    /// 起動時の復旧。前回の実行中ジョブを失敗扱いにし、待機中ジョブがあればワーカーを起こす。
    pub async fn recover(&self) -> AppResult<()> {
        let n = self
            .db
            .call(|c| repo::fail_interrupted(c, now_ms()))
            .await?;
        if n > 0 {
            tracing::warn!(count = n, "interrupted jobs marked as failed");
        }
        self.wake.notify_one();
        Ok(())
    }

    // ---- 公開 API(コマンドから呼ぶ) -------------------------------------------------

    /// 未分類へ投入する(テスト用の短縮形)。
    #[cfg(test)]
    pub async fn enqueue(&self, params: GenerationParams, count: u32) -> AppResult<Vec<Job>> {
        self.enqueue_in(params, count, None).await
    }

    /// プロジェクトを指定して投入する(`None` = 未分類。docs/40 §4.2)。生成画像はそのプロジェクトへ入り、
    /// 投入に成功したらそのプロジェクトの「最後の設定」を更新する。
    pub async fn enqueue_in(
        &self,
        params: GenerationParams,
        count: u32,
        project_id: Option<String>,
    ) -> AppResult<Vec<Job>> {
        params.validate()?;
        if let Some(pid) = project_id.clone() {
            let found = self
                .db
                .call(move |c| crate::db::projects::exists(c, &pid))
                .await?;
            if !found {
                return Err(AppError::ProjectNotFound(project_id.unwrap_or_default()));
            }
        }
        // カタログで用途が分かるモデル・エンコーダ・VAE が、種別と合っているか(docs/20 §2.4)
        if let Some(catalog) = catalog() {
            check_params(catalog, &params)?;
        }
        if !(1..=MAX_ENQUEUE_COUNT).contains(&count) {
            return Err(AppError::validation(
                "count",
                format!("投入数は 1〜{MAX_ENQUEUE_COUNT} で指定してください"),
            ));
        }
        // 参照画像(ControlNet)が保存先に残っているかを先に確認する(無ければ投入せずに知らせる)
        let units = params.active_controlnet_units();
        if !units.is_empty() {
            let root = self.images_root().await?;
            for u in units {
                if !input_abs(&root, &u.image)?.is_file() {
                    return Err(AppError::ImageNotFound(format!(
                        "参照画像がありません: {}",
                        u.image
                    )));
                }
            }
        }
        if let Some(ip) = params.active_inpaint() {
            let root = self.images_root().await?;
            check_inpaint(root, &ip.image, &ip.mask).await?;
        }
        if let Some(i2i) = params.active_img2img() {
            let root = self.images_root().await?;
            if !input_abs(&root, &i2i.image)?.is_file() {
                return Err(AppError::ImageNotFound(format!(
                    "元画像がありません: {}",
                    i2i.image
                )));
            }
        }
        if let Some(r) = params.active_reference() {
            let root = self.images_root().await?;
            for rel in &r.images {
                if !input_abs(&root, rel)?.is_file() {
                    return Err(AppError::ImageNotFound(format!(
                        "参照画像がありません: {rel}"
                    )));
                }
            }
        }
        // 顔の入れ替えの対象画像と顔画像(docs/42 §4.2)
        if let Some(f) = params.active_face_swap() {
            let root = self.images_root().await?;
            for (label, rel) in [("対象画像", &f.target), ("顔画像", &f.face)] {
                if !input_abs(&root, rel)?.is_file() {
                    return Err(AppError::ImageNotFound(format!(
                        "{label}がありません: {rel}"
                    )));
                }
            }
        }
        let mut jobs = Vec::with_capacity(count as usize);
        for i in 0..count {
            let mut p = params.clone();
            p.seed = if params.seed_randomize {
                random_seed()
            } else {
                (params.seed + u64::from(i)) % (MAX_SEED + 1)
            };
            jobs.push(self.insert_job(&p, project_id.clone()).await?);
        }
        if let Some(pid) = project_id {
            // 失敗しても投入自体は成功扱い(次回の投入で更新される)
            let json = serde_json::to_string(&params)?;
            let saved = self
                .db
                .call(move |c| crate::db::projects::set_last_params(c, &pid, &json, now_ms()))
                .await;
            if let Err(e) = saved {
                tracing::warn!(error = %e, "failed to save project last params");
            }
        }
        self.wake.notify_one();
        Ok(jobs)
    }

    pub async fn list(
        &self,
        statuses: Vec<JobStatus>,
        cursor: Option<String>,
        limit: Option<u32>,
    ) -> AppResult<Page<Job>> {
        let limit = i64::from(limit.unwrap_or(50).clamp(1, 200));
        let offset: i64 = cursor
            .as_deref()
            .map(|c| c.parse::<i64>())
            .transpose()
            .map_err(|_| AppError::validation("cursor", "cursor が不正です"))?
            .unwrap_or(0)
            .max(0);
        self.db
            .call(move |c| {
                let mut rows = repo::list(c, &statuses, limit + 1, offset)?;
                let next_cursor = (rows.len() as i64 > limit).then(|| (offset + limit).to_string());
                rows.truncate(limit as usize);
                let items = rows
                    .iter()
                    .map(|r| build_job(c, r))
                    .collect::<AppResult<Vec<_>>>()?;
                Ok(Page { items, next_cursor })
            })
            .await
    }

    /// 実行中・待機中のジョブ数(ComfyUI の再起動前の確認に使う。docs/15 §5)。
    pub async fn active_counts(&self) -> AppResult<(usize, usize)> {
        self.db
            .call(|c| {
                let running = repo::list(c, &[JobStatus::Running], 1000, 0)?.len();
                let queued = repo::list(c, &[JobStatus::Queued], 1000, 0)?.len();
                Ok((running, queued))
            })
            .await
    }

    pub async fn cancel(&self, id: &str) -> AppResult<()> {
        let id_owned = id.to_owned();
        let row = self.require(id).await?;
        match row.status {
            JobStatus::Queued => {
                let cancelled = self
                    .db
                    .call({
                        let id = id_owned.clone();
                        move |c| repo::cancel_queued(c, &id, now_ms())
                    })
                    .await?;
                if cancelled {
                    self.emit_updated(id).await;
                    return Ok(());
                }
                // ワーカーが先に実行を開始していた場合は、実行中の取消として扱う
                self.cancel_running(id)
            }
            JobStatus::Running => self.cancel_running(id),
            _ => Err(AppError::JobInvalidState(
                "終了済みのジョブは取り消せません".into(),
            )),
        }
    }

    fn cancel_running(&self, id: &str) -> AppResult<()> {
        let active = self
            .active
            .lock()
            .map_err(|_| AppError::internal("job state lock poisoned"))?;
        match active.as_ref() {
            Some(a) if a.job_id == id => {
                a.cancel.notify_one();
                Ok(())
            }
            _ => Err(AppError::JobInvalidState(
                "このジョブは実行中ではありません".into(),
            )),
        }
    }

    pub async fn reorder(&self, id: &str, new_position: usize) -> AppResult<()> {
        let id = id.to_owned();
        self.db
            .call(move |c| repo::reorder(c, &id, new_position))
            .await?;
        // 並び順が変わった待機中ジョブをまとめて通知する
        let queued = self.list(vec![JobStatus::Queued], None, Some(200)).await?;
        for job in &queued.items {
            self.sink.job_updated(job);
        }
        Ok(())
    }

    pub async fn retry(&self, id: &str) -> AppResult<Job> {
        let row = self.require(id).await?;
        if !matches!(row.status, JobStatus::Failed | JobStatus::Cancelled) {
            return Err(AppError::JobInvalidState(
                "失敗またはキャンセルされたジョブのみ再試行できます".into(),
            ));
        }
        let params: GenerationParams = serde_json::from_str(&row.params_json)?;
        let job = self.insert_job(&params, row.project_id.clone()).await?;
        self.wake.notify_one();
        Ok(job)
    }

    pub async fn clear_finished(&self) -> AppResult<usize> {
        self.db.call(repo::clear_finished).await
    }

    /// 指定した終了済みジョブを履歴から外す(docs/20 §3.4)。
    pub async fn remove(&self, ids: Vec<String>) -> AppResult<usize> {
        self.db.call(move |c| repo::remove_finished(c, &ids)).await
    }

    /// 実行中のジョブがあるか(ユーザーの切り替えの確認に使う。docs/36 §5.4)。
    pub fn has_running(&self) -> bool {
        self.active.lock().map(|a| a.is_some()).unwrap_or(false)
    }

    /// ワーカーを起こす(ログイン直後に、そのユーザーの待機中ジョブを始めるため)。
    pub fn wake_worker(&self) {
        self.wake.notify_one();
    }

    /// 実行中のジョブを取り消し、終わるまで待つ(ログアウトの前処理。最大 30 秒)。
    /// ジョブはユーザーの DB に結果を書くため、DB を手放す前に必ず終わらせる。
    pub async fn stop_running(&self) -> AppResult<()> {
        if let Ok(active) = self.active.lock()
            && let Some(a) = active.as_ref()
        {
            a.cancel.notify_one();
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while self.has_running() {
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::user(
                    "USER_BUSY",
                    "実行中のジョブを終了できませんでした。しばらくしてからやり直してください",
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// 次のジョブを始めるのを止める。実行中のジョブは完了まで続く。
    pub fn pause(&self) {
        if !self.paused.swap(true, Ordering::SeqCst) {
            self.sink.queue_state(true);
        }
    }

    pub fn resume(&self) {
        if self.paused.swap(false, Ordering::SeqCst) {
            self.sink.queue_state(false);
        }
        self.wake.notify_one();
    }

    // ---- ワーカー --------------------------------------------------------------------

    /// 待機中ジョブを順に実行し続ける(アプリの寿命と同じ)。
    pub async fn run_worker(self: Arc<Self>) {
        loop {
            // ログインしていない間は動かさない(待機中ジョブは、そのユーザーのログインまで実行しない。docs/36 §5.4)
            if !self.db.is_attached() {
                let _ =
                    tokio::time::timeout(Duration::from_millis(500), self.wake.notified()).await;
                continue;
            }
            if self.is_paused() {
                self.wake.notified().await;
                continue;
            }
            // ComfyUI の自動復旧中は待つ(復旧の終了で起こされる。取りこぼしに備えて定期的にも見る)
            if self.recovery_hold.load(Ordering::SeqCst) {
                let _ = tokio::time::timeout(Duration::from_secs(1), self.wake.notified()).await;
                continue;
            }
            let gate = self.start_gate.lock().await;
            if self.storage_hold.load(Ordering::SeqCst) {
                drop(gate);
                let _ = tokio::time::timeout(Duration::from_secs(1), self.wake.notified()).await;
                continue;
            }
            match self.db.call(|c| repo::next_queued(c)).await {
                Ok(Some(row)) => self.execute(row, gate).await,
                Ok(None) => {
                    drop(gate);
                    self.wake.notified().await;
                }
                Err(e) => {
                    drop(gate);
                    tracing::error!(error = %e, "job worker: failed to fetch next job");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    async fn execute(&self, row: JobRow, gate: tokio::sync::MutexGuard<'_, ()>) {
        let id = row.id.clone();
        let started = self
            .db
            .call({
                let id = id.clone();
                move |c| repo::mark_running(c, &id, now_ms())
            })
            .await;
        drop(gate);
        match started {
            Ok(true) => {}
            Ok(false) => return, // 直前に取り消された
            Err(e) => {
                tracing::error!(job = %id, error = %e, "failed to mark job running");
                return;
            }
        }
        self.emit_updated(&id).await;
        let job_started = Instant::now();
        tracing::debug!(job = %id, "job started");

        let cancel = Arc::new(Notify::new());
        if let Ok(mut a) = self.active.lock() {
            *a = Some(Active {
                job_id: id.clone(),
                cancel: Arc::clone(&cancel),
            });
        }
        if let Some(h) = self.hook.get() {
            h.before_generation().await;
        }
        let outcome = self.run(&row, &cancel).await;
        if let Ok(mut a) = self.active.lock() {
            *a = None;
        }

        // 接続断で止まったジョブは、自動復旧が有効なら失敗にせず待機の先頭へ戻す(docs/47 §4.3)
        if let Err(RunError::Failed(e)) = &outcome
            && e.code() == "ENGINE_UNREACHABLE"
            && self.take_requeue_slot(&id)
        {
            let requeued = self
                .db
                .call({
                    let id = id.clone();
                    move |c| repo::requeue_front(c, &id)
                })
                .await;
            if matches!(requeued, Ok(true)) {
                tracing::warn!(job = %id, "engine unreachable; job returned to the queue");
                self.emit_updated(&id).await;
                return;
            }
        }

        let (status, error) = match outcome {
            Ok(()) => (JobStatus::Completed, None),
            Err(RunError::Cancelled) => (JobStatus::Cancelled, None),
            Err(RunError::Failed(e)) => {
                tracing::warn!(job = %id, code = e.code(), "job failed");
                (
                    JobStatus::Failed,
                    Some((e.code().to_owned(), e.to_string())),
                )
            }
        };
        tracing::debug!(
            job = %id,
            status = ?status,
            elapsed_ms = job_started.elapsed().as_millis() as u64,
            "job finished"
        );
        let finished = self
            .db
            .call({
                let id = id.clone();
                move |c| {
                    repo::finish(
                        c,
                        &id,
                        status,
                        error
                            .as_ref()
                            .map(|(code, msg)| (code.as_str(), msg.as_str())),
                        now_ms(),
                    )
                }
            })
            .await;
        if let Err(e) = finished {
            tracing::error!(job = %id, error = %e, "failed to finalize job");
        }
        self.emit_updated(&id).await;
    }

    async fn run(&self, row: &JobRow, cancel: &Notify) -> Result<(), RunError> {
        let params: GenerationParams = serde_json::from_str(&row.params_json).map_err(|e| {
            RunError::Failed(AppError::internal(format!(
                "保存済みパラメータが不正です: {e}"
            )))
        })?;
        let engine = self.engine.engine()?;
        // 参照画像(ControlNet)をエンジンへ送る。順序は有効なユニットの順
        let mut uploaded = Vec::new();
        for u in params.active_controlnet_units() {
            let root = self.images_root().await?;
            let bytes = read_input(root, u.image.clone()).await?;
            uploaded.push(engine.upload_input(bytes).await?);
        }
        // Inpaint の画像とマスク(ControlNet の後ろ)
        if let Some(ip) = params.active_inpaint() {
            for rel in [&ip.image, &ip.mask] {
                let root = self.images_root().await?;
                let bytes = read_input(root, rel.clone()).await?;
                uploaded.push(engine.upload_input(bytes).await?);
            }
        }
        // img2img の元画像(Inpaint とは排他。ControlNet の後ろ)
        if let Some(i2i) = params.active_img2img() {
            let root = self.images_root().await?;
            let bytes = read_input(root, i2i.image.clone()).await?;
            uploaded.push(engine.upload_input(bytes).await?);
        }
        // 参照画像つき生成(Qwen-Image 2.1。他の画像入力と排他)。順序は 1 枚目(編集対象)から
        if let Some(r) = params.active_reference() {
            for rel in &r.images {
                let root = self.images_root().await?;
                let bytes = read_input(root, rel.clone()).await?;
                uploaded.push(engine.upload_input(bytes).await?);
            }
        }
        // 顔の入れ替え(他の画像入力と排他)。順序は 対象 → 顔
        if let Some(f) = params.active_face_swap() {
            for rel in [&f.target, &f.face] {
                let root = self.images_root().await?;
                let bytes = read_input(root, rel.clone()).await?;
                uploaded.push(engine.upload_input(bytes).await?);
            }
        }
        let remote = engine.submit(&params, &uploaded).await?;
        tracing::debug!(job = %row.id, remote = %remote, uploads = uploaded.len(), "prompt submitted");
        self.db
            .call({
                let (id, remote) = (row.id.clone(), remote.clone());
                move |c| repo::set_remote_id(c, &id, &remote)
            })
            .await?;

        let last_emit: Mutex<Option<Instant>> = Mutex::new(None);
        let job_id = row.id.clone();
        let on_progress = |p: Progress| {
            let done = p.total > 0 && p.step >= p.total;
            if let Ok(mut last) = last_emit.lock() {
                let due = last.is_none_or(|t| t.elapsed() >= PROGRESS_INTERVAL);
                if !(due || done) {
                    return;
                }
                *last = Some(Instant::now());
            }
            self.sink.job_progress(&ProgressEvent {
                job_id: job_id.clone(),
                step: p.step,
                total_steps: p.total,
                node: p.node,
                phase: p.phase,
            });
        };

        // 取消の通知(待機中に押された分も含め、Notify の許可証として保持される)と競合させる
        let images = tokio::select! {
            result = engine.wait(&remote, &on_progress) => result?,
            _ = cancel.notified() => {
                if let Err(e) = engine.cancel(&remote).await {
                    tracing::warn!(error = %e, "failed to cancel remote job");
                }
                return Err(RunError::Cancelled);
            }
        };
        tracing::debug!(job = %row.id, images = images.len(), "images received from engine");
        self.save_images(row, &params, images).await?;
        Ok(())
    }

    // ---- 内部 ------------------------------------------------------------------------

    async fn require(&self, id: &str) -> AppResult<JobRow> {
        let id_owned = id.to_owned();
        self.db
            .call(move |c| repo::get(c, &id_owned))
            .await?
            .ok_or_else(|| AppError::JobNotFound(id.to_owned()))
    }

    async fn insert_job(
        &self,
        params: &GenerationParams,
        project_id: Option<String>,
    ) -> AppResult<Job> {
        let id = uuid::Uuid::now_v7().to_string();
        let json = serde_json::to_string(params)?;
        let (prompt, model) = (params.prompt.clone(), params.model.name.clone());
        let job = self
            .db
            .call({
                let id = id.clone();
                move |c| {
                    repo::insert_in_project(
                        c,
                        &id,
                        &json,
                        &prompt,
                        &model,
                        now_ms(),
                        project_id.as_deref(),
                    )?;
                    let row =
                        repo::get(c, &id)?.ok_or_else(|| AppError::internal("job vanished"))?;
                    build_job(c, &row)
                }
            })
            .await?;
        self.sink.job_updated(&job);
        Ok(job)
    }

    async fn emit_updated(&self, id: &str) {
        let id = id.to_owned();
        let job = self
            .db
            .call(move |c| match repo::get(c, &id)? {
                Some(row) => build_job(c, &row).map(Some),
                None => Ok(None),
            })
            .await;
        if let Ok(Some(job)) = job {
            self.sink.job_updated(&job);
        }
    }

    async fn images_root(&self) -> AppResult<PathBuf> {
        let s = self.db.call(|c| settings::load(c)).await?;
        Ok(self.paths.resolve_images_dir(&s))
    }

    /// 生成画像をファイルに保存し、DB に記録して通知する。
    async fn save_images(
        &self,
        row: &JobRow,
        params: &GenerationParams,
        images: Vec<ImageData>,
    ) -> AppResult<()> {
        let root = self.images_root().await?;
        for img in images {
            let now = now_ms();
            let (year, month) = year_month(now);
            let id = uuid::Uuid::now_v7().to_string();
            let rel = format!(
                "{year:04}/{month:02}/{id}.{}",
                safe_extension(&img.filename)
            );
            let abs = join_rel(&root, &rel);

            let file_size = img.bytes.len() as u64;
            let target = abs.clone();
            let thumb_rel = thumb_rel_path(&id);
            let thumb_target = join_rel(&root, &thumb_rel);
            // 原寸の保存とサムネイル生成(サムネイルが作れなくても原寸の保存は成功扱い)
            let thumb_ok = tokio::task::spawn_blocking(move || -> std::io::Result<bool> {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&target, &img.bytes)?;
                let Some(thumb) = make_thumbnail(&img.bytes) else {
                    return Ok(false);
                };
                if let Some(parent) = thumb_target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                Ok(std::fs::write(&thumb_target, thumb).is_ok())
            })
            .await
            .map_err(|e| AppError::internal(e.to_string()))??;

            let image_row = ImageRow {
                id,
                job_id: row.id.clone(),
                // サムネイルを作れなかった場合は原寸を指す(起動時のバックフィルで再試行される)
                thumb_path: if thumb_ok { thumb_rel } else { rel.clone() },
                file_path: rel,
                width: params.width,
                height: params.height,
                seed: params.seed,
                file_size,
                is_favorite: false,
                created_at: now,
                project_id: row.project_id.clone(),
            };
            let inserted = self
                .db
                .call({
                    let r = image_row.clone();
                    move |c| repo::insert_image(c, &r)
                })
                .await;
            if let Err(e) = inserted {
                let _ = std::fs::remove_file(&abs); // 記録できなかった孤立ファイルを残さない
                return Err(e);
            }
            self.sink.image_created(&image_item(&root, &image_row));
        }
        Ok(())
    }
}

pub(crate) fn build_job(conn: &rusqlite::Connection, row: &JobRow) -> AppResult<Job> {
    Ok(Job {
        id: row.id.clone(),
        status: row.status,
        position: row.position,
        params: serde_json::from_str(&row.params_json).unwrap_or(serde_json::Value::Null),
        error: match (&row.error_code, &row.error_message) {
            (Some(code), Some(message)) => Some(JobError {
                code: code.clone(),
                message: message.clone(),
            }),
            _ => None,
        },
        image_ids: repo::image_ids(conn, &row.id)?,
        project_id: row.project_id.clone(),
        created_at: row.created_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
    })
}

pub fn image_item(root: &Path, r: &ImageRow) -> ImageItem {
    ImageItem {
        id: r.id.clone(),
        job_id: r.job_id.clone(),
        width: r.width,
        height: r.height,
        seed: r.seed,
        is_favorite: r.is_favorite,
        created_at: r.created_at,
        project_id: r.project_id.clone(),
        file_path: join_rel(root, &r.file_path).to_string_lossy().into_owned(),
        thumb_path: join_rel(root, &r.thumb_path).to_string_lossy().into_owned(),
    }
}

/// `/` 区切りの相対パスを OS のパスに結合する(Windows でも `\` を直書きしない)。
pub(crate) fn join_rel(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.to_path_buf(), |p, part| p.join(part))
}

/// 拡張子を安全な英数字に限定する(ファイル名はエンジン由来なので信用しない)。
fn safe_extension(filename: &str) -> String {
    Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty() && e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "png".to_owned())
}

fn random_seed() -> u64 {
    (uuid::Uuid::new_v4().as_u128() as u64) & MAX_SEED
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::params::{ModelKind, sample};
    use crate::engine::{EngineModels, EngineStatus, GenerationEngine};
    use async_trait::async_trait;
    use std::collections::HashMap;

    // ---- 模擬エンジン: プロンプトで挙動を切り替える("hang" / "fail" / それ以外は成功) -------

    #[derive(Default)]
    struct FakeEngine {
        prompts: Mutex<HashMap<String, String>>, // remote_id -> prompt
        submitted: Mutex<Vec<String>>,           // 投入されたプロンプトの順序
        cancelled: Mutex<Vec<String>>,
        counter: Mutex<u32>,
        uploads: Mutex<Vec<usize>>, // アップロードされた参照画像のバイト数
        submit_inputs: Mutex<Vec<Vec<String>>>, // submit に渡された参照画像名
    }

    #[async_trait]
    impl GenerationEngine for FakeEngine {
        fn url(&self) -> String {
            "http://fake".into()
        }
        async fn health(&self) -> AppResult<EngineStatus> {
            unreachable!()
        }
        async fn list_models(&self) -> AppResult<EngineModels> {
            unreachable!()
        }
        async fn upload_input(&self, png: Vec<u8>) -> AppResult<String> {
            self.uploads.lock().unwrap().push(png.len());
            Ok("uploaded.png".into())
        }
        async fn submit(&self, params: &GenerationParams, inputs: &[String]) -> AppResult<String> {
            self.submit_inputs.lock().unwrap().push(inputs.to_vec());
            if params.prompt == "submit-error" {
                return Err(AppError::EngineModelMissing("ckpt_name: 'x'".into()));
            }
            let mut n = self.counter.lock().unwrap();
            *n += 1;
            let id = format!("remote-{n}");
            self.prompts
                .lock()
                .unwrap()
                .insert(id.clone(), params.prompt.clone());
            self.submitted.lock().unwrap().push(params.prompt.clone());
            Ok(id)
        }
        async fn wait(
            &self,
            remote_id: &str,
            on_progress: &(dyn Fn(Progress) + Send + Sync),
        ) -> AppResult<Vec<ImageData>> {
            let prompt = self.prompts.lock().unwrap()[remote_id].clone();
            match prompt.as_str() {
                "hang" => std::future::pending().await,
                "unreachable" => Err(AppError::EngineUnreachable("connection refused".into())),
                "fail" => Err(AppError::EngineExecutionFailed(
                    "KSampler: out of memory".into(),
                )),
                "png" => Ok(vec![ImageData {
                    filename: "real.png".into(),
                    bytes: crate::services::thumbs::png_bytes(1000, 600),
                }]),
                _ => {
                    on_progress(Progress {
                        step: 1,
                        total: 2,
                        node: Some("5".into()),
                        phase: None,
                    });
                    on_progress(Progress {
                        step: 2,
                        total: 2,
                        node: Some("5".into()),
                        phase: None,
                    });
                    Ok(vec![
                        ImageData {
                            filename: "gazooo_00001_.png".into(),
                            bytes: b"PNG-1".to_vec(),
                        },
                        ImageData {
                            filename: "weird name.PNG!".into(),
                            bytes: b"PNG-2".to_vec(),
                        },
                    ])
                }
            }
        }
        async fn cancel(&self, remote_id: &str) -> AppResult<()> {
            self.cancelled.lock().unwrap().push(remote_id.to_owned());
            Ok(())
        }
    }

    // ---- 記録用のイベント受け口 ----------------------------------------------------------

    #[derive(Default)]
    struct Recorder {
        updated: Mutex<Vec<(String, JobStatus)>>,
        progress: Mutex<Vec<ProgressEvent>>,
        images: Mutex<Vec<ImageItem>>,
    }

    impl EventSink for Recorder {
        fn job_updated(&self, job: &Job) {
            self.updated
                .lock()
                .unwrap()
                .push((job.id.clone(), job.status));
        }
        fn job_progress(&self, p: &ProgressEvent) {
            self.progress.lock().unwrap().push(p.clone());
        }
        fn image_created(&self, i: &ImageItem) {
            self.images.lock().unwrap().push(i.clone());
        }
        fn image_deleted(&self, _: &[String]) {}
    }

    struct Harness {
        svc: Arc<JobService>,
        engine: Arc<FakeEngine>,
        events: Arc<Recorder>,
        dir: PathBuf,
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn harness() -> Harness {
        let dir = std::env::temp_dir().join(format!("gazooo-jobs-{}", uuid::Uuid::now_v7()));
        let engine = Arc::new(FakeEngine::default());
        let events = Arc::new(Recorder::default());
        let svc = JobService::new(
            Db::open_in_memory().unwrap(),
            Arc::new(EngineService::from_engine(engine.clone())),
            events.clone(),
            AppPaths::new(dir.clone()),
        );
        Harness {
            svc,
            engine,
            events,
            dir,
        }
    }

    fn params(prompt: &str) -> GenerationParams {
        let mut p = sample(ModelKind::Sd15);
        p.prompt = prompt.into();
        p
    }

    async fn status_of(h: &Harness, id: &str) -> JobStatus {
        let page = h.svc.list(vec![], None, Some(200)).await.unwrap();
        page.items.iter().find(|j| j.id == id).unwrap().status
    }

    async fn wait_for_status(h: &Harness, id: &str, want: JobStatus) {
        for _ in 0..200 {
            if status_of(h, id).await == want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "job {id} did not reach {want:?}, is {:?}",
            status_of(h, id).await
        );
    }

    // ---- テスト ---------------------------------------------------------------------------

    #[tokio::test]
    async fn runs_job_saves_images_and_emits_events() {
        let h = harness();
        tokio::spawn(h.svc.clone().run_worker());
        let jobs = h.svc.enqueue(params("hello"), 1).await.unwrap();
        let id = jobs[0].id.clone();
        wait_for_status(&h, &id, JobStatus::Completed).await;

        // 画像 2 枚が保存され、DB と通知に反映される
        let images = h.events.images.lock().unwrap().clone();
        assert_eq!(images.len(), 2);
        for img in &images {
            assert!(Path::new(&img.file_path).is_file(), "{}", img.file_path);
            assert_eq!((img.width, img.height, img.seed), (512, 512, 42));
        }
        assert_eq!(std::fs::read(&images[0].file_path).unwrap(), b"PNG-1");
        // 想定外のファイル名でも拡張子は安全な値になる
        assert!(images[1].file_path.ends_with(".png"));
        let job = &h.svc.list(vec![], None, None).await.unwrap().items[0];
        assert_eq!(job.image_ids.len(), 2);

        // running → completed の順に通知され、進捗も届く
        let statuses: Vec<_> = h
            .events
            .updated
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.1)
            .collect();
        assert_eq!(
            statuses,
            [JobStatus::Queued, JobStatus::Running, JobStatus::Completed]
        );
        let progress = h.events.progress.lock().unwrap();
        assert_eq!(
            progress.last().unwrap().step,
            2,
            "完了時の進捗は必ず通知される"
        );
        assert_eq!(progress[0].job_id, id);
    }

    #[tokio::test]
    async fn project_jobs_put_images_in_the_project_and_remember_the_last_params() {
        let h = harness();
        h.svc
            .db
            .call(|c| crate::db::projects::insert(c, "p1", "キャラA", None, 1))
            .await
            .unwrap();
        tokio::spawn(h.svc.clone().run_worker());

        // 存在しないプロジェクトは投入できない(何も作られない)
        assert_eq!(
            h.svc
                .enqueue_in(params("x"), 1, Some("nope".into()))
                .await
                .unwrap_err()
                .code(),
            "PROJECT_NOT_FOUND"
        );
        assert!(
            h.svc
                .list(vec![], None, None)
                .await
                .unwrap()
                .items
                .is_empty()
        );

        let job = h
            .svc
            .enqueue_in(params("in project"), 1, Some("p1".into()))
            .await
            .unwrap()
            .remove(0);
        assert_eq!(job.project_id.as_deref(), Some("p1"));
        wait_for_status(&h, &job.id, JobStatus::Completed).await;

        // 生成画像はそのプロジェクトに入り、プロジェクトに最後の設定が残る
        let images = h.events.images.lock().unwrap().clone();
        assert_eq!(images.len(), 2);
        assert!(images.iter().all(|i| i.project_id.as_deref() == Some("p1")));
        let last = h
            .svc
            .db
            .call(|c| Ok(crate::db::projects::get(c, "p1")?.unwrap().last_params_json))
            .await
            .unwrap()
            .expect("last params");
        assert!(last.contains("in project"));

        // 未分類の投入は、プロジェクトの設定に触れない
        let plain = h.svc.enqueue(params("plain"), 1).await.unwrap().remove(0);
        assert_eq!(plain.project_id, None);
        wait_for_status(&h, &plain.id, JobStatus::Completed).await;
        let last2 = h
            .svc
            .db
            .call(|c| Ok(crate::db::projects::get(c, "p1")?.unwrap().last_params_json))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(last, last2);
    }

    #[tokio::test]
    async fn retry_keeps_the_project_and_deleted_project_falls_back_to_unassigned() {
        let h = harness();
        h.svc
            .db
            .call(|c| crate::db::projects::insert(c, "p1", "A", None, 1))
            .await
            .unwrap();
        tokio::spawn(h.svc.clone().run_worker());
        let id = h
            .svc
            .enqueue_in(params("fail"), 1, Some("p1".into()))
            .await
            .unwrap()[0]
            .id
            .clone();
        wait_for_status(&h, &id, JobStatus::Failed).await;
        let retried = h.svc.retry(&id).await.unwrap();
        assert_eq!(retried.project_id.as_deref(), Some("p1"));

        // プロジェクトを消しても、ジョブは残って未分類へ戻る
        h.svc
            .db
            .call(|c| crate::db::projects::delete(c, "p1").map(|_| ()))
            .await
            .unwrap();
        let jobs = h.svc.list(vec![], None, None).await.unwrap().items;
        assert!(jobs.iter().all(|j| j.project_id.is_none()));
    }

    #[tokio::test]
    async fn generation_hook_runs_before_each_job() {
        struct Counter(std::sync::atomic::AtomicU32);
        #[async_trait::async_trait]
        impl GenerationHook for Counter {
            async fn before_generation(&self) {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let h = harness();
        let hook = Arc::new(Counter(Default::default()));
        h.svc.set_generation_hook(hook.clone());
        tokio::spawn(h.svc.clone().run_worker());
        let jobs = h.svc.enqueue(params("hello"), 2).await.unwrap();
        for j in &jobs {
            wait_for_status(&h, &j.id, JobStatus::Completed).await;
        }
        assert_eq!(hook.0.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn real_images_get_thumbnails_and_undecodable_ones_fall_back() {
        let h = harness();
        tokio::spawn(h.svc.clone().run_worker());
        let id = h.svc.enqueue(params("png"), 1).await.unwrap()[0].id.clone();
        wait_for_status(&h, &id, JobStatus::Completed).await;
        let img = h.events.images.lock().unwrap()[0].clone();
        assert_ne!(img.thumb_path, img.file_path);
        assert!(img.thumb_path.contains("thumbs"));
        let thumb = std::fs::read(&img.thumb_path).unwrap();
        assert_eq!(&thumb[..2], &[0xFF, 0xD8]);

        // デコードできないデータ(模擬の "PNG-1")は原寸を指す
        let id = h.svc.enqueue(params("hello"), 1).await.unwrap()[0]
            .id
            .clone();
        wait_for_status(&h, &id, JobStatus::Completed).await;
        let fallback = h.events.images.lock().unwrap()[1].clone();
        assert_eq!(fallback.thumb_path, fallback.file_path);
    }

    #[tokio::test]
    async fn engine_failure_marks_job_failed_with_code() {
        let h = harness();
        tokio::spawn(h.svc.clone().run_worker());
        for (prompt, code) in [
            ("fail", "ENGINE_EXECUTION_FAILED"),
            ("submit-error", "ENGINE_MODEL_MISSING"),
        ] {
            let id = h.svc.enqueue(params(prompt), 1).await.unwrap()[0]
                .id
                .clone();
            wait_for_status(&h, &id, JobStatus::Failed).await;
            let page = h
                .svc
                .list(vec![JobStatus::Failed], None, None)
                .await
                .unwrap();
            let job = page.items.iter().find(|j| j.id == id).unwrap();
            assert_eq!(job.error.as_ref().unwrap().code, code);
            assert!(job.image_ids.is_empty());
        }
    }

    #[tokio::test]
    async fn unreachable_job_returns_to_the_queue_front_when_recovery_is_on() {
        let h = harness();
        h.svc.set_recovery(false, true);
        let first = h.svc.enqueue(params("ok"), 1).await.unwrap()[0].id.clone();
        let stuck = h.svc.enqueue(params("unreachable"), 1).await.unwrap()[0]
            .id
            .clone();
        // 実行開始 → 接続断 → 待機の先頭へ
        let row = h
            .svc
            .db
            .call({
                let stuck = stuck.clone();
                move |c| repo::get(c, &stuck)
            })
            .await
            .unwrap()
            .unwrap();
        let gate = h.svc.start_gate.lock().await;
        h.svc.execute(row, gate).await;
        assert_eq!(status_of(&h, &stuck).await, JobStatus::Queued);
        let next = h
            .svc
            .db
            .call(|c| repo::next_queued(c))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.id, stuck, "戻したジョブが先頭");
        assert_ne!(next.id, first);
    }

    #[tokio::test]
    async fn unreachable_job_fails_without_recovery_or_after_two_requeues() {
        // 自動復旧が OFF → 従来どおり失敗
        let h = harness();
        tokio::spawn(h.svc.clone().run_worker());
        let id = h.svc.enqueue(params("unreachable"), 1).await.unwrap()[0]
            .id
            .clone();
        wait_for_status(&h, &id, JobStatus::Failed).await;

        // 自動復旧が ON → 2 回までは戻り、3 回目で失敗
        let h = harness();
        h.svc.set_recovery(false, true);
        tokio::spawn(h.svc.clone().run_worker());
        let id = h.svc.enqueue(params("unreachable"), 1).await.unwrap()[0]
            .id
            .clone();
        wait_for_status(&h, &id, JobStatus::Failed).await;
        assert_eq!(h.svc.requeue_counts.lock().unwrap()[&id], MAX_REQUEUES);
    }

    #[tokio::test]
    async fn worker_holds_the_queue_during_recovery_and_resumes_after() {
        let h = harness();
        h.svc.set_recovery(true, true);
        tokio::spawn(h.svc.clone().run_worker());
        let id = h.svc.enqueue(params("ok"), 1).await.unwrap()[0].id.clone();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(status_of(&h, &id).await, JobStatus::Queued);
        h.svc.set_recovery(false, true);
        wait_for_status(&h, &id, JobStatus::Completed).await;
    }

    #[tokio::test]
    async fn worker_does_not_start_jobs_while_storage_is_migrating() {
        let h = harness();
        h.svc.set_storage_hold(true).await;
        tokio::spawn(h.svc.clone().run_worker());
        let id = h.svc.enqueue(params("ok"), 1).await.unwrap()[0].id.clone();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(status_of(&h, &id).await, JobStatus::Queued);
        assert!(h.engine.submitted.lock().unwrap().is_empty());
        h.svc.set_storage_hold(false).await;
        wait_for_status(&h, &id, JobStatus::Completed).await;
    }

    fn controlnet_params(prompt: &str, image: &str) -> GenerationParams {
        use crate::domain::params::{ControlnetParams, ControlnetUnit, Preprocessor};
        let mut p = params(prompt);
        p.controlnet = Some(ControlnetParams {
            enabled: true,
            units: vec![ControlnetUnit {
                image: image.into(),
                model: "canny.safetensors".into(),
                preprocessor: Preprocessor::Canny,
                strength: 1.0,
                start_percent: 0.0,
                end_percent: 1.0,
            }],
        });
        p
    }

    #[tokio::test]
    async fn controlnet_requires_the_reference_image_to_exist() {
        let h = harness();
        let err = h
            .svc
            .enqueue(controlnet_params("x", "inputs/missing.png"), 1)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "IMAGE_NOT_FOUND");
        // 不正なパスは存在確認の前に検証で断る
        let err = h
            .svc
            .enqueue(controlnet_params("x", "inputs/../escape.png"), 1)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
        assert!(
            h.svc
                .list(vec![], None, None)
                .await
                .unwrap()
                .items
                .is_empty()
        );
    }

    #[tokio::test]
    async fn controlnet_job_uploads_reference_before_submit() {
        let h = harness();
        let inputs = h.dir.join("images").join("inputs");
        std::fs::create_dir_all(&inputs).unwrap();
        std::fs::write(inputs.join("ref.png"), b"PNGBYTES").unwrap();

        tokio::spawn(h.svc.clone().run_worker());
        let id = h
            .svc
            .enqueue(controlnet_params("with-cn", "inputs/ref.png"), 1)
            .await
            .unwrap()[0]
            .id
            .clone();
        wait_for_status(&h, &id, JobStatus::Completed).await;
        assert_eq!(*h.engine.uploads.lock().unwrap(), [8]);
        assert_eq!(
            *h.engine.submit_inputs.lock().unwrap(),
            [vec!["uploaded.png".to_string()]]
        );

        // ControlNet を使わないジョブはアップロードしない
        let id = h.svc.enqueue(params("plain"), 1).await.unwrap()[0]
            .id
            .clone();
        wait_for_status(&h, &id, JobStatus::Completed).await;
        assert_eq!(h.engine.uploads.lock().unwrap().len(), 1);
        assert_eq!(
            h.engine.submit_inputs.lock().unwrap()[1],
            Vec::<String>::new()
        );
    }

    #[tokio::test]
    async fn controlnet_job_fails_when_reference_vanished_after_enqueue() {
        let h = harness();
        let inputs = h.dir.join("images").join("inputs");
        std::fs::create_dir_all(&inputs).unwrap();
        std::fs::write(inputs.join("gone.png"), b"x").unwrap();
        let id = h
            .svc
            .enqueue(controlnet_params("x", "inputs/gone.png"), 1)
            .await
            .unwrap()[0]
            .id
            .clone();
        std::fs::remove_file(inputs.join("gone.png")).unwrap(); // 待機中に消えた
        tokio::spawn(h.svc.clone().run_worker());
        wait_for_status(&h, &id, JobStatus::Failed).await;
        let page = h
            .svc
            .list(vec![JobStatus::Failed], None, None)
            .await
            .unwrap();
        assert_eq!(
            page.items[0].error.as_ref().unwrap().code,
            "IMAGE_NOT_FOUND"
        );
    }

    #[tokio::test]
    async fn cancel_running_job_interrupts_engine() {
        let h = harness();
        tokio::spawn(h.svc.clone().run_worker());
        let id = h.svc.enqueue(params("hang"), 1).await.unwrap()[0]
            .id
            .clone();
        wait_for_status(&h, &id, JobStatus::Running).await;
        // submit 完了(= 取消が効く状態)まで待つ
        while h.engine.submitted.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        h.svc.cancel(&id).await.unwrap();
        wait_for_status(&h, &id, JobStatus::Cancelled).await;
        assert_eq!(*h.engine.cancelled.lock().unwrap(), ["remote-1"]);

        // ワーカーは次のジョブに進める
        let next = h.svc.enqueue(params("ok"), 1).await.unwrap()[0].id.clone();
        wait_for_status(&h, &next, JobStatus::Completed).await;
    }

    #[tokio::test]
    async fn cancel_queued_job_never_reaches_engine() {
        let h = harness(); // ワーカーを起動しない = 待機中のまま
        let id = h.svc.enqueue(params("later"), 1).await.unwrap()[0]
            .id
            .clone();
        h.svc.cancel(&id).await.unwrap();
        assert_eq!(status_of(&h, &id).await, JobStatus::Cancelled);

        tokio::spawn(h.svc.clone().run_worker());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(h.engine.submitted.lock().unwrap().is_empty());
        // 終了済みは再度取り消せない
        assert_eq!(
            h.svc.cancel(&id).await.unwrap_err().code(),
            "JOB_INVALID_STATE"
        );
        assert_eq!(
            h.svc.cancel("nope").await.unwrap_err().code(),
            "JOB_NOT_FOUND"
        );
    }

    #[tokio::test]
    async fn pause_holds_the_next_job_until_resumed() {
        let h = harness();
        tokio::spawn(h.svc.clone().run_worker());
        h.svc.pause();
        assert!(h.svc.is_paused());
        let id = h.svc.enqueue(params("held"), 1).await.unwrap()[0]
            .id
            .clone();
        tokio::time::sleep(Duration::from_millis(150)).await;
        // 一時停止中は、待機中のまま(エンジンへ送られない)
        assert_eq!(status_of(&h, &id).await, JobStatus::Queued);
        assert!(h.engine.submitted.lock().unwrap().is_empty());

        h.svc.resume();
        assert!(!h.svc.is_paused());
        wait_for_status(&h, &id, JobStatus::Completed).await;
    }

    #[tokio::test]
    async fn remove_only_takes_finished_jobs() {
        let h = harness();
        let id = h.svc.enqueue(params("x"), 1).await.unwrap()[0].id.clone();
        // 待機中は外せない
        assert_eq!(
            h.svc.remove(vec![id.clone()]).await.unwrap_err().code(),
            "VALIDATION"
        );
        h.svc.cancel(&id).await.unwrap();
        assert_eq!(h.svc.remove(vec![id.clone()]).await.unwrap(), 1);
        assert!(
            h.svc
                .list(vec![], None, None)
                .await
                .unwrap()
                .items
                .is_empty()
        );
    }

    #[tokio::test]
    async fn enqueue_rejects_known_models_of_another_kind() {
        let h = harness();
        let mut p = params("mismatch"); // SD 1.5
        p.model.name = "sd_xl_base_1.0.safetensors".into(); // SDXL のファイル
        assert_eq!(h.svc.enqueue(p, 1).await.unwrap_err().code(), "VALIDATION");
        // 手動配置のファイルは断らない
        let mut ok = params("own");
        ok.model.name = "my-own-merge.safetensors".into();
        assert!(h.svc.enqueue(ok, 1).await.is_ok());
    }

    #[tokio::test]
    async fn reorder_changes_execution_order() {
        let h = harness();
        let a = h.svc.enqueue(params("a"), 1).await.unwrap()[0].id.clone();
        let _b = h.svc.enqueue(params("b"), 1).await.unwrap();
        let c = h.svc.enqueue(params("c"), 1).await.unwrap()[0].id.clone();
        h.svc.reorder(&c, 0).await.unwrap(); // c, a, b
        h.svc.reorder(&a, 99).await.unwrap(); // c, b, a

        tokio::spawn(h.svc.clone().run_worker());
        wait_for_status(&h, &a, JobStatus::Completed).await;
        assert_eq!(*h.engine.submitted.lock().unwrap(), ["c", "b", "a"]);
    }

    #[tokio::test]
    async fn enqueue_validates_and_assigns_seeds() {
        let h = harness();
        let mut bad = params("x");
        bad.width = 100;
        assert_eq!(
            h.svc.enqueue(bad, 1).await.unwrap_err().code(),
            "VALIDATION"
        );
        assert_eq!(
            h.svc.enqueue(params("x"), 0).await.unwrap_err().code(),
            "VALIDATION"
        );
        assert_eq!(
            h.svc.enqueue(params("x"), 21).await.unwrap_err().code(),
            "VALIDATION"
        );

        // 固定シード: seed, seed+1, ...
        let seeds = |jobs: &[Job]| -> Vec<u64> {
            jobs.iter()
                .map(|j| j.params["seed"].as_u64().unwrap())
                .collect()
        };
        let jobs = h.svc.enqueue(params("x"), 3).await.unwrap();
        assert_eq!(seeds(&jobs), [42, 43, 44]);
        // ランダムシード: 範囲内で、ジョブごとに異なる
        let mut p = params("x");
        p.seed_randomize = true;
        let jobs = h.svc.enqueue(p, 5).await.unwrap();
        let s = seeds(&jobs);
        assert!(s.iter().all(|v| *v <= MAX_SEED));
        let mut uniq = s.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), 5, "{s:?}");
    }

    #[tokio::test]
    async fn retry_clones_failed_and_rejects_others() {
        let h = harness();
        tokio::spawn(h.svc.clone().run_worker());
        let id = h.svc.enqueue(params("fail"), 1).await.unwrap()[0]
            .id
            .clone();
        wait_for_status(&h, &id, JobStatus::Failed).await;

        let retried = h.svc.retry(&id).await.unwrap();
        assert_ne!(retried.id, id);
        assert_eq!(retried.params["prompt"], "fail");
        assert_eq!(retried.params["seed"], 42);

        let ok = h.svc.enqueue(params("ok"), 1).await.unwrap()[0].id.clone();
        wait_for_status(&h, &ok, JobStatus::Completed).await;
        assert_eq!(
            h.svc.retry(&ok).await.unwrap_err().code(),
            "JOB_INVALID_STATE"
        );
    }

    #[tokio::test]
    async fn recover_fails_interrupted_jobs_and_resumes_queue() {
        let h = harness();
        // 前回のセッションで running のまま残ったジョブ + 待機中のジョブ
        let stuck = h.svc.enqueue(params("stuck"), 1).await.unwrap()[0]
            .id
            .clone();
        h.svc
            .db
            .call({
                let id = stuck.clone();
                move |c| repo::mark_running(c, &id, 1).map(|_| ())
            })
            .await
            .unwrap();
        let waiting = h.svc.enqueue(params("waiting"), 1).await.unwrap()[0]
            .id
            .clone();

        h.svc.recover().await.unwrap();
        assert_eq!(status_of(&h, &stuck).await, JobStatus::Failed);
        let page = h
            .svc
            .list(vec![JobStatus::Failed], None, None)
            .await
            .unwrap();
        assert_eq!(page.items[0].error.as_ref().unwrap().code, "INTERRUPTED");

        tokio::spawn(h.svc.clone().run_worker());
        wait_for_status(&h, &waiting, JobStatus::Completed).await;
    }

    #[tokio::test]
    async fn list_paginates_with_cursor() {
        let h = harness();
        h.svc.enqueue(params("x"), 5).await.unwrap();
        let first = h.svc.list(vec![], None, Some(2)).await.unwrap();
        assert_eq!(first.items.len(), 2);
        let second = h
            .svc
            .list(vec![], first.next_cursor.clone(), Some(2))
            .await
            .unwrap();
        let third = h
            .svc
            .list(vec![], second.next_cursor.clone(), Some(2))
            .await
            .unwrap();
        assert_eq!((second.items.len(), third.items.len()), (2, 1));
        assert!(third.next_cursor.is_none());
        assert_eq!(
            h.svc
                .list(vec![], Some("abc".into()), None)
                .await
                .unwrap_err()
                .code(),
            "VALIDATION"
        );
    }

    #[test]
    fn extension_and_paths_are_safe() {
        assert_eq!(safe_extension("a.PNG"), "png");
        assert_eq!(safe_extension("a.webp"), "webp");
        assert_eq!(safe_extension("noext"), "png");
        assert_eq!(safe_extension("evil.p/ng"), "png");
        assert_eq!(safe_extension("x.toolongext"), "png");
        let p = join_rel(Path::new("/root"), "2026/11/a.png");
        assert!(p.ends_with(Path::new("2026").join("11").join("a.png")));
    }

    // ---- 実機(ComfyUI + SD 1.5)での通し検証 ------------------------------------------------

    struct RealHarness {
        svc: Arc<JobService>,
        events: Arc<Recorder>,
        dir: PathBuf,
    }
    impl Drop for RealHarness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn real_harness() -> RealHarness {
        let url = std::env::var("COMFYUI_URL").unwrap_or_else(|_| "http://127.0.0.1:8188".into());
        let dir = std::env::temp_dir().join(format!("gazooo-real-{}", uuid::Uuid::now_v7()));
        let events = Arc::new(Recorder::default());
        let svc = JobService::new(
            Db::open_in_memory().unwrap(),
            Arc::new(EngineService::new(&url).unwrap()),
            events.clone(),
            AppPaths::new(dir.clone()),
        );
        tokio::spawn(svc.clone().run_worker());
        RealHarness { svc, events, dir }
    }

    fn real_params(steps: u32, size: u32) -> GenerationParams {
        let mut p = sample(ModelKind::Sd15);
        p.model.name = std::env::var("COMFYUI_SD15_MODEL")
            .unwrap_or_else(|_| "v1-5-pruned-emaonly.safetensors".into());
        p.prompt = "a red apple on a wooden table".into();
        p.width = size;
        p.height = size;
        p.steps = steps;
        p
    }

    async fn real_wait(svc: &JobService, id: &str, want: JobStatus, secs: u64) {
        for _ in 0..secs * 10 {
            let page = svc.list(vec![], None, Some(200)).await.unwrap();
            let job = page.items.iter().find(|j| j.id == id).unwrap();
            if job.status == want {
                return;
            }
            assert!(
                !matches!(job.status, JobStatus::Failed) || want == JobStatus::Failed,
                "job failed unexpectedly: {:?}",
                job.error
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("job {id} did not reach {want:?} in {secs}s");
    }

    /// 実機: キューに投入 → 実際に生成 → 画像とサムネイルが保存され、通知される。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with an SD 1.5 checkpoint"]
    async fn real_comfyui_job_end_to_end_saves_image_and_thumbnail() {
        let h = real_harness();
        let id = h.svc.enqueue(real_params(8, 512), 1).await.unwrap()[0]
            .id
            .clone();
        real_wait(&h.svc, &id, JobStatus::Completed, 180).await;

        let images = h.events.images.lock().unwrap().clone();
        assert_eq!(images.len(), 1);
        let img = &images[0];
        assert_eq!(
            &std::fs::read(&img.file_path).unwrap()[..4],
            &[0x89, b'P', b'N', b'G']
        );
        assert_ne!(img.thumb_path, img.file_path, "サムネイルが作られている");
        let thumb = image::load_from_memory(&std::fs::read(&img.thumb_path).unwrap()).unwrap();
        assert_eq!(
            (thumb.width(), thumb.height()),
            (384, 384),
            "512px の画像が 384px に縮小される"
        );
        assert!(
            !h.events.progress.lock().unwrap().is_empty(),
            "進捗イベントが届く"
        );

        let job = &h.svc.list(vec![], None, None).await.unwrap().items[0];
        assert_eq!(job.image_ids, std::slice::from_ref(&img.id));
    }

    /// 実機: 実行中のキャンセルで ComfyUI が実際に中断され、次のジョブが正常に実行できる。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with an SD 1.5 checkpoint"]
    async fn real_comfyui_cancel_interrupts_generation_and_queue_continues() {
        let h = real_harness();
        // 十分に長い生成(多ステップ・大きめのサイズ)を投入し、進捗が出てから取り消す
        let long = h.svc.enqueue(real_params(150, 768), 1).await.unwrap()[0]
            .id
            .clone();
        for _ in 0..1800 {
            if !h.events.progress.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            !h.events.progress.lock().unwrap().is_empty(),
            "生成が始まっていない"
        );

        let cancelled_at = Instant::now();
        h.svc.cancel(&long).await.unwrap();
        real_wait(&h.svc, &long, JobStatus::Cancelled, 30).await;
        println!("cancelled in {:?}", cancelled_at.elapsed());
        assert!(
            h.events.images.lock().unwrap().is_empty(),
            "取り消したジョブの画像は保存されない"
        );

        // 中断後も ComfyUI とワーカーが正常で、次のジョブが完了する
        let next = h.svc.enqueue(real_params(4, 512), 1).await.unwrap()[0]
            .id
            .clone();
        real_wait(&h.svc, &next, JobStatus::Completed, 180).await;
        assert_eq!(h.events.images.lock().unwrap().len(), 1);
    }
}
