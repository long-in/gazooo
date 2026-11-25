//! モデル取得サービスのテスト。ローカルのモックサーバーだけを使い、外部へは出ない。

use std::time::Duration;

use sha2::{Digest, Sha256};

use super::download::DownloadPolicy;
use super::testserver::{Reply, Server, spawn};
use super::*;

fn data(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 17 % 253) as u8).collect()
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Default)]
struct TestSink {
    events: Mutex<Vec<(&'static str, DownloadState)>>,
}

impl TestSink {
    fn done_states(&self) -> Vec<DownloadState> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| *k == "done")
            .map(|(_, s)| s.clone())
            .collect()
    }
}

impl ModelSink for TestSink {
    fn progress(&self, state: &DownloadState) {
        self.events
            .lock()
            .unwrap()
            .push(("progress", state.clone()));
    }
    fn done(&self, state: &DownloadState) {
        self.events.lock().unwrap().push(("done", state.clone()));
    }
}

struct TmpDir(PathBuf);

impl TmpDir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("gazooo-models-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn catalog_json(base: &str, body: &[u8], requires_consent: bool) -> String {
    format!(
        r#"{{"schemaVersion":2,"families":[{{"id":"fam","category":"controlnet",
        "name":{{"ja":"a","en":"a"}},"description":{{"ja":"a","en":"a"}},
        "variants":[{{"id":"var","label":{{"ja":"a","en":"a"}},"baseModel":"sdxl",
        "source":{{"url":"{base}/m","repo":"r"}},"sizeBytes":{},"sha256":"{}",
        "destDir":"controlnet","fileName":"controlnet-canny-test.safetensors",
        "license":{{"name":"l","url":"https://x.test","commercialUse":"see-license"}},
        "requiresConsent":{requires_consent}}}]}}]}}"#,
        body.len(),
        sha(body)
    )
}

struct Fixture {
    svc: Arc<ModelService>,
    sink: Arc<TestSink>,
    models: TmpDir,
    #[allow(dead_code)]
    server: Server,
}

impl Fixture {
    fn target(&self) -> PathBuf {
        self.models
            .0
            .join("controlnet")
            .join("controlnet-canny-test.safetensors")
    }
    fn part(&self) -> PathBuf {
        part_path_of(&self.target())
    }
}

async fn fixture(reply: Reply, body: &[u8], requires_consent: bool, with_dir: bool) -> Fixture {
    let server = spawn(vec![("/m", reply)]).await;
    let policy = DownloadPolicy::for_test("127.0.0.1");
    let catalog =
        Catalog::parse(&catalog_json(&server.base, body, requires_consent), &policy).unwrap();
    let sink = Arc::new(TestSink::default());
    let svc = ModelService::new(
        Db::open_in_memory().unwrap(),
        // 到達できない ComfyUI(自動検出は失敗する)
        Arc::new(EngineService::new("http://127.0.0.1:1").unwrap()),
        Arc::new(catalog),
        policy,
        Arc::clone(&sink) as Arc<dyn ModelSink>,
    )
    .unwrap();
    let models = TmpDir::new();
    if with_dir {
        svc.set_dir(Some(models.0.to_string_lossy().into_owned()))
            .await
            .unwrap();
    }
    Fixture {
        svc,
        sink,
        models,
        server,
    }
}

async fn wait_done(f: &Fixture, n: usize) -> Vec<DownloadState> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let done = f.sink.done_states();
            if done.len() >= n {
                return done;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("ダウンロードが終わらない")
}

fn body_reply(body: &[u8]) -> Reply {
    Reply::Body {
        body: body.to_vec(),
        ranges: true,
    }
}

#[tokio::test]
async fn downloads_end_to_end_and_reports_installed() {
    let body = data(120_000);
    let f = fixture(body_reply(&body), &body, false, true).await;
    tokio::spawn(Arc::clone(&f.svc).run_worker());

    let before = f.svc.catalog_view().await.unwrap();
    assert!(!before.installed[0].installed);
    assert_eq!(before.models_dir_source, DirSource::Setting);

    let id = f.svc.start("fam", "var", false).await.unwrap();
    let done = wait_done(&f, 1).await;
    assert_eq!(done[0].download_id, id);
    assert_eq!(done[0].status, Status::Done);
    assert_eq!(done[0].received_bytes, body.len() as u64);
    assert_eq!(std::fs::read(f.target()).unwrap(), body);
    assert!(!f.part().exists());

    let after = f.svc.catalog_view().await.unwrap();
    assert!(after.installed[0].installed && after.installed[0].size_matches);
    assert!(f.svc.verify("fam", "var").await.unwrap());
    assert_eq!(f.svc.list().unwrap()[0].status, Status::Done);
}

#[tokio::test]
async fn failure_is_reported_with_a_code_and_can_be_retried() {
    let body = data(10_000);
    let f = fixture(Reply::Status(500), &body, false, true).await;
    tokio::spawn(Arc::clone(&f.svc).run_worker());
    f.svc.start("fam", "var", false).await.unwrap();
    let done = wait_done(&f, 1).await;
    assert_eq!(done[0].status, Status::Failed);
    assert_eq!(
        done[0].error.as_ref().unwrap().code,
        "MODELS_DOWNLOAD_FAILED"
    );
    assert!(!f.target().exists());

    // 再試行すると、過去の失敗は置き換えられる(一覧に重複しない)
    f.svc.start("fam", "var", false).await.unwrap();
    wait_done(&f, 2).await;
    assert_eq!(f.svc.list().unwrap().len(), 1);
}

#[tokio::test]
async fn wrong_hash_is_rejected_and_not_installed() {
    let body = data(10_000);
    // カタログのハッシュは別のデータのもの
    let f = fixture(body_reply(&data(10_000)[..9_999]), &body, false, true).await;
    tokio::spawn(Arc::clone(&f.svc).run_worker());
    f.svc.start("fam", "var", false).await.unwrap();
    let done = wait_done(&f, 1).await;
    assert_eq!(done[0].status, Status::Failed);
    assert!(!f.target().exists());
}

#[tokio::test]
async fn start_validates_before_queueing() {
    let body = data(1_000);

    // モデルフォルダが特定できない
    let f = fixture(body_reply(&body), &body, false, false).await;
    let e = f.svc.start("fam", "var", false).await.unwrap_err();
    assert_eq!(e.code(), "MODELS_DIR_UNKNOWN");
    assert_eq!(
        f.svc.catalog_view().await.unwrap().models_dir_source,
        DirSource::Unknown
    );
    assert!(f.svc.list().unwrap().is_empty());

    // カタログにない
    let f = fixture(body_reply(&body), &body, false, true).await;
    assert_eq!(
        f.svc.start("nope", "var", false).await.unwrap_err().code(),
        "MODELS_NOT_FOUND"
    );
    assert_eq!(
        f.svc.start("fam", "nope", false).await.unwrap_err().code(),
        "MODELS_NOT_FOUND"
    );

    // すでにある(上書きしない)
    std::fs::create_dir_all(f.target().parent().unwrap()).unwrap();
    std::fs::write(f.target(), b"mine").unwrap();
    assert_eq!(
        f.svc.start("fam", "var", false).await.unwrap_err().code(),
        "MODELS_ALREADY_INSTALLED"
    );
    assert_eq!(std::fs::read(f.target()).unwrap(), b"mine");
}

#[tokio::test]
async fn consent_is_required_once_then_remembered() {
    let body = data(1_000);
    let f = fixture(body_reply(&body), &body, true, true).await;
    assert_eq!(
        f.svc.start("fam", "var", false).await.unwrap_err().code(),
        "MODELS_NO_CONSENT"
    );
    assert!(
        f.svc.list().unwrap().is_empty(),
        "同意なしでは何も始まらない"
    );

    f.svc.start("fam", "var", true).await.unwrap();
    // 2 回目以降は同意の指定が無くても進める
    let f2_id = f.svc.start("fam", "var", false).await.unwrap();
    assert_eq!(f.svc.list().unwrap().len(), 1);
    assert_eq!(
        f.svc.list().unwrap()[0].download_id,
        f2_id,
        "取得中の同じモデルは同じ ID を返す"
    );
}

#[tokio::test]
async fn cancel_queued_marks_cancelled_without_a_worker() {
    let body = data(1_000);
    let f = fixture(body_reply(&body), &body, false, true).await;
    let id = f.svc.start("fam", "var", false).await.unwrap();
    f.svc.cancel(&id, false).unwrap();
    assert_eq!(f.svc.list().unwrap()[0].status, Status::Cancelled);
    assert_eq!(f.sink.done_states().len(), 1);
    assert_eq!(
        f.svc.cancel("missing", false).unwrap_err().code(),
        "MODELS_NOT_FOUND"
    );
}

#[tokio::test]
async fn cancel_running_keeps_or_discards_the_part_file() {
    for discard in [false, true] {
        let body = data(3_000_000);
        let slow = Reply::Slow {
            body: body.clone(),
            chunk: 100_000,
            delay: Duration::from_millis(60),
        };
        let f = fixture(slow, &body, false, true).await;
        tokio::spawn(Arc::clone(&f.svc).run_worker());
        let id = f.svc.start("fam", "var", false).await.unwrap();

        // 受信が始まるまで待つ
        tokio::time::timeout(Duration::from_secs(5), async {
            while !f.part().exists() || std::fs::metadata(f.part()).unwrap().len() == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        f.svc.cancel(&id, discard).unwrap();
        let done = wait_done(&f, 1).await;
        assert_eq!(done[0].status, Status::Cancelled);
        assert!(!f.target().exists());
        assert_eq!(f.part().exists(), !discard, "discard={discard}");
    }
}

#[tokio::test]
async fn verify_detects_a_corrupted_file_and_missing_file() {
    let body = data(2_000);
    let f = fixture(body_reply(&body), &body, false, true).await;
    assert_eq!(
        f.svc.verify("fam", "var").await.unwrap_err().code(),
        "MODELS_NOT_FOUND"
    );
    std::fs::create_dir_all(f.target().parent().unwrap()).unwrap();
    std::fs::write(f.target(), data(2_000)).unwrap();
    assert!(f.svc.verify("fam", "var").await.unwrap());
    std::fs::write(f.target(), b"corrupted").unwrap();
    assert!(!f.svc.verify("fam", "var").await.unwrap());
    let view = f.svc.catalog_view().await.unwrap();
    assert!(view.installed[0].installed && !view.installed[0].size_matches);
}

#[tokio::test]
async fn set_dir_validates_and_can_revert_to_detection() {
    let body = data(100);
    let f = fixture(body_reply(&body), &body, false, false).await;
    assert_eq!(
        f.svc
            .set_dir(Some("relative/dir".into()))
            .await
            .unwrap_err()
            .code(),
        "VALIDATION"
    );
    let missing = f.models.0.join("nope");
    assert_eq!(
        f.svc
            .set_dir(Some(missing.to_string_lossy().into_owned()))
            .await
            .unwrap_err()
            .code(),
        "VALIDATION"
    );
    let ok = f
        .svc
        .set_dir(Some(f.models.0.to_string_lossy().into_owned()))
        .await
        .unwrap();
    assert_eq!(ok.source, DirSource::Setting);
    // 検査用の一時ファイルが残らない
    assert_eq!(std::fs::read_dir(&f.models.0).unwrap().count(), 0);
    let reverted = f.svc.set_dir(None).await.unwrap();
    assert_eq!(reverted.source, DirSource::Unknown);
    assert!(reverted.models_dir.is_none());
}

#[test]
fn models_dir_is_derived_from_comfyui_launch_args() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    // 08 のセットアップ(--base-directory)
    let argv = s(&[
        "C:\\w\\comfyui\\ComfyUI\\main.py",
        "--listen",
        "127.0.0.1",
        "--port",
        "8188",
        "--base-directory",
        "C:\\w\\comfyui\\data",
        "--disable-auto-launch",
    ]);
    if cfg!(windows) {
        assert_eq!(
            models_dir_from_argv(&argv),
            Some(PathBuf::from("C:\\w\\comfyui\\data").join("models"))
        );
    }
    let unix = s(&["/opt/ComfyUI/main.py", "--base-directory=/data/c"]);
    if cfg!(unix) {
        assert_eq!(
            models_dir_from_argv(&unix),
            Some(PathBuf::from("/data/c/models"))
        );
        // --base-directory が無ければ main.py と同じ場所の models
        assert_eq!(
            models_dir_from_argv(&s(&["/opt/ComfyUI/main.py", "--port", "8188"])),
            Some(PathBuf::from("/opt/ComfyUI/models"))
        );
    }
    // 相対パスは特定できない
    assert_eq!(models_dir_from_argv(&s(&["main.py"])), None);
    assert_eq!(
        models_dir_from_argv(&s(&["main.py", "--base-directory", "data"])),
        None
    );
    assert_eq!(models_dir_from_argv(&[]), None);
}

#[test]
fn only_local_engines_are_detected() {
    for url in [
        "http://127.0.0.1:8188",
        "http://localhost:8188",
        "http://[::1]:8188",
        "http://LOCALHOST",
    ] {
        assert!(is_local_engine(url), "{url}");
    }
    for url in [
        "http://192.168.1.5:8188",
        "https://comfy.example.com",
        "http://127.0.0.1.evil.com",
        "nonsense",
    ] {
        assert!(!is_local_engine(url), "{url}");
    }
}

/// 実機検証: 実際の Hugging Face から最小のモデル(約 6MB)を取得できるか。
/// 本番の許可リスト・CDN へのリダイレクト・TLS・ハッシュ照合を通す。通常の `cargo test` では実行されない。
///
/// ```text
/// cd src-tauri && cargo test -- --ignored real_huggingface --nocapture
/// ```
#[tokio::test]
#[ignore = "外部ネットワークに接続する(Hugging Face)"]
async fn real_huggingface_download_of_the_smallest_model() {
    let policy = DownloadPolicy::production();
    let catalog = Catalog::embedded(&policy).unwrap();
    let sink = Arc::new(TestSink::default());
    let svc = ModelService::new(
        Db::open_in_memory().unwrap(),
        Arc::new(EngineService::new("http://127.0.0.1:1").unwrap()),
        Arc::new(catalog),
        policy,
        Arc::clone(&sink) as Arc<dyn ModelSink>,
    )
    .unwrap();
    let models = TmpDir::new();
    svc.set_dir(Some(models.0.to_string_lossy().into_owned()))
        .await
        .unwrap();
    tokio::spawn(Arc::clone(&svc).run_worker());

    svc.start("detector-face", "yolov8n-v2", false)
        .await
        .unwrap();
    let done = tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            if let Some(d) = sink.done_states().first() {
                return d.clone();
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("実際のダウンロードが終わらない");

    assert_eq!(done.status, Status::Done, "{:?}", done.error);
    let placed = models
        .0
        .join("ultralytics")
        .join("bbox")
        .join("face_yolov8n_v2.pt");
    assert!(placed.is_file());
    assert!(svc.verify("detector-face", "yolov8n-v2").await.unwrap());
    println!(
        "downloaded {} bytes -> {}",
        done.total_bytes,
        placed.display()
    );
}

/// 実機検証: Flux の VAE(約 335MB)を、認証不要のミラー(Comfy-Org)から取得できるか。
/// 公式(Black Forest Labs)の `ae.safetensors` と同一の SHA-256 であることは、取得後の照合で確かめられる。
#[tokio::test]
#[ignore = "外部ネットワークに接続する(Hugging Face。約 335MB)"]
async fn real_huggingface_flux_vae_matches_the_official_hash() {
    let policy = DownloadPolicy::production();
    let catalog = Catalog::embedded(&policy).unwrap();
    let sink = Arc::new(TestSink::default());
    let svc = ModelService::new(
        Db::open_in_memory().unwrap(),
        Arc::new(EngineService::new("http://127.0.0.1:1").unwrap()),
        Arc::new(catalog),
        policy,
        Arc::clone(&sink) as Arc<dyn ModelSink>,
    )
    .unwrap();
    let models = TmpDir::new();
    svc.set_dir(Some(models.0.to_string_lossy().into_owned()))
        .await
        .unwrap();
    tokio::spawn(Arc::clone(&svc).run_worker());

    svc.start("flux-vae", "ae", false).await.unwrap();
    let done = tokio::time::timeout(Duration::from_secs(600), async {
        loop {
            if let Some(d) = sink.done_states().first() {
                return d.clone();
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("実際のダウンロードが終わらない");

    assert_eq!(done.status, Status::Done, "{:?}", done.error);
    assert!(models.0.join("vae").join("ae.safetensors").is_file());
    assert!(svc.verify("flux-vae", "ae").await.unwrap());
}

#[test]
fn engine_dir_status_detects_mismatch_and_blocks() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let argv = if cfg!(windows) {
        s(&[r"C:\w\ComfyUI\main.py", "--base-directory", r"C:\w\data"])
    } else {
        s(&["/w/ComfyUI/main.py", "--base-directory", "/w/data"])
    };
    let (same, other) = if cfg!(windows) {
        (r"C:\w\data\models", r"D:\models")
    } else {
        ("/w/data/models", "/other/models")
    };

    // 保存先が ComfyUI の参照先と同じ → 注意なし
    let st = engine_dir_status_of(Some(Path::new(same)), Some(&argv), true);
    assert!(!st.mismatch && !st.applied && st.can_apply);

    // 異なる → 未反映の食い違い
    let st = engine_dir_status_of(Some(Path::new(other)), Some(&argv), true);
    assert!(st.mismatch && st.can_apply && st.engine_models_dir.is_some());

    // 自動検出(設定なし)は反映できない
    let st = engine_dir_status_of(None, Some(&argv), true);
    assert_eq!(st.reason, Some(ApplyBlock::NoSetting));
    assert!(!st.mismatch);

    // リモート・場所不明
    let st = engine_dir_status_of(Some(Path::new(other)), None, false);
    assert_eq!(st.reason, Some(ApplyBlock::Remote));
    let st = engine_dir_status_of(Some(Path::new(other)), None, true);
    assert_eq!(st.reason, Some(ApplyBlock::EngineUnknown));

    // --extra-model-paths-config があれば断定しない
    let mut custom = argv.clone();
    custom.push("--extra-model-paths-config".into());
    let st = engine_dir_status_of(Some(Path::new(other)), Some(&custom), true);
    assert!(st.has_custom_config && !st.mismatch);
}
