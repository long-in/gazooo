//! ギャラリー(画像の一覧・詳細・お気に入り・削除・エクスポート・サムネイル補完)。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use crate::AppPaths;
use crate::db::Db;
use crate::db::images::{self as repo, ImageFilter, ImageScope};
use crate::db::jobs::{self as job_repo, ImageRow};
use crate::domain::job::{ImageItem, Job, Page};
use crate::error::{AppError, AppResult};
use crate::services::events::EventSink;
use crate::services::jobs::{build_job, image_item, join_rel};
use crate::services::settings;
use crate::services::thumbs::{make_thumbnail, thumb_rel_path};

pub const DEFAULT_PAGE_SIZE: u32 = 60;
pub const MAX_PAGE_SIZE: u32 = 200;
/// 一度に移動できる画像数の上限(docs/40 §7)。
pub const MAX_MOVE_IDS: usize = 500;
/// お気に入り以外の一括削除で、1 回に処理する件数(docs/49 §5)。
const DELETE_BATCH: i64 = 500;
/// 一括ダウンロードで一度に保存できる画像数の上限(docs/44 §3)。
pub const MAX_EXPORT_IDS: usize = 1000;
const EXPORT_TMP_EXT: &str = "gazooo-tmp";

/// 一括ダウンロードの結果(docs/44 §4)。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExportManyResult {
    pub saved: usize,
    pub failed: Vec<ExportFailure>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportFailure {
    pub id: String,
    pub error: String,
}

/// ファイルをゴミ箱へ移動する(テストで差し替えられるようトレイトにしている)。
pub trait Trasher: Send + Sync {
    fn trash(&self, path: &Path) -> std::io::Result<()>;
}

/// OS のゴミ箱へ移動する(Windows / macOS)。
pub struct OsTrash;

impl Trasher for OsTrash {
    fn trash(&self, path: &Path) -> std::io::Result<()> {
        trash::delete(path).map_err(std::io::Error::other)
    }
}

pub struct ImageService {
    db: Db,
    sink: Arc<dyn EventSink>,
    paths: AppPaths,
    trasher: Arc<dyn Trasher>,
}

#[derive(Serialize)]
pub struct ImageDetail {
    pub image: ImageItem,
    pub job: Job,
}

/// カーソル(`<created_at>:<id>`)。クライアントには不透明な文字列として渡す。
fn encode_cursor(row: &ImageRow) -> String {
    format!("{}:{}", row.created_at, row.id)
}

fn decode_cursor(cursor: &str) -> AppResult<(i64, String)> {
    cursor
        .split_once(':')
        .and_then(|(t, id)| Some((t.parse::<i64>().ok()?, id.to_owned())))
        .filter(|(_, id)| !id.is_empty())
        .ok_or_else(|| AppError::validation("cursor", "cursor が不正です"))
}

impl ImageService {
    pub fn new(
        db: Db,
        sink: Arc<dyn EventSink>,
        paths: AppPaths,
        trasher: Arc<dyn Trasher>,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            sink,
            paths,
            trasher,
        })
    }

    async fn root(&self) -> AppResult<PathBuf> {
        let s = self.db.call(|c| settings::load(c)).await?;
        Ok(self.paths.resolve_images_dir(&s))
    }

    async fn require(&self, id: &str) -> AppResult<ImageRow> {
        let owned = id.to_owned();
        self.db
            .call(move |c| repo::get(c, &owned))
            .await?
            .ok_or_else(|| AppError::ImageNotFound(id.to_owned()))
    }

    pub async fn list(
        &self,
        query: Option<String>,
        favorites_only: bool,
        scope: ImageScope,
        cursor: Option<String>,
        limit: Option<u32>,
    ) -> AppResult<Page<ImageItem>> {
        let limit = i64::from(limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE));
        let after = cursor.as_deref().map(decode_cursor).transpose()?;
        let root = self.root().await?;
        if let ImageScope::Project(id) = scope.clone() {
            let owned = id.clone();
            if !self
                .db
                .call(move |c| crate::db::projects::exists(c, &owned))
                .await?
            {
                return Err(AppError::ProjectNotFound(id));
            }
        }
        let filter = ImageFilter {
            scope,
            query,
            favorites_only,
        };
        let mut rows = self
            .db
            .call(move |c| {
                let after = after.as_ref().map(|(t, id)| (*t, id.as_str()));
                repo::list(c, &filter, after, limit + 1)
            })
            .await?;
        let next_cursor =
            (rows.len() as i64 > limit).then(|| encode_cursor(&rows[limit as usize - 1]));
        rows.truncate(limit as usize);
        Ok(Page {
            items: rows.iter().map(|r| image_item(&root, r)).collect(),
            next_cursor,
        })
    }

    pub async fn get(&self, id: &str) -> AppResult<ImageDetail> {
        let root = self.root().await?;
        let row = self.require(id).await?;
        let job_id = row.job_id.clone();
        let job = self
            .db
            .call(move |c| {
                let job = job_repo::get(c, &job_id)?
                    .ok_or_else(|| AppError::JobNotFound(job_id.clone()))?;
                build_job(c, &job)
            })
            .await?;
        Ok(ImageDetail {
            image: image_item(&root, &row),
            job,
        })
    }

    /// 画像の所属プロジェクトを変える(`None` = 未分類)。DB のみの更新でファイルは動かさない。
    pub async fn move_to_project(
        &self,
        ids: Vec<String>,
        project_id: Option<String>,
    ) -> AppResult<()> {
        if ids.len() > MAX_MOVE_IDS {
            return Err(AppError::validation(
                "ids",
                format!("一度に移動できるのは {MAX_MOVE_IDS} 件までです"),
            ));
        }
        let target = project_id.clone();
        let moved = self
            .db
            .call(move |c| {
                if let Some(pid) = target.as_deref()
                    && !crate::db::projects::exists(c, pid)?
                {
                    return Err(AppError::ProjectNotFound(pid.to_owned()));
                }
                repo::move_to_project(c, &ids, target.as_deref())
            })
            .await?;
        if !moved.is_empty() {
            self.sink.image_moved(&moved, project_id.as_deref());
            self.sink.project_changed();
        }
        Ok(())
    }

    pub async fn set_favorite(&self, id: &str, value: bool) -> AppResult<()> {
        let owned = id.to_owned();
        let found = self
            .db
            .call(move |c| repo::set_favorite(c, &owned, value))
            .await?;
        if found {
            Ok(())
        } else {
            Err(AppError::ImageNotFound(id.to_owned()))
        }
    }

    /// 記録を削除し、`delete_files` ならファイル(原寸・サムネイル)を OS のゴミ箱へ移す。
    pub async fn delete(&self, ids: Vec<String>, delete_files: bool) -> AppResult<()> {
        let (_, failed) = self.delete_inner(ids, delete_files).await?;
        if failed > 0 {
            return Err(AppError::Storage(format!(
                "{failed} 件のファイルをゴミ箱へ移動できませんでした(記録は削除済みです)"
            )));
        }
        Ok(())
    }

    /// 削除の本体。`(削除した記録の数, ゴミ箱へ移せなかったファイルの数)` を返す。
    async fn delete_inner(
        &self,
        ids: Vec<String>,
        delete_files: bool,
    ) -> AppResult<(usize, usize)> {
        let root = self.root().await?;
        let removed = self
            .db
            .call(move |c| {
                let rows = repo::get_many(c, &ids)?;
                let remove_ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
                repo::delete(c, &remove_ids)?;
                Ok(rows)
            })
            .await?;
        if removed.is_empty() {
            return Ok((0, 0));
        }

        let mut failed = 0usize;
        if delete_files {
            for row in &removed {
                let mut files = vec![join_rel(&root, &row.file_path)];
                if row.thumb_path != row.file_path {
                    files.push(join_rel(&root, &row.thumb_path));
                }
                for path in files {
                    // 既に無いファイルはエラーにしない
                    if !path.exists() {
                        continue;
                    }
                    let trasher = Arc::clone(&self.trasher);
                    let p = path.clone();
                    let res = tokio::task::spawn_blocking(move || trasher.trash(&p))
                        .await
                        .map_err(|e| AppError::internal(e.to_string()))?;
                    if let Err(e) = res {
                        failed += 1;
                        tracing::warn!(path = %path.display(), error = %e, "failed to move file to trash");
                    }
                }
            }
        }

        let count = removed.len();
        let ids: Vec<String> = removed.into_iter().map(|r| r.id).collect();
        self.sink.image_deleted(&ids);
        Ok((count, failed))
    }

    async fn require_scope(&self, scope: &ImageScope) -> AppResult<()> {
        if let ImageScope::Project(id) = scope {
            let owned = id.clone();
            if !self
                .db
                .call(move |c| crate::db::projects::exists(c, &owned))
                .await?
            {
                return Err(AppError::ProjectNotFound(id.clone()));
            }
        }
        Ok(())
    }

    /// スコープ内の `(お気に入り以外の件数, お気に入りの件数)`(docs/49)。
    pub async fn count_non_favorite(&self, scope: ImageScope) -> AppResult<(i64, i64)> {
        self.require_scope(&scope).await?;
        self.db
            .call(move |c| repo::count_by_favorite(c, &scope))
            .await
    }

    /// スコープ内のお気に入り以外をすべて削除する(ファイルはゴミ箱へ。docs/49)。
    /// 実数が `expected` を超えていたら何も削除せず `StaleCount`。`(削除数, 失敗したファイル数)`。
    pub async fn delete_non_favorite(
        &self,
        scope: ImageScope,
        expected: i64,
    ) -> AppResult<(usize, usize)> {
        let (total, _) = self.count_non_favorite(scope.clone()).await?;
        if total > expected {
            return Err(AppError::StaleCount);
        }
        let (mut deleted, mut failed) = (0usize, 0usize);
        loop {
            let s = scope.clone();
            let ids = self
                .db
                .call(move |c| repo::non_favorite_ids(c, &s, DELETE_BATCH))
                .await?;
            if ids.is_empty() {
                break;
            }
            let (n, f) = self.delete_inner(ids, true).await?;
            if n == 0 {
                break; // 進まないときの無限ループ防止
            }
            deleted += n;
            failed += f;
        }
        Ok((deleted, failed))
    }

    /// 原寸の画像を `dest`(保存ダイアログで選んだ絶対パス)へコピーする。
    pub async fn export(&self, id: &str, dest: &Path) -> AppResult<()> {
        if !dest.is_absolute() {
            return Err(AppError::validation(
                "destPath",
                "保存先は絶対パスで指定してください",
            ));
        }
        let src = self.original_path(id).await?;
        if src == dest {
            return Err(AppError::validation(
                "destPath",
                "元のファイルと同じ場所には保存できません",
            ));
        }
        let dest = dest.to_path_buf();
        tokio::task::spawn_blocking(move || std::fs::copy(&src, &dest).map(|_| ()))
            .await
            .map_err(|e| AppError::internal(e.to_string()))??;
        Ok(())
    }

    /// 複数の原寸画像を `dest`(フォルダ)へ保存する(docs/44 §3)。名前は元のファイル名で、
    /// 同名は連番にして上書きしない。1 枚の失敗で止めない。`cancel` が立つと未着手を処理しない。
    pub async fn export_many(
        &self,
        ids: Vec<String>,
        dest: PathBuf,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        progress: impl Fn(usize, usize) + Send + 'static,
    ) -> AppResult<ExportManyResult> {
        use std::sync::atomic::Ordering;
        if ids.is_empty() {
            return Err(AppError::validation("ids", "画像を選んでください"));
        }
        if ids.len() > MAX_EXPORT_IDS {
            return Err(AppError::validation(
                "ids",
                format!("一度に保存できるのは {MAX_EXPORT_IDS} 枚までです"),
            ));
        }
        if !dest.is_absolute() || !dest.is_dir() {
            return Err(AppError::validation("dest", "保存先のフォルダが不正です"));
        }
        let root = self.root().await?;
        // 管理領域の内側への保存は拒否する(自分自身へのコピーを防ぐ)
        if let (Ok(r), Ok(d)) = (std::fs::canonicalize(&root), std::fs::canonicalize(&dest))
            && d.starts_with(&r)
        {
            return Err(AppError::validation(
                "dest",
                "ギャラリーの保存先の内側には保存できません",
            ));
        }
        let mut rows = Vec::with_capacity(ids.len());
        for id in &ids {
            rows.push((id.clone(), self.require(id).await.ok()));
        }
        let total = rows.len();
        tokio::task::spawn_blocking(move || {
            let mut result = ExportManyResult::default();
            let mut reserved = std::collections::HashSet::new();
            for (i, (id, row)) in rows.into_iter().enumerate() {
                if cancel.load(Ordering::SeqCst) {
                    result.cancelled = true;
                    break;
                }
                let one = (|| -> Result<(), String> {
                    let row = row.ok_or("画像の記録がありません")?;
                    let src = join_rel(&root, &row.file_path);
                    if !src.is_file() {
                        return Err("元のファイルがありません".into());
                    }
                    let stem = src
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("image")
                        .to_owned();
                    let ext = src
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("png")
                        .to_owned();
                    let out = crate::services::convert::choose_output(
                        &dest,
                        &stem,
                        &ext,
                        false,
                        &src,
                        &mut reserved,
                    );
                    let tmp = out.with_extension(format!("{ext}.{EXPORT_TMP_EXT}"));
                    let copied =
                        std::fs::copy(&src, &tmp).and_then(|_| std::fs::rename(&tmp, &out));
                    if let Err(e) = copied {
                        let _ = std::fs::remove_file(&tmp);
                        return Err(e.to_string());
                    }
                    Ok(())
                })();
                match one {
                    Ok(()) => result.saved += 1,
                    Err(error) => result.failed.push(ExportFailure { id, error }),
                }
                progress(i + 1, total);
            }
            result
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))
    }

    /// 原寸ファイルの絶対パス(存在確認つき)。「フォルダで表示」に使う。
    pub async fn original_path(&self, id: &str) -> AppResult<PathBuf> {
        let root = self.root().await?;
        let row = self.require(id).await?;
        let path = join_rel(&root, &row.file_path);
        if !path.is_file() {
            return Err(AppError::ImageNotFound(format!(
                "ファイルがありません: {}",
                path.display()
            )));
        }
        Ok(path)
    }

    /// サムネイル未生成の画像(M2 以前に保存したもの等)にサムネイルを作る。作れた件数を返す。
    pub async fn backfill_thumbnails(&self) -> AppResult<usize> {
        let root = self.root().await?;
        let rows = self.db.call(|c| repo::needing_thumbs(c)).await?;
        let mut made = 0;
        for row in rows {
            let src = join_rel(&root, &row.file_path);
            let rel = thumb_rel_path(&row.id);
            let dst = join_rel(&root, &rel);
            let ok = tokio::task::spawn_blocking(move || -> bool {
                let Ok(bytes) = std::fs::read(&src) else {
                    return false;
                };
                let Some(thumb) = make_thumbnail(&bytes) else {
                    return false;
                };
                if let Some(parent) = dst.parent()
                    && std::fs::create_dir_all(parent).is_err()
                {
                    return false;
                }
                std::fs::write(&dst, thumb).is_ok()
            })
            .await
            .unwrap_or(false);
            if ok {
                let id = row.id.clone();
                self.db
                    .call(move |c| repo::set_thumb_path(c, &id, &rel))
                    .await?;
                made += 1;
            }
        }
        Ok(made)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::jobs::insert_image;
    use crate::domain::job::ProgressEvent;
    use crate::services::thumbs::png_bytes;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeTrash {
        trashed: Mutex<Vec<PathBuf>>,
        fail: bool,
    }
    impl Trasher for FakeTrash {
        fn trash(&self, path: &Path) -> std::io::Result<()> {
            if self.fail {
                return Err(std::io::Error::other("nope"));
            }
            self.trashed.lock().unwrap().push(path.to_path_buf());
            std::fs::remove_file(path) // 本物のゴミ箱の代わりに削除して、移動された状態を再現する
        }
    }

    #[derive(Default)]
    struct Recorder {
        deleted: Mutex<Vec<Vec<String>>>,
        moved: Mutex<Vec<(Vec<String>, Option<String>)>>,
        project_changes: Mutex<u32>,
    }
    impl EventSink for Recorder {
        fn job_updated(&self, _: &Job) {}
        fn job_progress(&self, _: &ProgressEvent) {}
        fn image_created(&self, _: &ImageItem) {}
        fn image_moved(&self, ids: &[String], project_id: Option<&str>) {
            self.moved
                .lock()
                .unwrap()
                .push((ids.to_vec(), project_id.map(str::to_owned)));
        }
        fn project_changed(&self) {
            *self.project_changes.lock().unwrap() += 1;
        }
        fn image_deleted(&self, ids: &[String]) {
            self.deleted.lock().unwrap().push(ids.to_vec());
        }
    }

    struct Harness {
        svc: Arc<ImageService>,
        db: Db,
        trash: Arc<FakeTrash>,
        events: Arc<Recorder>,
        root: PathBuf,
        dir: PathBuf,
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn harness(trash_fails: bool) -> Harness {
        let dir = std::env::temp_dir().join(format!("gazooo-images-{}", uuid::Uuid::now_v7()));
        let db = Db::open_in_memory().unwrap();
        let trash = Arc::new(FakeTrash {
            fail: trash_fails,
            ..Default::default()
        });
        let events = Arc::new(Recorder::default());
        let paths = AppPaths::new(dir.clone());
        let root = dir.join("images");
        let svc = ImageService::new(db.clone(), events.clone(), paths, trash.clone());
        Harness {
            svc,
            db,
            trash,
            events,
            root,
            dir,
        }
    }

    /// ジョブ 1 件と、実ファイルを持つ画像を作る(サムネイルは原寸と同じパス = 未生成)。
    async fn add_image(h: &Harness, id: &str, created_at: i64, png: Vec<u8>) {
        let rel = format!("2026/11/{id}.png");
        let abs = join_rel(&h.root, &rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, &png).unwrap();
        let (id, size) = (id.to_owned(), png.len() as u64);
        h.db.call(move |c| {
            if crate::db::jobs::get(c, "job")?.is_none() {
                crate::db::jobs::insert(c, "job", r#"{"prompt":"a cat"}"#, "a cat", "m", 0)?;
            }
            insert_image(
                c,
                &ImageRow {
                    id,
                    job_id: "job".into(),
                    file_path: rel.clone(),
                    thumb_path: rel,
                    width: 64,
                    height: 32,
                    seed: 7,
                    file_size: size,
                    is_favorite: false,
                    created_at,
                    project_id: None,
                },
            )
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn list_returns_absolute_paths_with_cursor_paging() {
        let h = harness(false);
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            add_image(&h, id, i as i64, png_bytes(8, 8)).await;
        }
        let p1 = h
            .svc
            .list(None, false, ImageScope::All, None, Some(2))
            .await
            .unwrap();
        assert_eq!(
            p1.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["c", "b"]
        );
        assert!(Path::new(&p1.items[0].file_path).is_absolute());
        assert!(Path::new(&p1.items[0].file_path).is_file());

        let p2 = h
            .svc
            .list(None, false, ImageScope::All, p1.next_cursor, Some(2))
            .await
            .unwrap();
        assert_eq!(p2.items.len(), 1);
        assert!(p2.next_cursor.is_none());
        assert_eq!(
            h.svc
                .list(None, false, ImageScope::All, Some("garbage".into()), None)
                .await
                .unwrap_err()
                .code(),
            "VALIDATION"
        );
        assert_eq!(
            h.svc
                .list(None, false, ImageScope::All, Some("x:".into()), None)
                .await
                .unwrap_err()
                .code(),
            "VALIDATION"
        );
    }

    #[tokio::test]
    async fn move_and_scoped_list_work_across_projects() {
        let h = harness(false);
        h.db.call(|c| {
            crate::db::projects::insert(c, "p1", "A", None, 1)?;
            crate::db::projects::insert(c, "p2", "B", None, 2)
        })
        .await
        .unwrap();
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            add_image(&h, id, i as i64, png_bytes(8, 8)).await;
        }
        let ids = |p: Page<ImageItem>| p.items.into_iter().map(|i| i.id).collect::<Vec<_>>();
        let list = |scope: ImageScope, fav: bool| {
            let svc = h.svc.clone();
            async move { svc.list(None, fav, scope, None, None).await }
        };

        h.svc
            .move_to_project(
                vec!["a".into(), "ghost".into(), "b".into()],
                Some("p1".into()),
            )
            .await
            .unwrap();
        assert_eq!(
            ids(list(ImageScope::Project("p1".into()), false).await.unwrap()),
            ["b", "a"]
        );
        assert_eq!(
            ids(list(ImageScope::Unassigned, false).await.unwrap()),
            ["c"]
        );
        assert_eq!(
            ids(list(ImageScope::All, false).await.unwrap()),
            ["c", "b", "a"]
        );
        // 通知には実際に動いた画像だけが入る
        assert_eq!(
            h.events.moved.lock().unwrap().clone(),
            [(
                vec!["a".to_string(), "b".to_string()],
                Some("p1".to_string())
            )]
        );
        // 画像の項目に所属が載る
        let item = h.svc.get("a").await.unwrap().image;
        assert_eq!(item.project_id.as_deref(), Some("p1"));

        // お気に入りは階層なし(全プロジェクト横断)
        h.svc.set_favorite("a", true).await.unwrap();
        h.svc.set_favorite("c", true).await.unwrap();
        assert_eq!(ids(list(ImageScope::All, true).await.unwrap()), ["c", "a"]);
        assert_eq!(
            ids(list(ImageScope::Project("p1".into()), true).await.unwrap()),
            ["a"]
        );

        // 未分類へ戻す
        h.svc.move_to_project(vec!["a".into()], None).await.unwrap();
        assert_eq!(
            ids(list(ImageScope::Unassigned, false).await.unwrap()),
            ["c", "a"]
        );

        // エラー: 存在しないプロジェクト・件数超過
        assert_eq!(
            h.svc
                .move_to_project(vec!["a".into()], Some("nope".into()))
                .await
                .unwrap_err()
                .code(),
            "PROJECT_NOT_FOUND"
        );
        assert_eq!(
            list(ImageScope::Project("nope".into()), false)
                .await
                .unwrap_err()
                .code(),
            "PROJECT_NOT_FOUND"
        );
        let many: Vec<String> = (0..=MAX_MOVE_IDS).map(|i| i.to_string()).collect();
        assert_eq!(
            h.svc.move_to_project(many, None).await.unwrap_err().code(),
            "VALIDATION"
        );
        // 画像ファイルは動いていない
        assert!(h.root.join("a").exists() || h.svc.original_path("a").await.is_ok());
    }

    #[tokio::test]
    async fn get_returns_image_with_job_params_and_favorite_toggles() {
        let h = harness(false);
        add_image(&h, "a", 1, png_bytes(8, 8)).await;
        let d = h.svc.get("a").await.unwrap();
        assert_eq!(d.image.id, "a");
        assert_eq!(d.job.params["prompt"], "a cat");
        assert_eq!(d.job.image_ids, ["a"]);

        h.svc.set_favorite("a", true).await.unwrap();
        assert!(h.svc.get("a").await.unwrap().image.is_favorite);
        assert_eq!(
            h.svc
                .list(None, true, ImageScope::All, None, None)
                .await
                .unwrap()
                .items
                .len(),
            1
        );
        assert_eq!(
            h.svc.set_favorite("nope", true).await.unwrap_err().code(),
            "IMAGE_NOT_FOUND"
        );
        assert_eq!(
            h.svc.get("nope").await.err().unwrap().code(),
            "IMAGE_NOT_FOUND"
        );
    }

    #[tokio::test]
    async fn delete_moves_files_to_trash_and_notifies() {
        let h = harness(false);
        add_image(&h, "a", 1, png_bytes(8, 8)).await;
        add_image(&h, "b", 2, png_bytes(8, 8)).await;
        let a_path = join_rel(&h.root, "2026/11/a.png");

        h.svc
            .delete(vec!["a".into(), "ghost".into()], true)
            .await
            .unwrap();
        assert!(!a_path.exists());
        assert_eq!(h.trash.trashed.lock().unwrap().len(), 1);
        assert_eq!(
            h.svc
                .list(None, false, ImageScope::All, None, None)
                .await
                .unwrap()
                .items
                .len(),
            1
        );
        assert_eq!(*h.events.deleted.lock().unwrap(), [vec!["a".to_string()]]);
    }

    #[tokio::test]
    async fn delete_without_files_keeps_them_and_tolerates_missing_files() {
        let h = harness(false);
        add_image(&h, "a", 1, png_bytes(8, 8)).await;
        add_image(&h, "b", 2, png_bytes(8, 8)).await;
        h.svc.delete(vec!["a".into()], false).await.unwrap();
        assert!(
            join_rel(&h.root, "2026/11/a.png").exists(),
            "deleteFiles=false ではファイルを残す"
        );
        assert!(h.trash.trashed.lock().unwrap().is_empty());

        std::fs::remove_file(join_rel(&h.root, "2026/11/b.png")).unwrap();
        h.svc.delete(vec!["b".into()], true).await.unwrap(); // 既に無いファイルでもエラーにしない
    }

    #[tokio::test]
    async fn trash_failure_is_reported_after_record_removal() {
        let h = harness(true);
        add_image(&h, "a", 1, png_bytes(8, 8)).await;
        let err = h.svc.delete(vec!["a".into()], true).await.unwrap_err();
        assert_eq!(err.code(), "STORAGE_ERROR");
        assert!(
            h.svc
                .list(None, false, ImageScope::All, None, None)
                .await
                .unwrap()
                .items
                .is_empty()
        );
        assert_eq!(
            h.events.deleted.lock().unwrap().len(),
            1,
            "記録は消えているので通知する"
        );
    }

    #[tokio::test]
    async fn delete_non_favorite_keeps_favorites_and_counts() {
        let h = harness(false);
        for (i, id) in ["a", "b", "c", "d"].iter().enumerate() {
            add_image(&h, id, i as i64 + 1, png_bytes(8, 8)).await;
        }
        h.svc.set_favorite("b", true).await.unwrap();
        h.svc.set_favorite("d", true).await.unwrap();

        let (total, favs) = h.svc.count_non_favorite(ImageScope::All).await.unwrap();
        assert_eq!((total, favs), (2, 2));

        let (deleted, failed) = h.svc.delete_non_favorite(ImageScope::All, 2).await.unwrap();
        assert_eq!((deleted, failed), (2, 0));
        let left: Vec<String> = h
            .svc
            .list(None, false, ImageScope::All, None, None)
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(left.len(), 2);
        assert!(left.contains(&"b".to_string()) && left.contains(&"d".to_string()));
        assert_eq!(h.trash.trashed.lock().unwrap().len(), 2);
        assert!(join_rel(&h.root, "2026/11/b.png").exists());
    }

    #[tokio::test]
    async fn delete_non_favorite_refuses_when_count_grew() {
        let h = harness(false);
        add_image(&h, "a", 1, png_bytes(8, 8)).await;
        add_image(&h, "b", 2, png_bytes(8, 8)).await;
        let err = h
            .svc
            .delete_non_favorite(ImageScope::All, 1)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "STALE_COUNT");
        assert_eq!(
            h.svc.count_non_favorite(ImageScope::All).await.unwrap().0,
            2
        );
    }

    #[tokio::test]
    async fn delete_non_favorite_handles_many_batches_and_missing_files() {
        let h = harness(false);
        let n = (DELETE_BATCH as usize) + 7;
        for i in 0..n {
            add_image(&h, &format!("i{i:04}"), i as i64 + 1, png_bytes(8, 8)).await;
        }
        h.svc.set_favorite("i0000", true).await.unwrap();
        std::fs::remove_file(join_rel(&h.root, "2026/11/i0001.png")).unwrap();
        let (deleted, _) = h
            .svc
            .delete_non_favorite(ImageScope::All, n as i64 - 1)
            .await
            .unwrap();
        assert_eq!(deleted, n - 1);
        assert_eq!(
            h.svc.count_non_favorite(ImageScope::All).await.unwrap(),
            (0, 1)
        );
    }

    #[tokio::test]
    async fn delete_non_favorite_rejects_unknown_project() {
        let h = harness(false);
        let err = h
            .svc
            .delete_non_favorite(ImageScope::Project("nope".into()), 0)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "PROJECT_NOT_FOUND");
    }

    #[tokio::test]
    async fn export_copies_original_and_validates_destination() {
        let h = harness(false);
        let png = png_bytes(16, 16);
        add_image(&h, "a", 1, png.clone()).await;
        let dest = h.dir.join("out.png");
        std::fs::create_dir_all(&h.dir).unwrap();
        h.svc.export("a", &dest).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), png);

        assert_eq!(
            h.svc
                .export("a", Path::new("relative.png"))
                .await
                .unwrap_err()
                .code(),
            "VALIDATION"
        );
        let src = h.svc.original_path("a").await.unwrap();
        assert_eq!(
            h.svc.export("a", &src).await.unwrap_err().code(),
            "VALIDATION"
        );
        assert_eq!(
            h.svc.export("nope", &dest).await.unwrap_err().code(),
            "IMAGE_NOT_FOUND"
        );
    }

    fn no_cancel() -> Arc<std::sync::atomic::AtomicBool> {
        Arc::new(std::sync::atomic::AtomicBool::new(false))
    }

    #[tokio::test]
    async fn export_many_copies_originals_with_numbered_names_and_continues_on_failure() {
        let h = harness(false);
        let png = png_bytes(16, 16);
        add_image(&h, "a", 1, png.clone()).await;
        add_image(&h, "b", 2, png_bytes(8, 8)).await;
        add_image(&h, "c", 3, png_bytes(8, 8)).await;
        std::fs::remove_file(join_rel(&h.root, "2026/11/c.png")).unwrap();
        let out = h.dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("a.png"), b"existing").unwrap();

        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let r = h
            .svc
            .export_many(
                vec!["a".into(), "b".into(), "c".into(), "nope".into()],
                out.clone(),
                no_cancel(),
                move |d, t| s2.lock().unwrap().push((d, t)),
            )
            .await
            .unwrap();
        assert_eq!(r.saved, 2);
        assert_eq!(r.failed.len(), 2, "欠落ファイルと記録なしは失敗に数える");
        assert!(!r.cancelled);
        // 既存ファイルは上書きせず、連番になる
        assert_eq!(std::fs::read(out.join("a.png")).unwrap(), b"existing");
        assert_eq!(std::fs::read(out.join("a (1).png")).unwrap(), png);
        assert!(out.join("b.png").is_file());
        // 一時ファイルは残らない
        let tmp = std::fs::read_dir(&out)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .to_string_lossy()
                    .ends_with("gazooo-tmp")
            })
            .count();
        assert_eq!(tmp, 0);
        assert_eq!(seen.lock().unwrap().last(), Some(&(4, 4)));
    }

    #[tokio::test]
    async fn export_many_validates_destination_and_count_and_supports_cancel() {
        let h = harness(false);
        add_image(&h, "a", 1, png_bytes(8, 8)).await;
        let out = h.dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let code = |r: AppResult<ExportManyResult>| r.unwrap_err().code();
        // 空・上限超え・相対パス・管理領域の内側は拒否
        assert_eq!(
            code(
                h.svc
                    .export_many(vec![], out.clone(), no_cancel(), |_, _| {})
                    .await
            ),
            "VALIDATION"
        );
        let many = (0..=MAX_EXPORT_IDS).map(|i| i.to_string()).collect();
        assert_eq!(
            code(
                h.svc
                    .export_many(many, out.clone(), no_cancel(), |_, _| {})
                    .await
            ),
            "VALIDATION"
        );
        assert_eq!(
            code(
                h.svc
                    .export_many(vec!["a".into()], "rel".into(), no_cancel(), |_, _| {})
                    .await
            ),
            "VALIDATION"
        );
        assert_eq!(
            code(
                h.svc
                    .export_many(
                        vec!["a".into()],
                        h.root.join("2026"),
                        no_cancel(),
                        |_, _| {}
                    )
                    .await
            ),
            "VALIDATION"
        );
        // 中止済みなら未着手は処理しない
        let cancel = no_cancel();
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        let r = h
            .svc
            .export_many(vec!["a".into()], out.clone(), cancel, |_, _| {})
            .await
            .unwrap();
        assert!(r.cancelled && r.saved == 0);
        assert!(!out.join("a.png").exists());
    }

    #[tokio::test]
    async fn original_path_errors_when_file_is_missing() {
        let h = harness(false);
        add_image(&h, "a", 1, png_bytes(8, 8)).await;
        std::fs::remove_file(join_rel(&h.root, "2026/11/a.png")).unwrap();
        assert_eq!(
            h.svc.original_path("a").await.unwrap_err().code(),
            "IMAGE_NOT_FOUND"
        );
    }

    #[tokio::test]
    async fn backfill_creates_thumbnails_and_skips_undecodable() {
        let h = harness(false);
        add_image(&h, "ok", 1, png_bytes(1000, 500)).await;
        add_image(&h, "bad", 2, b"not an image".to_vec()).await;

        assert_eq!(h.svc.backfill_thumbnails().await.unwrap(), 1);
        let items = h
            .svc
            .list(None, false, ImageScope::All, None, None)
            .await
            .unwrap()
            .items;
        let ok = items.iter().find(|i| i.id == "ok").unwrap();
        let bad = items.iter().find(|i| i.id == "bad").unwrap();
        assert!(ok.thumb_path.ends_with("ok.jpg") && Path::new(&ok.thumb_path).is_file());
        assert_eq!(
            bad.thumb_path, bad.file_path,
            "作れない画像は原寸を指したまま"
        );
        // 2 回目は作成済みを再処理しない
        assert_eq!(h.svc.backfill_thumbnails().await.unwrap(), 0);
    }

    /// 実機: 本物の OS ゴミ箱へファイルを移動できること(通常は実行しない)。
    /// 一時ファイルが実際にユーザーのゴミ箱へ入るため、`cargo test -- --ignored real_os_trash` で明示的に実行する。
    #[test]
    #[ignore = "moves a temp file into the real OS trash"]
    fn real_os_trash_moves_file() {
        let dir = std::env::temp_dir().join(format!("gazooo-trash-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gazooo-trash-test.txt");
        std::fs::write(&file, "safe to delete").unwrap();

        OsTrash.trash(&file).expect("trash");
        assert!(!file.exists(), "ゴミ箱へ移動されて元の場所から無くなる");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
