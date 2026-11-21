//! ギャラリー(画像の一覧・詳細・お気に入り・削除・エクスポート・サムネイル補完)。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use crate::AppPaths;
use crate::db::Db;
use crate::db::images::{self as repo, ImageFilter};
use crate::db::jobs::{self as job_repo, ImageRow};
use crate::domain::job::{ImageItem, Job, Page};
use crate::error::{AppError, AppResult};
use crate::services::events::EventSink;
use crate::services::jobs::{build_job, image_item, join_rel};
use crate::services::settings;
use crate::services::thumbs::{make_thumbnail, thumb_rel_path};

pub const DEFAULT_PAGE_SIZE: u32 = 60;
pub const MAX_PAGE_SIZE: u32 = 200;

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
        cursor: Option<String>,
        limit: Option<u32>,
    ) -> AppResult<Page<ImageItem>> {
        let limit = i64::from(limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE));
        let after = cursor.as_deref().map(decode_cursor).transpose()?;
        let root = self.root().await?;
        let filter = ImageFilter {
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
            return Ok(());
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

        let ids: Vec<String> = removed.into_iter().map(|r| r.id).collect();
        self.sink.image_deleted(&ids);
        if failed > 0 {
            return Err(AppError::Storage(format!(
                "{failed} 件のファイルをゴミ箱へ移動できませんでした(記録は削除済みです)"
            )));
        }
        Ok(())
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
    }
    impl EventSink for Recorder {
        fn job_updated(&self, _: &Job) {}
        fn job_progress(&self, _: &ProgressEvent) {}
        fn image_created(&self, _: &ImageItem) {}
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
        let p1 = h.svc.list(None, false, None, Some(2)).await.unwrap();
        assert_eq!(
            p1.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["c", "b"]
        );
        assert!(Path::new(&p1.items[0].file_path).is_absolute());
        assert!(Path::new(&p1.items[0].file_path).is_file());

        let p2 = h
            .svc
            .list(None, false, p1.next_cursor, Some(2))
            .await
            .unwrap();
        assert_eq!(p2.items.len(), 1);
        assert!(p2.next_cursor.is_none());
        assert_eq!(
            h.svc
                .list(None, false, Some("garbage".into()), None)
                .await
                .unwrap_err()
                .code(),
            "VALIDATION"
        );
        assert_eq!(
            h.svc
                .list(None, false, Some("x:".into()), None)
                .await
                .unwrap_err()
                .code(),
            "VALIDATION"
        );
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
                .list(None, true, None, None)
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
                .list(None, false, None, None)
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
                .list(None, false, None, None)
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
        let items = h.svc.list(None, false, None, None).await.unwrap().items;
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
