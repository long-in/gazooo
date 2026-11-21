//! バックアップの取得とインポート(docs/35)。
//!
//! 設定値・ギャラリー・プリセットの 3 区分(`scope`)を、1 つの ZIP(`.gazooo-backup`)にまとめる。
//! 復元は選んだ区分の置き換えのみ(マージはしない)。復元前の自動バックアップは取らない(Q-219)。

mod export;
mod restore;
pub mod scope;

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::AppPaths;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::services::images::Trasher;
use crate::services::settings;
pub use restore::OrphanReport;
pub use scope::Include;

pub const FORMAT: u32 = 1;
pub const MANIFEST_NAME: &str = "manifest.json";
pub const DB_ENTRY: &str = "db/gazooo.db";
pub const IMAGES_PREFIX: &str = "images/";
/// 利用者が取得・復元を中止したときのコード(フロントでは通知しない)。
pub const CANCELLED: &str = "BACKUP_CANCELLED";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Counts {
    pub settings: u64,
    pub conversations: u64,
    pub jobs: u64,
    pub images: u64,
    pub inputs: u64,
    pub presets: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManifestInclude {
    pub settings: bool,
    pub gallery: bool,
    pub presets: bool,
    pub api_key: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Bytes {
    pub images: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Manifest {
    pub format: u32,
    pub app: String,
    pub app_version: String,
    pub schema_version: u32,
    pub created_at: i64,
    pub include: ManifestInclude,
    pub counts: Counts,
    pub bytes: Bytes,
}

impl Manifest {
    pub fn sections(&self) -> Include {
        Include {
            settings: self.include.settings,
            gallery: self.include.gallery,
            presets: self.include.presets,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub settings: u64,
    pub conversations: u64,
    pub jobs: u64,
    pub images: u64,
    pub presets: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub path: String,
    pub images: u64,
    pub skipped: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreRequest {
    pub include: Include,
    pub restore_api_key: bool,
    pub delete_orphans: bool,
}

/// 進捗の通知先(`backup:progress`)。
pub trait BackupSink: Send + Sync {
    fn progress(&self, phase: &str, done: u64, total: u64);
}

#[cfg(test)]
pub struct NoopSink;
#[cfg(test)]
impl BackupSink for NoopSink {
    fn progress(&self, _: &str, _: u64, _: u64) {}
}

pub(crate) fn map_io(e: std::io::Error) -> AppError {
    if e.kind() == std::io::ErrorKind::StorageFull {
        AppError::backup("BACKUP_NO_SPACE", "保存先の空き容量が足りません")
    } else {
        AppError::Storage(e.to_string())
    }
}

pub(crate) fn map_zip(e: zip::result::ZipError) -> AppError {
    match e {
        zip::result::ZipError::Io(e) => map_io(e),
        other => AppError::internal(format!("ZIP の書き込みに失敗しました: {other}")),
    }
}

/// ZIP・DB に記録された相対パス(`/` 区切り)が、画像保存先の中に収まる安全なものか。
pub(crate) fn safe_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.starts_with('/')
        && rel.split('/').all(|c| {
            !c.is_empty()
                && c != "."
                && c != ".."
                && !c.contains('\\')
                && !c.contains(':')
                && !c.contains('\0')
        })
}

/// `base` に `/` 区切りの相対パスを結合する(安全でなければ `None`)。
pub(crate) fn join_rel(base: &Path, rel: &str) -> Option<PathBuf> {
    if !safe_rel(rel) {
        return None;
    }
    Some(rel.split('/').fold(base.to_path_buf(), |p, c| p.join(c)))
}

struct Inspected {
    src: PathBuf,
    manifest: Manifest,
    staged_db: PathBuf,
}

pub struct BackupService {
    db: Db,
    paths: AppPaths,
    trasher: Arc<dyn Trasher>,
    sink: Arc<dyn BackupSink>,
    app_version: String,
    cancel: AtomicBool,
    busy: AtomicBool,
    inspected: Mutex<Option<Inspected>>,
}

/// 実行中の印を、スコープを抜けるとき(エラー・キャンセルを含む)に必ず外す。
struct BusyGuard<'a>(&'a AtomicBool);
impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl BackupService {
    pub fn new(
        db: Db,
        paths: AppPaths,
        trasher: Arc<dyn Trasher>,
        sink: Arc<dyn BackupSink>,
        app_version: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            paths,
            trasher,
            sink,
            app_version,
            cancel: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            inspected: Mutex::new(None),
        })
    }

    fn staging_dir(&self) -> PathBuf {
        self.paths.data_dir.join("restore-staging")
    }

    fn begin(&self) -> AppResult<BusyGuard<'_>> {
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err(AppError::backup(
                "BACKUP_BUSY",
                "別のバックアップ操作を実行中です",
            ));
        }
        self.cancel.store(false, Ordering::SeqCst);
        Ok(BusyGuard(&self.busy))
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    async fn current_images_dir(&self) -> AppResult<PathBuf> {
        let s = self.db.call(|c| settings::load(c)).await?;
        Ok(self.paths.resolve_images_dir(&s))
    }

    fn lock_inspected(&self) -> AppResult<std::sync::MutexGuard<'_, Option<Inspected>>> {
        self.inspected
            .lock()
            .map_err(|_| AppError::internal("backup state lock poisoned"))
    }

    // ------------------------------------------------------------ 見込み

    pub async fn estimate(&self, include: Include) -> AppResult<Estimate> {
        let images_dir = self.current_images_dir().await?;
        let (est, thumbs, sized) = self
            .db
            .call(|c| {
                let n = |sql: &str| -> AppResult<u64> {
                    Ok(c.query_row(sql, [], |r| r.get::<_, i64>(0))?.max(0) as u64)
                };
                let est = Estimate {
                    settings: n("SELECT COUNT(*) FROM settings")?,
                    conversations: n("SELECT COUNT(*) FROM assistant_conversations")?,
                    jobs: n("SELECT COUNT(*) FROM jobs")?,
                    images: n("SELECT COUNT(*) FROM images")?,
                    presets: n("SELECT COUNT(*) FROM presets")?,
                    bytes: 0,
                };
                let sized = n("SELECT COALESCE(SUM(file_size),0) FROM images")?;
                let mut stmt =
                    c.prepare("SELECT thumb_path FROM images WHERE thumb_path <> file_path")?;
                let thumbs = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((est, thumbs, sized))
            })
            .await?;
        let mut est = est;
        if include.gallery {
            est.bytes = tokio::task::spawn_blocking(move || {
                let mut total = sized;
                for t in thumbs {
                    if let Some(m) = join_rel(&images_dir, &t).and_then(|p| p.metadata().ok()) {
                        total += m.len();
                    }
                }
                for rel in export::list_inputs(&images_dir) {
                    if let Some(m) = join_rel(&images_dir, &rel).and_then(|p| p.metadata().ok()) {
                        total += m.len();
                    }
                }
                total
            })
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
        }
        Ok(est)
    }

    // ------------------------------------------------------------ 取得

    /// 保存先(`dest`)へバックアップを書く。取り消されたときは `None`。
    pub async fn export(
        self: &Arc<Self>,
        include: Include,
        api_key: bool,
        dest: PathBuf,
    ) -> AppResult<Option<ExportReport>> {
        if !include.any() {
            return Err(AppError::validation(
                "include",
                "バックアップに含めるものを 1 つ以上選んでください",
            ));
        }
        let _guard = self.begin()?;
        let images_dir = self.current_images_dir().await?;
        check_dest(&dest, &self.paths.data_dir, &images_dir)?;

        let tmp = self
            .paths
            .data_dir
            .join(format!("backup-tmp-{}.db", uuid::Uuid::new_v4()));
        let snap = tmp.clone();
        let schema: u32 = self
            .db
            .call(move |c| {
                export::snapshot(c, &snap)?;
                Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?)
            })
            .await?;

        let me = Arc::clone(self);
        let (dest2, tmp2) = (dest.clone(), tmp.clone());
        let res = tokio::task::spawn_blocking(move || {
            let sink = Arc::clone(&me.sink);
            let progress = move |p: &str, d: u64, t: u64| sink.progress(p, d, t);
            export::write_backup(&export::WriteParams {
                snapshot: &tmp2,
                schema_version: schema,
                app_version: &me.app_version,
                include,
                api_key,
                images_dir: &images_dir,
                dest: &dest2,
                cancel: &me.cancel,
                progress: &progress,
            })
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()));
        let _ = std::fs::remove_file(&tmp);
        match res? {
            Ok(w) => {
                tracing::info!(images = w.images, skipped = w.skipped, "backup exported");
                Ok(Some(ExportReport {
                    path: dest.to_string_lossy().into_owned(),
                    images: w.images,
                    skipped: w.skipped,
                    bytes: w.bytes,
                }))
            }
            Err(AppError::Backup { code, .. }) if code == CANCELLED => Ok(None),
            Err(e) => Err(e),
        }
    }

    // ------------------------------------------------------------ 検査

    /// バックアップを検査して、復元の対象として保持する。現在のデータは変更しない。
    pub async fn inspect(&self, src: PathBuf) -> AppResult<Manifest> {
        let _guard = self.begin()?;
        let current: u32 = self
            .db
            .call(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?))
            .await?;
        let staging = self.staging_dir();
        let src2 = src.clone();
        let (manifest, staged) =
            tokio::task::spawn_blocking(move || restore::stage(&src2, &staging, current))
                .await
                .map_err(|e| AppError::internal(e.to_string()))??;
        *self.lock_inspected()? = Some(Inspected {
            src,
            manifest: manifest.clone(),
            staged_db: staged,
        });
        Ok(manifest)
    }

    fn take_inspected(&self) -> AppResult<(PathBuf, Manifest, PathBuf)> {
        let g = self.lock_inspected()?;
        let i = g.as_ref().ok_or_else(|| {
            AppError::validation("backup", "先にバックアップファイルを選んでください")
        })?;
        Ok((i.src.clone(), i.manifest.clone(), i.staged_db.clone()))
    }

    /// 復元後の画像保存先(設定値を復元するならバックアップの値、しなければ現在の値)。
    async fn target_images_dir(&self, include: &Include, staged: &Path) -> AppResult<PathBuf> {
        if include.settings {
            let staged = staged.to_path_buf();
            let dir = tokio::task::spawn_blocking(move || restore::staged_images_dir(&staged))
                .await
                .map_err(|e| AppError::internal(e.to_string()))??;
            Ok(dir
                .map(PathBuf::from)
                .unwrap_or_else(|| self.paths.default_images_dir()))
        } else {
            self.current_images_dir().await
        }
    }

    /// 復元した場合に「バックアップに無い画像ファイル」になるものの数と容量。
    pub async fn orphans(&self, include: Include) -> AppResult<OrphanReport> {
        let (_, manifest, staged) = self.take_inspected()?;
        if !include.is_subset_of(&manifest.sections()) {
            return Err(AppError::validation(
                "include",
                "バックアップに含まれない区分は選べません",
            ));
        }
        if !include.gallery {
            return Ok(OrphanReport::default());
        }
        let dir = self.target_images_dir(&include, &staged).await?;
        let (gal, pre) = ("bk", if include.presets { "bk" } else { "main" });
        let found = self
            .db
            .call(move |c| {
                c.execute(
                    "ATTACH DATABASE ?1 AS bk",
                    [staged.to_string_lossy().as_ref()],
                )?;
                let r = restore::scan_orphans(c, gal, pre, &dir);
                let _ = c.execute_batch("DETACH DATABASE bk");
                r
            })
            .await?;
        Ok(OrphanReport {
            count: found.len() as u64,
            bytes: found.iter().map(|(_, n)| n).sum(),
        })
    }

    // ------------------------------------------------------------ 復元

    pub async fn restore(self: &Arc<Self>, req: RestoreRequest) -> AppResult<()> {
        let (src, manifest, staged) = self.take_inspected()?;
        if !req.include.any() {
            return Err(AppError::validation(
                "include",
                "復元する区分を 1 つ以上選んでください",
            ));
        }
        if !req.include.is_subset_of(&manifest.sections()) {
            return Err(AppError::validation(
                "include",
                "バックアップに含まれない区分は選べません",
            ));
        }
        let _guard = self.begin()?;
        let active: i64 = self
            .db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM jobs WHERE status IN ('queued','running')",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await?;
        if active > 0 {
            return Err(AppError::backup(
                "BACKUP_BUSY",
                "実行中・待機中のジョブがあります。停止してから復元してください",
            ));
        }

        // 1. 画像(追加のみ。この段階の失敗・中止は、画像が増えるだけで現在のデータは壊れない)
        if req.include.gallery {
            let target = self.target_images_dir(&req.include, &staged).await?;
            let me = Arc::clone(self);
            let src2 = src.clone();
            tokio::task::spawn_blocking(move || {
                let sink = Arc::clone(&me.sink);
                let progress = move |p: &str, d: u64, t: u64| sink.progress(p, d, t);
                restore::extract_images(&src2, &target, &me.cancel, &progress)
            })
            .await
            .map_err(|e| AppError::internal(e.to_string()))??;
        }

        // 2. DB(1 トランザクション。失敗すればロールバックされ、現在の DB は変わらない)
        self.sink.progress("swap", 0, 1);
        let include = req.include;
        let (restore_key, staged2) = (req.restore_api_key, staged.clone());
        self.db
            .call(move |c| restore::apply(c, &staged2, &include, restore_key))
            .await?;
        self.sink.progress("swap", 1, 1);

        // 3. 復元後の DB が参照しない画像ファイルをゴミ箱へ(選んだときのみ)
        if req.include.gallery && req.delete_orphans {
            let dir = self.target_images_dir(&req.include, &staged).await?;
            let found = self
                .db
                .call(move |c| restore::scan_orphans(c, "main", "main", &dir))
                .await?;
            let trasher = Arc::clone(&self.trasher);
            tokio::task::spawn_blocking(move || {
                for (path, _) in found {
                    if let Err(e) = trasher.trash(&path) {
                        tracing::warn!(error = %e, "orphan image could not be trashed");
                    }
                }
            })
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
        }

        let _ = std::fs::remove_dir_all(self.staging_dir());
        *self.lock_inspected()? = None;
        tracing::info!("backup restored");
        Ok(())
    }
}

/// 保存先がアプリのデータ領域・画像保存先の中なら拒否する(自分自身を含む再帰・復元時の上書きを避ける)。
fn check_dest(dest: &Path, data_dir: &Path, images_dir: &Path) -> AppResult<()> {
    let parent = dest.parent().unwrap_or(dest);
    let parent = std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    for root in [data_dir, images_dir] {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        if parent.starts_with(&root) {
            return Err(AppError::backup(
                "BACKUP_PATH_INVALID",
                "アプリのデータ領域・画像保存先の中には保存できません",
            ));
        }
    }
    // 親が `..` を含むなどの異常な指定
    if dest.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(AppError::backup(
            "BACKUP_PATH_INVALID",
            "保存先のパスが不正です",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::{Read, Write};

    use super::*;

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("gazooo-bk-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct FakeTrash;
    impl Trasher for FakeTrash {
        fn trash(&self, path: &Path) -> std::io::Result<()> {
            fs::remove_file(path)
        }
    }

    const ALL: Include = Include {
        settings: true,
        gallery: true,
        presets: true,
    };

    fn inc(settings: bool, gallery: bool, presets: bool) -> Include {
        Include {
            settings,
            gallery,
            presets,
        }
    }

    fn req(include: Include, restore_api_key: bool, delete_orphans: bool) -> RestoreRequest {
        RestoreRequest {
            include,
            restore_api_key,
            delete_orphans,
        }
    }

    struct Env {
        svc: Arc<BackupService>,
        db: Db,
        images: PathBuf,
        _tmp: TmpDir,
        root: PathBuf,
    }

    fn env() -> Env {
        let tmp = TmpDir::new();
        let root = tmp.path().to_path_buf();
        let data = root.join("data");
        fs::create_dir_all(&data).unwrap();
        let db = Db::open_in_memory().unwrap();
        let paths = AppPaths { data_dir: data };
        let images = paths.default_images_dir();
        let svc = BackupService::new(
            db.clone(),
            paths,
            Arc::new(FakeTrash),
            Arc::new(NoopSink),
            "0.0.0".into(),
        );
        Env {
            svc,
            db,
            images,
            _tmp: tmp,
            root,
        }
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    /// 設定・プリセット・会話・ジョブ・画像(原寸 + サムネイル)の記録を入れる。
    async fn seed(e: &Env, tag: &str) {
        let tag = tag.to_string();
        e.db.call(move |c| {
            c.execute_batch(&format!(
                "INSERT INTO settings(key,value_json,updated_at) VALUES
                   ('ui.theme','\"light\"',1),
                   ('assistant.api_key','\"sk-secret-{tag}\"',1),
                   ('assistant.key_verified','true',1),
                   ('storage.images_dir','null',1);
                 INSERT INTO presets(id,name,params_json,created_at,updated_at)
                   VALUES ('p-{tag}','preset-{tag}','{{\"controlnet\":{{\"image\":\"inputs/in-{tag}.png\"}}}}',1,1);
                 INSERT INTO assistant_conversations(id,title,created_at,updated_at) VALUES ('c-{tag}','t',1,1);
                 INSERT INTO assistant_messages(id,conversation_id,role,text,created_at) VALUES ('m-{tag}','c-{tag}','user','hi',1);
                 INSERT INTO jobs(id,status,position,engine,params_json,prompt_text,model_name,created_at)
                   VALUES ('j-{tag}','completed',0,'comfyui','{{}}','prompt {tag}','m',1);
                 INSERT INTO images(id,job_id,file_path,thumb_path,width,height,seed,file_size,created_at)
                   VALUES ('i-{tag}','j-{tag}','2026/11/i-{tag}.png','thumbs/i-{tag}.jpg',1,1,1,3,1);"
            ))?;
            Ok(())
        })
        .await
        .unwrap();
    }

    /// 待機中のジョブを 1 件足す。
    async fn add_queued(e: &Env, tag: &str) {
        let tag = tag.to_string();
        e.db.call(move |c| {
            c.execute_batch(&format!(
                "INSERT INTO jobs(id,status,position,engine,params_json,prompt_text,model_name,created_at)
                 VALUES ('q-{tag}','queued',1,'comfyui','{{}}','queued','m',2);"
            ))?;
            Ok(())
        })
        .await
        .unwrap();
    }

    fn seed_files(e: &Env, tag: &str) {
        write(&e.images.join(format!("2026/11/i-{tag}.png")), b"png");
        write(&e.images.join(format!("thumbs/i-{tag}.jpg")), b"jpg");
        write(&e.images.join(format!("inputs/in-{tag}.png")), b"inp");
    }

    async fn count(db: &Db, sql: &'static str) -> i64 {
        db.call(move |c| Ok(c.query_row(sql, [], |r| r.get(0))?))
            .await
            .unwrap()
    }

    async fn api_key(db: &Db) -> String {
        db.call(|c| {
            Ok(c.query_row(
                "SELECT value_json FROM settings WHERE key='assistant.api_key'",
                [],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .unwrap()
    }

    fn read_zip_entry(zip: &Path, name: &str) -> Option<Vec<u8>> {
        let mut z = zip::ZipArchive::new(File::open(zip).unwrap()).unwrap();
        let mut f = z.by_name(name).ok()?;
        let mut v = Vec::new();
        f.read_to_end(&mut v).unwrap();
        Some(v)
    }

    fn zip_names(zip: &Path) -> Vec<String> {
        let mut z = zip::ZipArchive::new(File::open(zip).unwrap()).unwrap();
        (0..z.len())
            .map(|i| z.by_index_raw(i).unwrap().name().unwrap().into_owned())
            .collect()
    }

    fn manifest_of(zip: &Path) -> Manifest {
        serde_json::from_slice(&read_zip_entry(zip, MANIFEST_NAME).unwrap()).unwrap()
    }

    fn make_zip(path: &Path, entries: &[(&str, Vec<u8>)]) {
        let mut z = zip::ZipWriter::new(File::create(path).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        for (n, b) in entries {
            z.start_file(*n, o).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }

    #[tokio::test]
    async fn round_trip_restores_all_sections_and_files() {
        let a = env();
        seed(&a, "a").await;
        add_queued(&a, "a").await;
        seed_files(&a, "a");
        let zip = a.root.join("out").join("b.gazooo-backup");
        fs::create_dir_all(zip.parent().unwrap()).unwrap();
        let r = a
            .svc
            .export(ALL, false, zip.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.skipped, 0);

        let m = manifest_of(&zip);
        assert!(m.include.settings && m.include.gallery && m.include.presets);
        assert!(!m.include.api_key);
        assert_eq!(m.counts.jobs, 2);
        assert_eq!(m.counts.images, 1);
        assert_eq!(m.counts.inputs, 1);
        // サムネイルも参照画像も入る
        let names = zip_names(&zip);
        assert!(names.contains(&"images/2026/11/i-a.png".to_string()));
        assert!(names.contains(&"images/thumbs/i-a.jpg".to_string()));
        assert!(names.contains(&"images/inputs/in-a.png".to_string()));
        // API キーは DB のファイルのどこにも残らない(VACUUM で空きページも消える)
        let db_bytes = read_zip_entry(&zip, DB_ENTRY).unwrap();
        assert!(!String::from_utf8_lossy(&db_bytes).contains("sk-secret-a"));

        // 別の環境(自分の API キーとプリセットを持つ)へ復元する
        let b = env();
        seed(&b, "b").await;
        let got = b.svc.inspect(zip.clone()).await.unwrap();
        assert_eq!(got.counts.images, 1);
        b.svc.restore(req(ALL, true, false)).await.unwrap();
        // 置き換え(b の元のデータは消え、a のデータになる)
        assert_eq!(
            count(&b.db, "SELECT COUNT(*) FROM presets WHERE name='preset-a'").await,
            1
        );
        assert_eq!(
            count(&b.db, "SELECT COUNT(*) FROM presets WHERE name='preset-b'").await,
            0
        );
        assert_eq!(
            count(&b.db, "SELECT COUNT(*) FROM images WHERE id='i-a'").await,
            1
        );
        assert_eq!(
            count(
                &b.db,
                "SELECT COUNT(*) FROM assistant_messages WHERE id='m-a'"
            )
            .await,
            1
        );
        // API キーはバックアップに無いので、自分のものが残る
        assert!(api_key(&b.db).await.contains("sk-secret-b"));
        assert_eq!(fs::read(b.images.join("2026/11/i-a.png")).unwrap(), b"png");
        assert_eq!(fs::read(b.images.join("thumbs/i-a.jpg")).unwrap(), b"jpg");
        assert_eq!(fs::read(b.images.join("inputs/in-a.png")).unwrap(), b"inp");
        // 待機中だったジョブは取り消し済みになる(再起動後に勝手に生成しない)
        assert_eq!(
            count(
                &b.db,
                "SELECT COUNT(*) FROM jobs WHERE status IN ('queued','running')"
            )
            .await,
            0
        );
        assert!(!b.svc.staging_dir().exists());
    }

    #[tokio::test]
    async fn api_key_is_included_and_restored_only_when_chosen() {
        let a = env();
        seed(&a, "a").await;
        let zip = a.root.join("k.gazooo-backup");
        let only_settings = inc(true, false, false);
        a.svc
            .export(only_settings, true, zip.clone())
            .await
            .unwrap();
        assert!(manifest_of(&zip).include.api_key);
        let b = env();
        seed(&b, "b").await;
        // 復元しない選択では自分のキーが残る
        b.svc.inspect(zip.clone()).await.unwrap();
        b.svc
            .restore(req(only_settings, false, false))
            .await
            .unwrap();
        assert!(api_key(&b.db).await.contains("sk-secret-b"));
        b.svc.inspect(zip.clone()).await.unwrap();
        b.svc
            .restore(req(only_settings, true, false))
            .await
            .unwrap();
        assert!(api_key(&b.db).await.contains("sk-secret-a"));
    }

    #[tokio::test]
    async fn toggles_leave_unselected_sections_out() {
        let a = env();
        seed(&a, "a").await;
        add_queued(&a, "a").await;
        seed_files(&a, "a");
        for (i, jobs, presets, has_settings, has_images) in [
            (inc(false, true, true), 2, 1, false, true),
            (inc(true, false, true), 0, 1, true, false),
            (inc(true, true, false), 2, 0, true, true),
        ] {
            let zip = a
                .root
                .join(format!("t-{jobs}-{presets}-{has_images}.gazooo-backup"));
            a.svc.export(i, false, zip.clone()).await.unwrap().unwrap();
            let m = manifest_of(&zip);
            assert_eq!(m.counts.jobs, jobs);
            assert_eq!(m.counts.presets, presets);
            assert_eq!(m.counts.settings > 0, has_settings);
            assert_eq!(
                zip_names(&zip).iter().any(|n| n.starts_with("images/")),
                has_images
            );
        }
        let err = a
            .svc
            .export(
                inc(false, false, false),
                false,
                a.root.join("n.gazooo-backup"),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    #[tokio::test]
    async fn partial_restore_changes_only_the_chosen_sections() {
        let a = env();
        seed(&a, "a").await;
        let zip = a.root.join("p.gazooo-backup");
        a.svc.export(ALL, false, zip.clone()).await.unwrap();
        let b = env();
        seed(&b, "b").await;
        seed_files(&b, "b");
        b.svc.inspect(zip).await.unwrap();
        b.svc
            .restore(req(inc(false, false, true), false, false))
            .await
            .unwrap();
        assert_eq!(
            count(&b.db, "SELECT COUNT(*) FROM presets WHERE name='preset-a'").await,
            1
        );
        assert_eq!(
            count(&b.db, "SELECT COUNT(*) FROM jobs WHERE id='j-b'").await,
            1
        );
        assert_eq!(
            count(&b.db, "SELECT COUNT(*) FROM jobs WHERE id='j-a'").await,
            0
        );
        assert_eq!(
            count(
                &b.db,
                "SELECT COUNT(*) FROM assistant_conversations WHERE id='c-b'"
            )
            .await,
            1
        );
        assert!(!b.images.join("2026/11/i-a.png").exists());
        // バックアップに含まれない区分は選べない
        let zip2 = a.root.join("q.gazooo-backup");
        a.svc
            .export(inc(false, false, true), false, zip2.clone())
            .await
            .unwrap();
        b.svc.inspect(zip2).await.unwrap();
        let err = b.svc.restore(req(ALL, false, false)).await.unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    #[tokio::test]
    async fn missing_image_files_are_skipped_not_fatal() {
        let a = env();
        seed(&a, "a").await; // ファイルは作らない
        let zip = a.root.join("m.gazooo-backup");
        let r = a
            .svc
            .export(ALL, false, zip.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.skipped, 2); // 原寸とサムネイル
        assert!(zip.exists());
    }

    #[tokio::test]
    async fn orphan_files_are_trashed_only_when_chosen() {
        let a = env();
        seed(&a, "a").await;
        seed_files(&a, "a");
        let zip = a.root.join("o.gazooo-backup");
        a.svc.export(ALL, false, zip.clone()).await.unwrap();

        let b = env();
        seed_files(&b, "a"); // バックアップが参照するファイル
        write(&b.images.join("2020/01/old.png"), b"old");
        write(&b.images.join("thumbs/old.jpg"), b"old");
        write(&b.images.join("inputs/old.png"), b"old");
        write(&b.images.join("notes.txt"), b"keep"); // 想定外のファイルは触らない
        b.svc.inspect(zip.clone()).await.unwrap();
        let rep = b.svc.orphans(ALL).await.unwrap();
        assert_eq!(rep.count, 3);
        assert_eq!(rep.bytes, 9);

        // OFF のときは残る
        b.svc.restore(req(ALL, false, false)).await.unwrap();
        assert!(b.images.join("2020/01/old.png").exists());

        b.svc.inspect(zip).await.unwrap();
        b.svc.restore(req(ALL, false, true)).await.unwrap();
        assert!(!b.images.join("2020/01/old.png").exists());
        assert!(!b.images.join("thumbs/old.jpg").exists());
        assert!(!b.images.join("inputs/old.png").exists());
        assert!(b.images.join("2026/11/i-a.png").exists());
        assert!(b.images.join("inputs/in-a.png").exists()); // プリセット・ジョブが参照
        assert!(b.images.join("notes.txt").exists());
    }

    #[tokio::test]
    async fn restore_uses_the_backup_images_dir_when_settings_are_restored() {
        let a = env();
        seed(&a, "a").await;
        let custom = a.root.join("custom-images");
        write(&custom.join("2026/11/i-a.png"), b"png");
        let escaped = custom.to_string_lossy().replace('\\', "\\\\");
        let q = format!(
            "UPDATE settings SET value_json = '\"{escaped}\"' WHERE key='storage.images_dir'"
        );
        a.db.call(move |c| {
            c.execute_batch(&q)?;
            Ok(())
        })
        .await
        .unwrap();
        let zip = a.root.join("d.gazooo-backup");
        a.svc.export(ALL, false, zip.clone()).await.unwrap();
        let b = env();
        b.svc.inspect(zip).await.unwrap();
        b.svc.restore(req(ALL, false, false)).await.unwrap();
        // 存在しなかった保存先は作られ、画像はそこへ置かれる
        assert_eq!(fs::read(custom.join("2026/11/i-a.png")).unwrap(), b"png");
        assert!(!b.images.join("2026/11/i-a.png").exists());
    }

    #[tokio::test]
    async fn broken_unsupported_and_unsafe_archives_are_rejected_without_changes() {
        let a = env();
        seed(&a, "a").await;
        let good = a.root.join("g.gazooo-backup");
        a.svc.export(ALL, false, good.clone()).await.unwrap();
        let db_bytes = read_zip_entry(&good, DB_ENTRY).unwrap();
        let manifest = |schema: u32| {
            serde_json::to_vec(&serde_json::json!({
                "format": 1, "app": "gazooo", "appVersion": "9", "schemaVersion": schema,
                "createdAt": 1, "include": {"settings": true, "gallery": true, "presets": true},
                "counts": {}, "bytes": {}
            }))
            .unwrap()
        };
        let b = env();
        seed(&b, "b").await;

        // ZIP ではないファイル
        let junk = b.root.join("junk.gazooo-backup");
        fs::write(&junk, b"not a zip").unwrap();
        assert_eq!(
            b.svc.inspect(junk).await.unwrap_err().code(),
            "BACKUP_CORRUPT"
        );

        // 新しすぎるスキーマ
        let newer = b.root.join("newer.gazooo-backup");
        make_zip(
            &newer,
            &[(MANIFEST_NAME, manifest(99)), (DB_ENTRY, db_bytes.clone())],
        );
        assert_eq!(
            b.svc.inspect(newer).await.unwrap_err().code(),
            "BACKUP_UNSUPPORTED"
        );

        // manifest が無い
        let nomanifest = b.root.join("nm.gazooo-backup");
        make_zip(&nomanifest, &[(DB_ENTRY, db_bytes.clone())]);
        assert_eq!(
            b.svc.inspect(nomanifest).await.unwrap_err().code(),
            "BACKUP_UNSUPPORTED"
        );

        // DB が壊れている
        let bad_db = b.root.join("bd.gazooo-backup");
        make_zip(
            &bad_db,
            &[(MANIFEST_NAME, manifest(3)), (DB_ENTRY, vec![7u8; 4096])],
        );
        assert_eq!(
            b.svc.inspect(bad_db).await.unwrap_err().code(),
            "BACKUP_CORRUPT"
        );

        // Zip Slip
        for evil in [
            "images/../evil.txt",
            "images//abs.png",
            "images/C:/x.png",
            "images/a\\b.png",
        ] {
            let z = b.root.join("evil.gazooo-backup");
            make_zip(
                &z,
                &[
                    (MANIFEST_NAME, manifest(3)),
                    (DB_ENTRY, db_bytes.clone()),
                    (evil, b"x".to_vec()),
                ],
            );
            assert_eq!(
                b.svc.inspect(z).await.unwrap_err().code(),
                "BACKUP_CORRUPT",
                "{evil}"
            );
        }
        assert!(!b.root.join("evil.txt").exists());
        // 検査に失敗しても現在のデータは変わらない。検査していない状態での復元もできない
        assert_eq!(
            count(&b.db, "SELECT COUNT(*) FROM presets WHERE name='preset-b'").await,
            1
        );
        let err = b.svc.restore(req(ALL, false, false)).await.unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }

    #[tokio::test]
    async fn failed_swap_rolls_back_to_the_current_data() {
        let b = env();
        seed(&b, "b").await;
        // 現在の `presets` の NOT NULL に違反する行を持つ、壊れたステージング
        let staged = b.root.join("staged.db");
        {
            let c = rusqlite::Connection::open(&staged).unwrap();
            c.execute_batch(
                "CREATE TABLE presets(id, name, params_json, created_at, updated_at);
                 INSERT INTO presets VALUES ('x', NULL, '{}', 1, 1);",
            )
            .unwrap();
        }
        let include = inc(false, false, true);
        let err =
            b.db.call(move |c| restore::apply(c, &staged, &include, false))
                .await
                .unwrap_err();
        assert_eq!(err.code(), "DB_ERROR");
        assert_eq!(
            count(&b.db, "SELECT COUNT(*) FROM presets WHERE name='preset-b'").await,
            1
        );
        // ATTACH したままにしない
        assert_eq!(
            count(
                &b.db,
                "SELECT COUNT(*) FROM pragma_database_list WHERE name='bk'"
            )
            .await,
            0
        );
    }

    #[tokio::test]
    async fn restore_is_refused_while_jobs_are_active() {
        let a = env();
        seed(&a, "a").await;
        let zip = a.root.join("j.gazooo-backup");
        a.svc.export(ALL, false, zip.clone()).await.unwrap();
        let b = env();
        seed(&b, "b").await;
        add_queued(&b, "b").await;
        b.svc.inspect(zip).await.unwrap();
        let err = b.svc.restore(req(ALL, false, false)).await.unwrap_err();
        assert_eq!(err.code(), "BACKUP_BUSY");
        assert_eq!(
            count(&b.db, "SELECT COUNT(*) FROM jobs WHERE id='j-b'").await,
            1
        );
    }

    #[tokio::test]
    async fn destination_inside_app_data_is_rejected() {
        let a = env();
        seed(&a, "a").await;
        fs::create_dir_all(&a.images).unwrap();
        for dest in [
            a.images.join("x.gazooo-backup"),
            a.root.join("data").join("x.gazooo-backup"),
        ] {
            let err = a.svc.export(ALL, false, dest.clone()).await.unwrap_err();
            assert_eq!(err.code(), "BACKUP_PATH_INVALID");
            assert!(!dest.exists());
        }
    }

    #[tokio::test]
    async fn cancelled_export_leaves_no_file() {
        let a = env();
        seed(&a, "a").await;
        seed_files(&a, "a");
        let dest = a.root.join("c.gazooo-backup");
        let snap = a.root.join("snap.db");
        a.db.call({
            let snap = snap.clone();
            move |c| export::snapshot(c, &snap)
        })
        .await
        .unwrap();
        let cancel = AtomicBool::new(true);
        let err = export::write_backup(&export::WriteParams {
            snapshot: &snap,
            schema_version: 3,
            app_version: "0",
            include: ALL,
            api_key: false,
            images_dir: &a.images,
            dest: &dest,
            cancel: &cancel,
            progress: &|_, _, _| {},
        })
        .unwrap_err();
        assert_eq!(err.code(), CANCELLED);
        assert!(!dest.exists());
    }

    #[test]
    fn safe_rel_rejects_escapes() {
        for ok in ["a.png", "2026/11/a.png", "thumbs/a.jpg"] {
            assert!(safe_rel(ok), "{ok}");
        }
        for bad in [
            "", "/a", "../a", "a/../b", "a//b", "a\\b", "C:/a", "./a", "a/.",
        ] {
            assert!(!safe_rel(bad), "{bad}");
        }
    }

    #[test]
    fn manifest_reads_with_missing_fields() {
        let m: Manifest =
            serde_json::from_str(r#"{"format":1,"app":"gazooo","schemaVersion":3}"#).unwrap();
        assert_eq!(m.counts, Counts::default());
        assert!(!m.include.settings);
    }
}
