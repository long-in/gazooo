use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::db::jobs::{ImageRow, insert, insert_image};
use crate::services::jobs::join_rel;

#[derive(Default)]
struct FakeTrash {
    trashed: Mutex<Vec<PathBuf>>,
    fail: AtomicBool,
}

impl Trasher for FakeTrash {
    fn trash(&self, path: &Path) -> std::io::Result<()> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("trash unavailable"));
        }
        self.trashed.lock().unwrap().push(path.to_path_buf());
        std::fs::remove_file(path)
    }
}

#[derive(Default)]
struct FakeHost {
    holds: Mutex<Vec<bool>>,
    busy: Mutex<Vec<Blocker>>,
    models: Mutex<Option<PathBuf>>,
    engine: Mutex<EngineProbe>,
    calls: Mutex<Vec<&'static str>>,
}

#[async_trait::async_trait]
impl MigrationHost for FakeHost {
    async fn hold_jobs(&self, hold: bool) {
        self.holds.lock().unwrap().push(hold);
    }
    fn busy(&self, _: Target) -> Vec<Blocker> {
        self.busy.lock().unwrap().clone()
    }
    async fn models_source(&self) -> AppResult<Option<PathBuf>> {
        Ok(self.models.lock().unwrap().clone())
    }
    async fn engine_probe(&self) -> AppResult<EngineProbe> {
        Ok(self.engine.lock().unwrap().clone())
    }
    async fn stop_engine(&self) -> AppResult<()> {
        self.calls.lock().unwrap().push("stop");
        Ok(())
    }
    async fn start_engine(&self) -> AppResult<()> {
        self.calls.lock().unwrap().push("start");
        Ok(())
    }
}

/// 進捗を記録し、指定されていればコピー中に中止する。
#[derive(Default)]
struct FakeSink {
    phases: Mutex<Vec<Phase>>,
    cancel_with: Mutex<Option<Arc<MigrationService>>>,
}

impl MigrationSink for FakeSink {
    fn progress(&self, p: &MigrationProgress) {
        self.phases.lock().unwrap().push(p.phase);
        if p.phase == Phase::Copying
            && let Some(s) = self.cancel_with.lock().unwrap().as_ref()
        {
            s.cancel();
        }
    }
}

struct Harness {
    svc: Arc<MigrationService>,
    db: Db,
    host: Arc<FakeHost>,
    sink: Arc<FakeSink>,
    trash: Arc<FakeTrash>,
    active: Arc<AtomicBool>,
    /// 移行元の画像保存先(既定)。
    images: PathBuf,
    dir: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.sink.cancel_with.lock().unwrap().take();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Harness {
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
    fn dest(&self, name: &str) -> Option<String> {
        Some(self.path(name).to_string_lossy().into_owned())
    }
    async fn images_setting(&self) -> Option<String> {
        self.db
            .call(|c| settings::load(c))
            .await
            .unwrap()
            .images_dir
    }
    async fn models_setting(&self) -> Option<String> {
        self.db.call(|c| settings::get_models_dir(c)).await.unwrap()
    }
}

fn harness() -> Harness {
    let dir = std::env::temp_dir().join(format!("gazooo-migration-{}", uuid::Uuid::now_v7()));
    let db = Db::open_in_memory().unwrap();
    // アプリのデータ領域は、移行先の候補とは別のフォルダにする
    let paths = AppPaths::new(dir.join("appdata"));
    let images = paths.default_images_dir();
    std::fs::create_dir_all(&images).unwrap();
    let host = Arc::new(FakeHost::default());
    let sink = Arc::new(FakeSink::default());
    let trash = Arc::new(FakeTrash::default());
    let active = Arc::new(AtomicBool::new(false));
    let svc = MigrationService::new(
        db.clone(),
        paths,
        trash.clone(),
        host.clone(),
        sink.clone(),
        active.clone(),
    );
    Harness {
        svc,
        db,
        host,
        sink,
        trash,
        active,
        images,
        dir,
    }
}

fn put(root: &Path, rel: &str, body: &str) {
    let p = join_rel(root, rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(join_rel(root, rel)).unwrap()
}

async fn add_image(h: &Harness, id: &str) {
    let rel = format!("2026/11/{id}.png");
    let thumb = format!("thumbs/{id}.jpg");
    put(&h.images, &rel, "ORIGINAL");
    put(&h.images, &thumb, "THUMB");
    let id = id.to_owned();
    h.db.call(move |c| {
        if crate::db::jobs::get(c, "job")?.is_none() {
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
                project_id: None,
            },
        )
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn moves_images_switches_setting_and_trashes_sources() {
    let h = harness();
    add_image(&h, "a").await;
    put(&h.images, "inputs/ref.png", "REF");
    put(&h.images, "unrelated.txt", "x");

    let plan = h.svc.plan(Target::ImagesUser, h.dest("new")).await.unwrap();
    assert_eq!(plan.blockers, vec![]);
    assert_eq!((plan.file_count, plan.copy_bytes), (3, 16));
    assert!(!h.path("new").exists(), "事前確認では何も作らない");

    let report = h
        .svc
        .start(Target::ImagesUser, h.dest("new"))
        .await
        .unwrap();
    assert_eq!((report.moved, report.left_in_source), (3, 0));
    assert_eq!(read(&h.path("new"), "2026/11/a.png"), "ORIGINAL");
    assert_eq!(read(&h.path("new"), "inputs/ref.png"), "REF");
    assert!(!join_rel(&h.images, "2026/11/a.png").exists());
    assert!(
        join_rel(&h.images, "unrelated.txt").exists(),
        "対象外は残す"
    );
    assert_eq!(h.trash.trashed.lock().unwrap().len(), 3);
    assert_eq!(h.images_setting().await, h.dest("new"));
    assert_eq!(*h.host.holds.lock().unwrap(), vec![true, false]);
    assert!(!h.active.load(Ordering::SeqCst));
    let phases = h.sink.phases.lock().unwrap();
    assert!(phases.contains(&Phase::Copying) && phases.contains(&Phase::Cleaning));
}

#[tokio::test]
async fn back_to_default_clears_the_setting() {
    let h = harness();
    add_image(&h, "a").await;
    h.svc
        .start(Target::ImagesUser, h.dest("new"))
        .await
        .unwrap();
    let report = h.svc.start(Target::ImagesUser, None).await.unwrap();
    assert_eq!(report.moved, 2);
    assert_eq!(h.images_setting().await, None);
    assert_eq!(read(&h.images, "thumbs/a.jpg"), "THUMB");
}

#[tokio::test]
async fn cancel_removes_copies_and_keeps_everything() {
    let h = harness();
    add_image(&h, "a").await;
    add_image(&h, "b").await;
    *h.sink.cancel_with.lock().unwrap() = Some(h.svc.clone());
    let err = h
        .svc
        .start(Target::ImagesUser, h.dest("new"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "STORAGE_CANCELLED");
    assert_eq!(h.images_setting().await, None);
    assert_eq!(read(&h.images, "2026/11/a.png"), "ORIGINAL");
    assert!(!join_rel(&h.path("new"), "2026/11/a.png").exists());
    assert!(h.trash.trashed.lock().unwrap().is_empty());
    assert_eq!(*h.host.holds.lock().unwrap(), vec![true, false]);
    assert!(!h.active.load(Ordering::SeqCst));
}

#[tokio::test]
async fn same_size_files_are_not_copied_and_sources_are_kept() {
    let h = harness();
    add_image(&h, "a").await;
    put(&h.path("new"), "2026/11/a.png", "OTHERONE");
    // 前回の中断で残った一時ファイルは上書きする
    put(&h.path("new"), "thumbs/a.jpg.gazooo-migrating", "partial");
    let report = h
        .svc
        .start(Target::ImagesUser, h.dest("new"))
        .await
        .unwrap();
    assert_eq!(
        (report.moved, report.kept_same_size, report.left_in_source),
        (1, 1, 1)
    );
    assert_eq!(read(&h.path("new"), "2026/11/a.png"), "OTHERONE");
    assert_eq!(read(&h.images, "2026/11/a.png"), "ORIGINAL");
    assert_eq!(read(&h.path("new"), "thumbs/a.jpg"), "THUMB");
    assert!(!join_rel(&h.path("new"), "thumbs/a.jpg.gazooo-migrating").exists());
}

#[tokio::test]
async fn different_size_conflict_blocks_without_changes() {
    let h = harness();
    add_image(&h, "a").await;
    put(&h.path("new"), "2026/11/a.png", "x");
    let plan = h.svc.plan(Target::ImagesUser, h.dest("new")).await.unwrap();
    assert_eq!(plan.blockers, vec![Blocker::Conflict]);
    assert_eq!(plan.conflicts, vec!["2026/11/a.png".to_string()]);
    let err = h
        .svc
        .start(Target::ImagesUser, h.dest("new"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "STORAGE_CONFLICT");
    assert_eq!(read(&h.path("new"), "2026/11/a.png"), "x");
    assert_eq!(h.images_setting().await, None);
}

#[tokio::test]
async fn trash_failure_still_switches_and_reports_leftovers() {
    let h = harness();
    add_image(&h, "a").await;
    h.trash.fail.store(true, Ordering::SeqCst);
    let report = h
        .svc
        .start(Target::ImagesUser, h.dest("new"))
        .await
        .unwrap();
    assert_eq!((report.moved, report.left_in_source), (2, 2));
    assert_eq!(h.images_setting().await, h.dest("new"));
}

#[tokio::test]
async fn rejects_unsafe_destinations() {
    let h = harness();
    add_image(&h, "a").await;
    let blockers = |d: Option<String>| {
        let svc = h.svc.clone();
        async move { svc.plan(Target::ImagesUser, d).await.unwrap().blockers }
    };
    assert!(
        blockers(Some("relative".into()))
            .await
            .contains(&Blocker::DestNotAbsolute)
    );
    assert!(blockers(None).await.contains(&Blocker::SameDir));
    let inside = h.images.join("sub").to_string_lossy().into_owned();
    assert!(blockers(Some(inside)).await.contains(&Blocker::Nested));
    let app = h.path("appdata").join("users/other/images");
    assert!(
        blockers(Some(app.to_string_lossy().into_owned()))
            .await
            .contains(&Blocker::DestInAppData)
    );
    if cfg!(any(windows, target_os = "macos")) {
        let upper = h.images.to_string_lossy().to_uppercase();
        assert!(blockers(Some(upper)).await.contains(&Blocker::SameDir));
    }
}

#[tokio::test]
async fn refuses_while_other_work_is_running() {
    let h = harness();
    h.host.busy.lock().unwrap().push(Blocker::DownloadActive);
    let err = h
        .svc
        .start(Target::ImagesUser, h.dest("new"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "STORAGE_BUSY");

    h.host.busy.lock().unwrap().clear();
    h.db.call(|c| {
        insert(c, "r", "{}", "p", "m", 0)?;
        crate::db::jobs::mark_running(c, "r", 1).map(|_| ())
    })
    .await
    .unwrap();
    let err = h
        .svc
        .start(Target::ImagesUser, h.dest("new"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "STORAGE_BUSY");

    h.active.store(true, Ordering::SeqCst);
    let err = h
        .svc
        .start(Target::ImagesUser, h.dest("new"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "STORAGE_BUSY");
    assert!(h.active.load(Ordering::SeqCst), "先行の移行の印は外さない");
}

/// モデルの移行元を作り、反映先の `extra_model_paths.yaml` の場所を返す。
fn model_setup(h: &Harness, block: Option<EngineBlock>) -> (PathBuf, PathBuf) {
    let models = h.path("models");
    put(&models, "checkpoints/a.safetensors", "CKPT");
    put(&models, "loras/sub/b.safetensors", "LORA");
    put(&models, "text_encoders/t.safetensors.part", "PART");
    put(&models, "checkpoints/put_checkpoints_here", "");
    put(&models, "upscale_models/u.pth", "UP");
    put(&models, "embeddings/put_embeddings_here", "");
    let engine_dir = h.path("engine");
    std::fs::create_dir_all(&engine_dir).unwrap();
    let yaml = engine_dir.join("extra_model_paths.yaml");
    *h.host.models.lock().unwrap() = Some(models.clone());
    *h.host.engine.lock().unwrap() = EngineProbe {
        managed_running: true,
        external_running: false,
        config_file: block.is_none().then(|| yaml.clone()),
        block,
    };
    (models, yaml)
}

#[tokio::test]
async fn moves_models_applies_to_engine_and_restarts_it() {
    let h = harness();
    let (models, yaml) = model_setup(&h, None);
    let plan = h.svc.plan(Target::Models, h.dest("m2")).await.unwrap();
    assert_eq!(plan.file_count, 3);
    assert_eq!(plan.skipped_folders, vec!["upscale_models".to_string()]);

    let report = h.svc.start(Target::Models, h.dest("m2")).await.unwrap();
    assert_eq!(report.moved, 3);
    assert_eq!(report.engine_applied, Some(true));
    assert_eq!(report.engine_restarted, Some(true));
    assert_eq!(*h.host.calls.lock().unwrap(), vec!["stop", "start"]);
    assert!(engine_paths::is_applied(&yaml, &h.path("m2")));
    assert_eq!(read(&h.path("m2"), "loras/sub/b.safetensors"), "LORA");
    assert!(!join_rel(&models, "checkpoints/a.safetensors").exists());
    assert!(join_rel(&models, "upscale_models/u.pth").exists());
    assert!(join_rel(&models, "checkpoints/put_checkpoints_here").exists());
    assert_eq!(h.models_setting().await, h.dest("m2"));
}

#[tokio::test]
async fn keeps_model_sources_when_engine_cannot_be_updated() {
    let h = harness();
    let (models, yaml) = model_setup(&h, Some(EngineBlock::Foreign));
    let report = h.svc.start(Target::Models, h.dest("m2")).await.unwrap();
    assert_eq!(report.engine_applied, Some(false));
    assert_eq!(report.left_in_source, 3);
    assert!(!yaml.exists());
    assert!(join_rel(&models, "checkpoints/a.safetensors").exists());
    assert!(h.trash.trashed.lock().unwrap().is_empty());
    assert_eq!(h.models_setting().await, h.dest("m2"));
}

#[tokio::test]
async fn restarts_engine_even_when_cancelled() {
    let h = harness();
    model_setup(&h, None);
    *h.sink.cancel_with.lock().unwrap() = Some(h.svc.clone());
    let err = h.svc.start(Target::Models, h.dest("m2")).await.unwrap_err();
    assert_eq!(err.code(), "STORAGE_CANCELLED");
    assert_eq!(*h.host.calls.lock().unwrap(), vec!["stop", "start"]);
    assert_eq!(h.models_setting().await, None);
}

#[tokio::test]
async fn unknown_or_missing_model_source_is_blocked() {
    let h = harness();
    let plan = h.svc.plan(Target::Models, h.dest("m2")).await.unwrap();
    assert!(plan.blockers.contains(&Blocker::SourceUnknown));
    *h.host.models.lock().unwrap() = Some(h.path("unplugged"));
    let plan = h.svc.plan(Target::Models, h.dest("m2")).await.unwrap();
    assert!(plan.blockers.contains(&Blocker::SourceMissing));
}

#[test]
fn engine_target_reports_why_it_cannot_apply() {
    let main = std::env::temp_dir()
        .join("gazooo-engine-target")
        .join("main.py")
        .to_string_lossy()
        .into_owned();
    let argv = vec![main.clone()];
    assert_eq!(
        engine_target(Some(&argv), false).1,
        Some(EngineBlock::Remote)
    );
    assert_eq!(
        engine_target(None, true).1,
        Some(EngineBlock::EngineUnknown)
    );
    let custom = vec![main, "--extra-model-paths-config".into(), "x.yaml".into()];
    assert_eq!(
        engine_target(Some(&custom), true).1,
        Some(EngineBlock::CustomConfig)
    );
    let (file, block) = engine_target(Some(&argv), true);
    assert!(file.is_some() && block.is_none());
}

// ---- 全体の移行(docs/52 §4.2) ----

use crate::db::scopes::{KEY_LEGACY_IMAGES_USER, Scope};
use crate::services::image_locations;

/// `app.db` にユーザーを登録し、そのユーザーの DB(画像の記録と個別設定)を作る。
async fn add_user(h: &Harness, id: &str, individual: Option<&Path>, images: &[&str]) {
    let (uid, name) = (id.to_owned(), format!("name-{id}"));
    h.db.call_device(move |c| {
        c.execute(
            "INSERT INTO app.users (id, name, color, created_at) VALUES (?1, ?2, 'blue', 0)",
            [&uid, &name],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let workspace = h.path("appdata").join("users").join(id);
    std::fs::create_dir_all(&workspace).unwrap();
    let c = rusqlite::Connection::open(workspace.join("gazooo.db")).unwrap();
    c.execute_batch(
        "CREATE TABLE settings (key TEXT PRIMARY KEY, value_json TEXT NOT NULL, updated_at INTEGER NOT NULL);
         CREATE TABLE images (file_path TEXT NOT NULL, thumb_path TEXT NOT NULL);",
    )
    .unwrap();
    if let Some(dir) = individual {
        c.execute(
            "INSERT INTO settings VALUES ('storage.images_dir', ?1, 0)",
            [serde_json::json!(dir.to_string_lossy()).to_string()],
        )
        .unwrap();
    }
    for rel in images {
        c.execute("INSERT INTO images VALUES (?1, ?1)", [rel])
            .unwrap();
    }
}

fn standard(h: &Harness, id: &str) -> PathBuf {
    h.path("appdata").join("users").join(id).join("images")
}

async fn global(h: &Harness) -> (Option<String>, Option<String>) {
    h.db.call_device(|c| settings::global_images(c))
        .await
        .unwrap()
}

#[tokio::test]
async fn global_move_takes_every_follower_and_skips_individual_users() {
    let h = harness();
    let own = h.path("own");
    let legacy = h.path("appdata").join("images");
    add_user(&h, "a", None, &["2026/11/a.png"]).await;
    add_user(&h, "b", None, &["2026/11/b.png"]).await;
    add_user(&h, "c", Some(&own), &["2026/11/c.png"]).await;
    add_user(&h, "empty", None, &[]).await;
    // a は以前の版の場所を使っている
    h.db.call_device(|c| {
        scopes::write(c, Scope::Global, KEY_LEGACY_IMAGES_USER, Some(&"a".into()))
    })
    .await
    .unwrap();
    put(&legacy, "2026/11/a.png", "A");
    put(&legacy, "inputs/ra.png", "RA");
    put(&standard(&h, "b"), "2026/11/b.png", "B");
    put(&own, "2026/11/c.png", "C");

    let plan = h
        .svc
        .plan(Target::ImagesGlobal, h.dest("root"))
        .await
        .unwrap();
    assert_eq!(plan.blockers, vec![]);
    assert_eq!((plan.file_count, plan.excluded_users), (3, 1));
    assert_eq!(plan.users.len(), 3);

    let report = h
        .svc
        .start(Target::ImagesGlobal, h.dest("root"))
        .await
        .unwrap();
    assert_eq!((report.moved, report.left_in_source), (3, 0));
    let root = h.path("root");
    assert_eq!(read(&root.join("a"), "inputs/ra.png"), "RA");
    assert_eq!(read(&root.join("b"), "2026/11/b.png"), "B");
    assert!(own.join("2026/11/c.png").exists(), "個別設定は動かさない");
    assert_eq!(global(&h).await, (h.dest("root"), None));
    let all = image_locations::all(&h.db, &AppPaths::new(h.path("appdata")))
        .await
        .unwrap();
    let dir = |id: &str| all.users.iter().find(|u| u.id == id).unwrap().dir.clone();
    assert_eq!(dir("a"), root.join("a"));
    assert_eq!(dir("c"), own);

    // 標準値へ戻すと、以前の版の場所ではなく各ユーザーの既定の場所になる
    let report = h.svc.start(Target::ImagesGlobal, None).await.unwrap();
    assert_eq!(report.moved, 3);
    assert_eq!(read(&standard(&h, "a"), "2026/11/a.png"), "A");
    assert_eq!(global(&h).await, (None, None));
}

#[tokio::test]
async fn global_move_changes_nothing_when_one_user_fails() {
    let h = harness();
    add_user(&h, "a", None, &["2026/11/a.png"]).await;
    add_user(&h, "b", None, &["2026/11/b.png"]).await;
    put(&standard(&h, "a"), "2026/11/a.png", "A");
    put(&standard(&h, "b"), "2026/11/b.png", "B");
    // b のコピー先に、フォルダの代わりにファイルを置いて失敗させる
    put(&h.path("root"), "b/2026", "blocker");
    let err = h
        .svc
        .start(Target::ImagesGlobal, h.dest("root"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "STORAGE_ERROR");
    assert_eq!(global(&h).await, (None, None));
    assert!(!join_rel(&h.path("root"), "a/2026/11/a.png").exists());
    assert_eq!(read(&standard(&h, "a"), "2026/11/a.png"), "A");
    assert!(h.trash.trashed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn global_destination_must_not_overlap_individual_folders() {
    let h = harness();
    let own = h.path("own");
    add_user(&h, "c", Some(&own), &[]).await;
    let inside = own.join("sub").to_string_lossy().into_owned();
    let plan = h
        .svc
        .plan(Target::ImagesGlobal, Some(inside))
        .await
        .unwrap();
    assert!(plan.blockers.contains(&Blocker::DestInUse));
    let app = h.path("appdata").join("x").to_string_lossy().into_owned();
    let plan = h.svc.plan(Target::ImagesGlobal, Some(app)).await.unwrap();
    assert!(plan.blockers.contains(&Blocker::DestInAppData));
}

#[tokio::test]
async fn global_move_is_blocked_when_a_users_folder_is_missing() {
    let h = harness();
    add_user(&h, "a", None, &["2026/11/a.png"]).await;
    let plan = h
        .svc
        .plan(Target::ImagesGlobal, h.dest("root"))
        .await
        .unwrap();
    assert!(plan.blockers.contains(&Blocker::SourceMissing));
}

#[tokio::test]
async fn individual_destination_cannot_use_another_users_folder() {
    let h = harness();
    let own = h.path("own");
    add_user(&h, "other", Some(&own), &[]).await;
    add_image(&h, "a").await;
    let plan = h.svc.plan(Target::ImagesUser, h.dest("own")).await.unwrap();
    assert!(plan.blockers.contains(&Blocker::DestInUse));
}

#[tokio::test]
async fn shared_source_keeps_the_original_reference_images() {
    let h = harness();
    let shared = h.path("shared");
    add_user(&h, "other", Some(&shared), &[]).await;
    let value = serde_json::json!(shared.to_string_lossy());
    h.db.call(move |c| scopes::write(c, Scope::User, "storage.images_dir", Some(&value)))
        .await
        .unwrap();
    put(&shared, "inputs/theirs.png", "REF");
    let report = h
        .svc
        .start(Target::ImagesUser, h.dest("mine"))
        .await
        .unwrap();
    assert_eq!((report.moved, report.left_in_source), (1, 1));
    assert_eq!(read(&shared, "inputs/theirs.png"), "REF");
    assert!(h.trash.trashed.lock().unwrap().is_empty());
}
