//! 画像保存先の変更(docs/04-ipc-api.md「保存先の変更」、Q-11)。
//!
//! DB には保存先からの相対パスしか持たないため、既存画像のファイルを新しい保存先へ移さないと
//! 変更後に表示できなくなる。移動は「コピー → 検証 → 設定切替 → 旧ファイルをゴミ箱へ」の順で行い、
//! 途中で失敗したら作ったコピーを消して中止する(元のファイルと設定は変わらない)。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use crate::AppPaths;
use crate::db::Db;
use crate::db::images as image_repo;
use crate::db::jobs::ImageRow;
use crate::domain::params::is_valid_input_path;
use crate::error::{AppError, AppResult};
use crate::services::images::Trasher;
use crate::services::jobs::join_rel;
use crate::services::settings::{self, SettingsPatch};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveReport {
    /// コピーしたファイル数(原寸 + サムネイル + ControlNet の参照画像)。
    pub moved: usize,
    /// 元ファイルが無くスキップした数。
    pub missing: usize,
}

pub struct StorageService {
    db: Db,
    paths: AppPaths,
    trasher: Arc<dyn Trasher>,
}

struct CopyResult {
    report: MoveReport,
    /// 新しい保存先に新規作成したファイル(ロールバック用)。
    created: Vec<PathBuf>,
    /// コピー元(成功後にゴミ箱へ移す)。
    sources: Vec<PathBuf>,
}

fn storage_err(path: &Path, e: impl std::fmt::Display) -> AppError {
    AppError::Storage(format!("{}: {e}", path.display()))
}

/// コピーして、サイズが一致することを確認する。
fn copy_verified(src: &Path, dst: &Path) -> AppResult<()> {
    let copied = std::fs::copy(src, dst).map_err(|e| storage_err(dst, e))?;
    let expected = std::fs::metadata(src)
        .map_err(|e| storage_err(src, e))?
        .len();
    if copied != expected {
        return Err(storage_err(dst, "コピー後のサイズが一致しません"));
    }
    Ok(())
}

/// 移動の対象になる相対パスの一覧: 各画像の原寸・サムネイルと、ControlNet の参照画像(`inputs/`)。
fn rels_to_move(rows: &[ImageRow], old_root: &Path) -> Vec<String> {
    let mut rels = Vec::new();
    for row in rows {
        rels.push(row.file_path.clone());
        if row.thumb_path != row.file_path {
            rels.push(row.thumb_path.clone());
        }
    }
    // 参照画像は DB に記録が無い(パラメータの中にだけある)ため、フォルダを走査する。
    // ファイル名は取り込み時に安全な形に限っているが、念のため検証を通ったものだけを移す
    if let Ok(entries) = std::fs::read_dir(old_root.join("inputs")) {
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

fn copy_all(rels: &[String], old_root: &Path, new_root: &Path) -> AppResult<CopyResult> {
    let mut out = CopyResult {
        report: MoveReport::default(),
        created: Vec::new(),
        sources: Vec::new(),
    };
    let rollback = |created: &[PathBuf]| {
        for f in created {
            let _ = std::fs::remove_file(f);
        }
    };

    for rel in rels {
        let src = join_rel(old_root, rel);
        if !src.is_file() {
            out.report.missing += 1;
            continue;
        }
        let dst = join_rel(new_root, rel);
        let existed = dst.exists();
        let result = match dst.parent() {
            Some(parent) => std::fs::create_dir_all(parent).map_err(|e| storage_err(parent, e)),
            None => Ok(()),
        }
        .and_then(|()| copy_verified(&src, &dst));
        if let Err(e) = result {
            rollback(&out.created);
            return Err(e);
        }
        if !existed {
            out.created.push(dst);
        }
        out.report.moved += 1;
        out.sources.push(src);
    }
    Ok(out)
}

impl StorageService {
    pub fn new(db: Db, paths: AppPaths, trasher: Arc<dyn Trasher>) -> Arc<Self> {
        Arc::new(Self { db, paths, trasher })
    }

    /// 保存先を変更し、新しい保存先の絶対パスと結果を返す。`new_dir = None` は既定の保存先に戻す。
    pub async fn change_images_dir(
        &self,
        new_dir: Option<String>,
        move_existing: bool,
    ) -> AppResult<(PathBuf, MoveReport)> {
        let current = self.db.call(|c| settings::load(c)).await?;
        let old_root = self.paths.resolve_images_dir(&current);
        let (new_root, new_setting) = match new_dir {
            Some(dir) => {
                let p = PathBuf::from(&dir);
                if !p.is_absolute() {
                    return Err(AppError::validation(
                        "newDir",
                        "保存先は絶対パスで指定してください",
                    ));
                }
                (p, Some(dir))
            }
            None => (self.paths.default_images_dir(), None),
        };

        let apply_setting = |setting: Option<String>| {
            let patch = SettingsPatch {
                images_dir: Some(setting),
                ..SettingsPatch::default()
            };
            self.db
                .call(move |c| settings::update(c, patch).map(|_| ()))
        };

        if new_root == old_root {
            apply_setting(new_setting).await?; // 既定パスの明示指定などの表記だけを正規化する
            return Ok((new_root, MoveReport::default()));
        }
        if new_root.starts_with(&old_root) || old_root.starts_with(&new_root) {
            return Err(AppError::validation(
                "newDir",
                "新しい保存先が現在の保存先と入れ子になっています",
            ));
        }

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
            return Err(AppError::JobInvalidState(
                "生成中は保存先を変更できません。完了またはキャンセルしてから変更してください"
                    .into(),
            ));
        }

        // 作成できて書き込めることを先に確認する(移動の途中で気づくのを避ける)
        let probe_root = new_root.clone();
        tokio::task::spawn_blocking(move || -> AppResult<()> {
            std::fs::create_dir_all(&probe_root).map_err(|e| storage_err(&probe_root, e))?;
            let probe = probe_root.join(format!(".gazooo-write-test-{}", uuid::Uuid::now_v7()));
            std::fs::write(&probe, b"").map_err(|_| {
                AppError::validation(
                    "newDir",
                    "この場所には書き込めません。別の場所を選んでください",
                )
            })?;
            let _ = std::fs::remove_file(&probe);
            Ok(())
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;

        if !move_existing {
            apply_setting(new_setting).await?;
            return Ok((new_root, MoveReport::default()));
        }

        let rows = self.db.call(|c| image_repo::all(c)).await?;
        let (old, new) = (old_root.clone(), new_root.clone());
        let copied = tokio::task::spawn_blocking(move || {
            let rels = rels_to_move(&rows, &old);
            copy_all(&rels, &old, &new)
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;

        if let Err(e) = apply_setting(new_setting).await {
            for f in &copied.created {
                let _ = std::fs::remove_file(f);
            }
            return Err(e);
        }

        // 切替後、旧ファイルはゴミ箱へ(失敗しても新しい保存先で動作するので続行する)
        for src in copied.sources {
            let trasher = Arc::clone(&self.trasher);
            let path = src.clone();
            let res = tokio::task::spawn_blocking(move || trasher.trash(&path)).await;
            if !matches!(res, Ok(Ok(()))) {
                tracing::warn!(path = %src.display(), "failed to move old file to trash");
            }
        }
        Ok((new_root, copied.report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::jobs::{get, insert, insert_image};
    use crate::services::settings::load;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeTrash {
        trashed: Mutex<Vec<PathBuf>>,
    }
    impl Trasher for FakeTrash {
        fn trash(&self, path: &Path) -> std::io::Result<()> {
            self.trashed.lock().unwrap().push(path.to_path_buf());
            std::fs::remove_file(path)
        }
    }

    struct Harness {
        svc: Arc<StorageService>,
        db: Db,
        trash: Arc<FakeTrash>,
        old_root: PathBuf,
        dir: PathBuf,
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn harness() -> Harness {
        let dir = std::env::temp_dir().join(format!("gazooo-storage-{}", uuid::Uuid::now_v7()));
        let db = Db::open_in_memory().unwrap();
        let trash = Arc::new(FakeTrash::default());
        let paths = AppPaths::new(dir.clone());
        let old_root = paths.default_images_dir();
        let svc = StorageService::new(db.clone(), paths, trash.clone());
        Harness {
            svc,
            db,
            trash,
            old_root,
            dir,
        }
    }

    /// 原寸 + サムネイルのファイルと DB の記録を作る。
    async fn add_image(h: &Harness, id: &str, with_thumb: bool) {
        let rel = format!("2026/11/{id}.png");
        let thumb = if with_thumb {
            format!("thumbs/{id}.jpg")
        } else {
            rel.clone()
        };
        for (r, body) in [(&rel, "ORIGINAL"), (&thumb, "THUMB")] {
            let p = join_rel(&h.old_root, r);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            if !p.exists() {
                std::fs::write(p, body).unwrap();
            }
        }
        let id = id.to_owned();
        h.db.call(move |c| {
            if get(c, "job")?.is_none() {
                insert(c, "job", "{}", "p", "m", 0)?;
            }
            insert_image(
                c,
                &ImageRow {
                    id,
                    job_id: "job".into(),
                    file_path: rel,
                    thumb_path: thumb,
                    width: 1,
                    height: 1,
                    seed: 1,
                    file_size: 1,
                    is_favorite: false,
                    created_at: 0,
                },
            )
        })
        .await
        .unwrap();
    }

    async fn images_dir_setting(h: &Harness) -> Option<String> {
        h.db.call(|c| load(c)).await.unwrap().images_dir
    }

    fn new_dir(h: &Harness, name: &str) -> String {
        h.dir.join(name).to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn move_copies_originals_and_thumbs_switches_setting_and_trashes_old() {
        let h = harness();
        add_image(&h, "a", true).await;
        add_image(&h, "b", false).await; // サムネイルなし(原寸と同じパス)
        let target = new_dir(&h, "new-images");

        let (root, report) = h
            .svc
            .change_images_dir(Some(target.clone()), true)
            .await
            .unwrap();
        assert_eq!(root, PathBuf::from(&target));
        assert_eq!(
            report,
            MoveReport {
                moved: 3,
                missing: 0
            }
        ); // a: 原寸+サムネ、b: 原寸のみ
        for rel in ["2026/11/a.png", "thumbs/a.jpg", "2026/11/b.png"] {
            assert!(
                join_rel(&root, rel).is_file(),
                "{rel} should exist in the new root"
            );
            assert!(
                !join_rel(&h.old_root, rel).exists(),
                "{rel} should be gone from the old root"
            );
        }
        assert_eq!(
            std::fs::read_to_string(join_rel(&root, "2026/11/a.png")).unwrap(),
            "ORIGINAL"
        );
        assert_eq!(
            std::fs::read_to_string(join_rel(&root, "thumbs/a.jpg")).unwrap(),
            "THUMB"
        );
        assert_eq!(
            h.trash.trashed.lock().unwrap().len(),
            3,
            "旧ファイルは削除ではなくゴミ箱へ"
        );
        assert_eq!(images_dir_setting(&h).await, Some(target));
    }

    #[tokio::test]
    async fn move_includes_controlnet_reference_images() {
        let h = harness();
        add_image(&h, "a", true).await;
        let inputs = h.old_root.join("inputs");
        std::fs::create_dir_all(&inputs).unwrap();
        std::fs::write(inputs.join("ref.png"), "REF").unwrap();
        // 不正な名前のファイルは移さない(取り込みでは作られない形)
        std::fs::write(inputs.join(".hidden"), "x").unwrap();
        std::fs::write(inputs.join("bad name.png"), "x").unwrap();
        let target = new_dir(&h, "with-inputs");

        let (root, report) = h.svc.change_images_dir(Some(target), true).await.unwrap();
        assert_eq!(
            report,
            MoveReport {
                moved: 3,
                missing: 0
            }
        ); // a: 原寸+サムネ、参照画像 1 枚
        assert_eq!(
            std::fs::read_to_string(join_rel(&root, "inputs/ref.png")).unwrap(),
            "REF"
        );
        assert!(!join_rel(&root, "inputs/.hidden").exists());
        assert!(!join_rel(&root, "inputs/bad name.png").exists());
        assert!(!inputs.join("ref.png").exists(), "旧ファイルはゴミ箱へ");
    }

    #[tokio::test]
    async fn without_move_only_the_setting_changes() {
        let h = harness();
        add_image(&h, "a", true).await;
        let target = new_dir(&h, "elsewhere");
        let (_, report) = h
            .svc
            .change_images_dir(Some(target.clone()), false)
            .await
            .unwrap();
        assert_eq!(report, MoveReport::default());
        assert!(
            join_rel(&h.old_root, "2026/11/a.png").exists(),
            "旧ファイルはそのまま残る"
        );
        assert!(Path::new(&target).is_dir(), "新しい保存先は作成される");
        assert_eq!(images_dir_setting(&h).await, Some(target));
        assert!(h.trash.trashed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn copy_failure_rolls_back_and_keeps_setting_and_originals() {
        let h = harness();
        add_image(&h, "a", false).await;
        add_image(&h, "b", false).await;
        let target = new_dir(&h, "new");
        // 2 枚目のコピー先にディレクトリを置いて、コピーを失敗させる
        let blocker = join_rel(Path::new(&target), "2026/11/b.png");
        std::fs::create_dir_all(&blocker).unwrap();

        let err = h
            .svc
            .change_images_dir(Some(target.clone()), true)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "STORAGE_ERROR");
        assert_eq!(images_dir_setting(&h).await, None, "設定は変わらない");
        assert!(
            join_rel(&h.old_root, "2026/11/a.png").exists(),
            "元ファイルは残る"
        );
        assert!(join_rel(&h.old_root, "2026/11/b.png").exists());
        assert!(
            !join_rel(Path::new(&target), "2026/11/a.png").exists(),
            "作ったコピーは片付ける"
        );
        assert!(h.trash.trashed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn missing_source_files_are_counted_and_skipped() {
        let h = harness();
        add_image(&h, "a", false).await;
        std::fs::remove_file(join_rel(&h.old_root, "2026/11/a.png")).unwrap();
        let (_, report) = h
            .svc
            .change_images_dir(Some(new_dir(&h, "n")), true)
            .await
            .unwrap();
        assert_eq!(
            report,
            MoveReport {
                moved: 0,
                missing: 1
            }
        );
    }

    #[tokio::test]
    async fn rejects_invalid_destinations() {
        let h = harness();
        let field = |e: AppError| match e {
            AppError::Validation { field, .. } => field,
            other => panic!("unexpected: {other:?}"),
        };
        assert_eq!(
            field(
                h.svc
                    .change_images_dir(Some("relative/dir".into()), true)
                    .await
                    .unwrap_err()
            ),
            "newDir"
        );
        // 現在の保存先の内側 / 外側
        let inside = h.old_root.join("sub").to_string_lossy().into_owned();
        assert_eq!(
            field(
                h.svc
                    .change_images_dir(Some(inside), true)
                    .await
                    .unwrap_err()
            ),
            "newDir"
        );
        let outside = h.dir.to_string_lossy().into_owned();
        assert_eq!(
            field(
                h.svc
                    .change_images_dir(Some(outside), true)
                    .await
                    .unwrap_err()
            ),
            "newDir"
        );
        assert_eq!(images_dir_setting(&h).await, None);
    }

    #[tokio::test]
    async fn refuses_while_a_job_is_running() {
        let h = harness();
        h.db.call(|c| {
            insert(c, "j", "{}", "p", "m", 0)?;
            crate::db::jobs::mark_running(c, "j", 1).map(|_| ())
        })
        .await
        .unwrap();
        let err = h
            .svc
            .change_images_dir(Some(new_dir(&h, "x")), true)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "JOB_INVALID_STATE");
        assert_eq!(images_dir_setting(&h).await, None);
    }

    #[tokio::test]
    async fn same_dir_is_a_noop_and_none_restores_default() {
        let h = harness();
        add_image(&h, "a", false).await;
        let (_, report) = h
            .svc
            .change_images_dir(Some(h.old_root.to_string_lossy().into_owned()), true)
            .await
            .unwrap();
        assert_eq!(report, MoveReport::default());
        assert!(join_rel(&h.old_root, "2026/11/a.png").exists());
        assert!(h.trash.trashed.lock().unwrap().is_empty());

        // 変更 → 既定に戻す(元の場所へ移動して戻る)
        let target = new_dir(&h, "custom");
        h.svc
            .change_images_dir(Some(target.clone()), true)
            .await
            .unwrap();
        let (root, report) = h.svc.change_images_dir(None, true).await.unwrap();
        assert_eq!(root, h.old_root);
        assert_eq!(report.moved, 1);
        assert_eq!(images_dir_setting(&h).await, None);
        assert!(join_rel(&h.old_root, "2026/11/a.png").is_file());
    }
}
