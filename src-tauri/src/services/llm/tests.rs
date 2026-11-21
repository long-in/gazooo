//! ローカル LLM の管理のテスト。実際の `llama-server` は使わず、小さな外部コマンドと
//! ローカルのテスト用 HTTP サーバで、導入・起動・自動停止・解放の挙動を確認する。

use std::io::Write;
use std::path::Path;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use super::*;
use crate::services::models::testserver::{Reply, spawn};

#[derive(Default)]
struct TestSink {
    states: StdMutex<Vec<LlmState>>,
    installs: StdMutex<Vec<InstallState>>,
}

impl LlmSink for TestSink {
    fn state(&self, state: LlmState, _message: Option<&str>) {
        self.states.lock().unwrap().push(state);
    }
    fn install(&self, state: &InstallState) {
        self.installs.lock().unwrap().push(state.clone());
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gazooo-llm-{name}-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn probe(flag: Arc<AtomicBool>) -> Probe {
    Arc::new(move |_| {
        let ok = flag.load(Ordering::SeqCst);
        Box::pin(async move { ok })
    })
}

/// 長く生き続けるコマンド(停止されるまで)。
fn long_running() -> LaunchSpec {
    let (program, args): (&str, &[&str]) = if cfg!(windows) {
        ("ping", &["-n", "60", "127.0.0.1"])
    } else {
        ("sleep", &["60"])
    };
    LaunchSpec {
        program: PathBuf::from(program),
        args: args.iter().map(Into::into).collect(),
        cwd: None,
    }
}

/// すぐ異常終了するコマンド。
fn exits_with_3() -> LaunchSpec {
    let (program, args): (&str, &[&str]) = if cfg!(windows) {
        ("cmd", &["/c", "exit 3"])
    } else {
        ("sh", &["-c", "exit 3"])
    };
    LaunchSpec {
        program: PathBuf::from(program),
        args: args.iter().map(Into::into).collect(),
        cwd: None,
    }
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

struct Harness {
    svc: Arc<LlmService>,
    sink: Arc<TestSink>,
    db: Db,
    root: PathBuf,
}

fn harness(builder: SpecBuilder, healthy: Arc<AtomicBool>) -> Harness {
    let root = tmp("root");
    let db = Db::open_in_memory().unwrap();
    let sink = Arc::new(TestSink::default());
    let svc = LlmService::with_options(
        db.clone(),
        root.clone(),
        Manifest::embedded().unwrap(),
        Some(Platform::WindowsX64),
        sink.clone(),
        None,
        probe(healthy),
        Duration::from_secs(10),
        builder,
        None,
    );
    Harness {
        svc,
        sink,
        db,
        root,
    }
}

/// 導入済みの状態(ファイルと記録)を作る。
fn fake_install(h: &Harness) {
    let m = &h.svc.manifest;
    let exe = h
        .svc
        .layout
        .bin_dir(&m.runtime.build)
        .join(&m.runtime.windows_x64.exe);
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"exe").unwrap();
    std::fs::create_dir_all(h.svc.layout.models_dir()).unwrap();
    std::fs::write(h.svc.layout.models_dir().join(&m.model.file_name), b"gguf").unwrap();
    let rec = InstallRecord {
        manifest_version: m.manifest_version.clone(),
        runtime_build: m.runtime.build.clone(),
        runtime_sha256: m.runtime.windows_x64.source.sha256.clone(),
        model_file: m.model.file_name.clone(),
        model_sha256: m.model.source.sha256.clone(),
    };
    std::fs::write(h.svc.layout.marker(), serde_json::to_string(&rec).unwrap()).unwrap();
}

fn running_harness() -> Harness {
    let healthy = Arc::new(AtomicBool::new(true));
    let h = harness(Box::new(|_| long_running()), healthy);
    fake_install(&h);
    h
}

async fn set_settings(h: &Harness, idle: Option<u32>, release: Option<bool>) {
    h.svc.update_settings(idle, release).await.unwrap();
}

// ---------------------------------------------------------------- 起動の引数

#[test]
fn launch_args_bind_loopback_and_limit_memory() {
    let mut p = LaunchParams {
        exe: PathBuf::from("bin/llama-server.exe"),
        model: PathBuf::from("models/m.gguf"),
        port: 12345,
        gpu_layers: None,
    };
    let spec = default_spec(&p);
    let args: Vec<String> = spec
        .args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let at = |flag: &str| args.iter().position(|a| a == flag).unwrap();
    assert_eq!(args[at("--host") + 1], "127.0.0.1");
    assert_eq!(args[at("--port") + 1], "12345");
    assert_eq!(args[at("-np") + 1], "1");
    // プロンプトのキャッシュをメモリに溜めない
    assert_eq!(args[at("-cram") + 1], "0");
    assert!(args.contains(&"--jinja".to_string()));
    assert!(args.contains(&r#"{"enable_thinking":false}"#.to_string()));
    assert!(!args.contains(&"-ngl".to_string()));

    p.gpu_layers = Some(0);
    let args: Vec<String> = default_spec(&p)
        .args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let i = args.iter().position(|a| a == "-ngl").unwrap();
    assert_eq!(args[i + 1], "0");
}

// ---------------------------------------------------------------- 状態

#[tokio::test]
async fn status_reflects_install_and_update_availability() {
    let h = harness(
        Box::new(|_| long_running()),
        Arc::new(AtomicBool::new(true)),
    );
    let s = h.svc.status().await.unwrap();
    assert!(s.supported && !s.installed);
    assert_eq!(s.state, LlmState::NotInstalled);
    assert!(s.download_mb > 5000);
    assert_eq!((s.idle_minutes, s.release_before_generation), (5, true));

    fake_install(&h);
    let s = h.svc.status().await.unwrap();
    assert!(s.installed && !s.update_available);
    assert_eq!(s.state, LlmState::Stopped);
    assert!(s.model.unwrap().contains("Qwen3.5-9B"));

    // マニフェストと記録が食い違えば更新できる
    let mut rec = h.svc.layout.load_record().unwrap();
    rec.runtime_build = "b1".into();
    std::fs::write(h.svc.layout.marker(), serde_json::to_string(&rec).unwrap()).unwrap();
    // 記録のビルドのファイルが無いので「導入済み」ではなくなる(再導入で直る)
    assert!(!h.svc.status().await.unwrap().installed);
}

#[tokio::test]
async fn settings_are_validated_and_persisted() {
    let h = running_harness();
    let s = h.svc.update_settings(Some(15), Some(false)).await.unwrap();
    assert_eq!((s.idle_minutes, s.release_before_generation), (15, false));
    let err = h.svc.update_settings(Some(7), None).await.unwrap_err();
    assert_eq!(err.code(), "VALIDATION");
    assert_eq!(h.svc.status().await.unwrap().idle_minutes, 15);
}

// ---------------------------------------------------------------- 起動・停止

#[tokio::test]
async fn ensure_ready_requires_install() {
    let h = harness(
        Box::new(|_| long_running()),
        Arc::new(AtomicBool::new(true)),
    );
    let err = h.svc.ensure_ready_endpoint().await.unwrap_err();
    assert_eq!(err.code(), "LLM_NOT_INSTALLED");
}

#[tokio::test]
async fn ensure_ready_starts_once_and_reuses_the_process() {
    let h = running_harness();
    let a = h.svc.ensure_ready_endpoint().await.unwrap();
    assert!(a.base_url.starts_with("http://127.0.0.1:") && a.base_url.ends_with("/v1"));
    assert_eq!(a.model, MODEL_ALIAS);
    let pid = h.svc.proc.state().pid;
    let b = h.svc.ensure_ready_endpoint().await.unwrap();
    assert_eq!(a, b);
    assert_eq!(h.svc.proc.state().pid, pid);
    assert_eq!(h.svc.status().await.unwrap().state, LlmState::Ready);

    let s = h.svc.stop().await.unwrap();
    assert_eq!(s.state, LlmState::Stopped);
    assert!(!h.svc.proc.has_child());
}

#[tokio::test]
async fn retries_once_without_gpu_offload_then_succeeds() {
    let healthy = Arc::new(AtomicBool::new(false));
    let seen: Arc<StdMutex<Vec<Option<u32>>>> = Arc::default();
    let (healthy2, seen2) = (Arc::clone(&healthy), Arc::clone(&seen));
    let h = harness(
        Box::new(move |p| {
            seen2.lock().unwrap().push(p.gpu_layers);
            if p.gpu_layers == Some(0) {
                healthy2.store(true, Ordering::SeqCst);
                long_running()
            } else {
                exits_with_3()
            }
        }),
        healthy,
    );
    fake_install(&h);
    h.svc.ensure_ready_endpoint().await.unwrap();
    assert_eq!(*seen.lock().unwrap(), [None, Some(0)]);
    h.svc.stop().await.unwrap();
}

#[tokio::test]
async fn reports_start_failure_after_the_retry() {
    let h = harness(
        Box::new(|_| exits_with_3()),
        Arc::new(AtomicBool::new(false)),
    );
    fake_install(&h);
    let err = h.svc.ensure_ready_endpoint().await.unwrap_err();
    assert_eq!(err.code(), "LLM_START_FAILED");
    // 失敗後も、また起動を試せる(状態が詰まらない)
    assert_eq!(
        h.svc.ensure_ready_endpoint().await.unwrap_err().code(),
        "LLM_START_FAILED"
    );
}

// ---------------------------------------------------------------- メモリの解放

#[tokio::test]
async fn idle_timeout_stops_the_process_and_frees_memory() {
    let h = running_harness();
    h.svc.ensure_ready_endpoint().await.unwrap();
    // 設定は 5 分。4 分ではまだ止めない
    let now = Instant::now();
    assert!(!h.svc.idle_tick(now + Duration::from_secs(4 * 60)).await);
    assert!(h.svc.proc.has_child());
    // 5 分を超えたら止める(プロセスが無くなる = メモリが OS に返る)
    assert!(h.svc.idle_tick(now + Duration::from_secs(5 * 60 + 5)).await);
    assert!(!h.svc.proc.has_child());
    assert_eq!(h.svc.status().await.unwrap().state, LlmState::Stopped);
    // 止まっていれば何もしない
    assert!(!h.svc.idle_tick(now + Duration::from_secs(3600)).await);
}

#[tokio::test]
async fn idle_timeout_respects_settings_and_active_use() {
    let h = running_harness();
    h.svc.ensure_ready_endpoint().await.unwrap();
    let far = Instant::now() + Duration::from_secs(3 * 3600);

    // 回答の生成中は止めない
    h.svc.use_started();
    assert!(!h.svc.idle_tick(far).await);
    h.svc.use_finished();

    // 「無効」なら止めない
    set_settings(&h, Some(0), None).await;
    assert!(!h.svc.idle_tick(far).await);
    assert!(h.svc.proc.has_child());

    // 1 分にすると、すぐ反映される
    set_settings(&h, Some(1), None).await;
    assert!(h.svc.idle_tick(far).await);
    assert!(!h.svc.proc.has_child());
}

#[tokio::test]
async fn releases_before_generation_when_enabled() {
    let h = running_harness();
    // 止まっているときは何もしない
    assert!(!h.svc.release_for_generation().await);

    h.svc.ensure_ready_endpoint().await.unwrap();
    assert!(h.svc.release_for_generation().await);
    assert!(!h.svc.proc.has_child());
}

#[tokio::test]
async fn does_not_release_when_disabled_or_answering() {
    let h = running_harness();
    h.svc.ensure_ready_endpoint().await.unwrap();

    set_settings(&h, None, Some(false)).await;
    assert!(!h.svc.release_for_generation().await);
    assert!(h.svc.proc.has_child());

    set_settings(&h, None, Some(true)).await;
    h.svc.use_started();
    assert!(!h.svc.release_for_generation().await);
    assert!(h.svc.proc.has_child());
    h.svc.use_finished();

    assert!(h.svc.release_for_generation().await);
    assert!(!h.svc.proc.has_child());
}

#[tokio::test]
async fn shutdown_stops_the_process() {
    let h = running_harness();
    h.svc.ensure_ready_endpoint().await.unwrap();
    h.svc.shutdown().await;
    assert!(!h.svc.proc.has_child());
}

// ---------------------------------------------------------------- 削除

#[tokio::test]
async fn remove_refuses_while_running_then_deletes_everything() {
    let h = running_harness();
    h.svc.ensure_ready_endpoint().await.unwrap();
    assert_eq!(h.svc.remove().await.unwrap_err().code(), "LLM_BUSY");
    assert!(h.root.exists());

    h.svc.stop().await.unwrap();
    let s = h.svc.remove().await.unwrap();
    assert!(!s.installed);
    assert_eq!(s.state, LlmState::NotInstalled);
    assert!(!h.root.exists());
    // 何も無くても失敗しない
    h.svc.remove().await.unwrap();
    let _ = &h.db;
    let _ = &h.sink;
}

// ---------------------------------------------------------------- 導入

fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }
    buf.into_inner()
}

/// テスト用のサーバから取得するマニフェスト。
fn served_manifest(base: &str, zip: &[u8], model: &[u8]) -> Manifest {
    let mut m = Manifest::embedded().unwrap();
    m.runtime.windows_x64.source.url = format!("{base}/llama.zip");
    m.runtime.windows_x64.source.sha256 = sha256_hex(zip);
    m.runtime.windows_x64.source.size_bytes = zip.len() as u64;
    m.model.source.url = format!("{base}/model.gguf");
    m.model.source.sha256 = sha256_hex(model);
    m.model.source.size_bytes = model.len() as u64;
    m
}

fn installer_service(root: &Path, m: Manifest) -> (Arc<LlmService>, Arc<TestSink>) {
    let sink = Arc::new(TestSink::default());
    let svc = LlmService::with_options(
        Db::open_in_memory().unwrap(),
        root.to_path_buf(),
        m,
        Some(Platform::WindowsX64),
        sink.clone(),
        None,
        probe(Arc::new(AtomicBool::new(true))),
        Duration::from_secs(10),
        Box::new(|_| long_running()),
        Some(DownloadPolicy::for_test("127.0.0.1")),
    );
    (svc, sink)
}

async fn wait_install(sink: &TestSink, want: InstallStatus) -> InstallState {
    for _ in 0..300 {
        if let Some(s) = sink
            .installs
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|s| s.status == want)
        {
            return s.clone();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("install status {want:?} に到達しなかった");
}

#[tokio::test]
async fn installs_runtime_and_model_and_records_them() {
    let zip = make_zip(&[("llama-server.exe", b"EXE"), ("ggml.dll", b"DLL")]);
    let model = vec![7u8; 4096];
    let server = spawn(vec![
        (
            "/llama.zip",
            Reply::Body {
                body: zip.clone(),
                ranges: true,
            },
        ),
        (
            "/model.gguf",
            Reply::Body {
                body: model.clone(),
                ranges: true,
            },
        ),
    ])
    .await;
    let root = tmp("install");
    let m = served_manifest(&server.base, &zip, &model);
    let (svc, sink) = installer_service(&root.join("llm"), m.clone());

    let id = svc.install_start().await.unwrap();
    let done = wait_install(&sink, InstallStatus::Done).await;
    assert_eq!(done.install_id.as_deref(), Some(id.as_str()));

    assert!(svc.is_installed());
    let rec = svc.layout.load_record().unwrap();
    assert_eq!(rec.model_sha256, sha256_hex(&model));
    assert!(
        svc.layout
            .bin_dir(&m.runtime.build)
            .join("llama-server.exe")
            .is_file()
    );
    assert_eq!(
        std::fs::read(svc.layout.models_dir().join(&m.model.file_name)).unwrap(),
        model
    );
    // 取得した圧縮ファイルは残さない
    assert!(
        std::fs::read_dir(svc.layout.downloads_dir())
            .map(|d| d
                .flatten()
                .all(|e| !e.file_name().to_string_lossy().ends_with(".zip")))
            .unwrap_or(true)
    );
    // 進捗は手順(runtime → extract → model)の順に通知される
    let steps: Vec<String> = sink
        .installs
        .lock()
        .unwrap()
        .iter()
        .filter_map(|s| s.step_id.clone())
        .fold(Vec::new(), |mut v, s| {
            if v.last() != Some(&s) {
                v.push(s);
            }
            v
        });
    assert_eq!(steps, ["runtime", "extract", "model"]);

    // 再実行は取り直さない(ファイルが検証済みのため)
    let before = server.requests.lock().unwrap().len();
    svc.install_start().await.unwrap();
    for _ in 0..100 {
        if sink
            .installs
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.status == InstallStatus::Done)
            .count()
            >= 2
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(server.requests.lock().unwrap().len(), before);
}

#[tokio::test]
async fn hash_mismatch_is_rejected_and_nothing_is_recorded() {
    let zip = make_zip(&[("llama-server.exe", b"EXE")]);
    let model = vec![1u8; 1024];
    let server = spawn(vec![
        (
            "/llama.zip",
            Reply::Body {
                body: zip.clone(),
                ranges: true,
            },
        ),
        // 期待と違う内容を返す
        (
            "/model.gguf",
            Reply::Body {
                body: vec![2u8; 1024],
                ranges: true,
            },
        ),
    ])
    .await;
    let root = tmp("mismatch");
    let (svc, sink) = installer_service(
        &root.join("llm"),
        served_manifest(&server.base, &zip, &model),
    );

    svc.install_start().await.unwrap();
    let failed = wait_install(&sink, InstallStatus::Failed).await;
    assert_eq!(failed.error.unwrap().code, "LLM_INSTALL_HASH_MISMATCH");
    assert!(!svc.is_installed());
    assert!(svc.layout.load_record().is_none());
}

#[tokio::test]
async fn install_can_be_cancelled() {
    let zip = make_zip(&[("llama-server.exe", b"EXE")]);
    let model = vec![3u8; 64 * 1024];
    let server = spawn(vec![
        (
            "/llama.zip",
            Reply::Body {
                body: zip.clone(),
                ranges: true,
            },
        ),
        (
            "/model.gguf",
            Reply::Slow {
                body: model.clone(),
                chunk: 1024,
                delay: Duration::from_millis(50),
            },
        ),
    ])
    .await;
    let root = tmp("cancel");
    let (svc, sink) = installer_service(
        &root.join("llm"),
        served_manifest(&server.base, &zip, &model),
    );

    let id = svc.install_start().await.unwrap();
    // 二重の開始は拒否される
    assert_eq!(svc.install_start().await.unwrap_err().code(), "LLM_BUSY");
    for _ in 0..100 {
        if sink
            .installs
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.step_id.as_deref() == Some("model") && s.received.unwrap_or(0) > 0)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    svc.install_cancel(&id);
    wait_install(&sink, InstallStatus::Cancelled).await;
    assert!(svc.layout.load_record().is_none());
}

#[tokio::test]
async fn install_is_refused_on_unsupported_platforms_and_while_running() {
    let root = tmp("unsupported");
    let sink = Arc::new(TestSink::default());
    let svc = LlmService::with_options(
        Db::open_in_memory().unwrap(),
        root.join("llm"),
        Manifest::embedded().unwrap(),
        None,
        sink,
        None,
        probe(Arc::new(AtomicBool::new(true))),
        Duration::from_secs(10),
        Box::new(|_| long_running()),
        None,
    );
    assert!(!svc.status().await.unwrap().supported);
    assert_eq!(
        svc.install_start().await.unwrap_err().code(),
        "LLM_INSTALL_FAILED"
    );

    let h = running_harness();
    h.svc.ensure_ready_endpoint().await.unwrap();
    assert_eq!(h.svc.install_start().await.unwrap_err().code(), "LLM_BUSY");
    h.svc.stop().await.unwrap();
}

// ---------------------------------------------------------------- 実機(手動)

/// 実際の `llama-server` と Qwen3.5-9B で、起動 → 相談(ツール呼び出しを含む)→ アイドル停止までを確認する。
///
/// 事前に、固定マニフェストと同じファイルを用意しておく(導入そのものは別途 `llm_install` で確認する):
/// - `GAZOOO_LLM_E2E_RUNTIME_DIR`: `llama-server` と DLL が入ったフォルダ(Windows)
/// - `GAZOOO_LLM_E2E_MODEL`: `Qwen3.5-9B-Q4_K_M.gguf`
///
/// `cargo test --lib real_llama_server -- --ignored --nocapture`
#[tokio::test]
#[ignore = "needs a real llama-server and the Qwen3.5-9B GGUF (env vars)"]
async fn real_llama_server_answers_and_frees_memory() {
    use crate::domain::params::{ModelKind, sample};
    use crate::services::assistant::anthropic::AnthropicClient;
    use crate::services::assistant::{AssistantPatch, AssistantService, AssistantSink};

    #[derive(Default)]
    struct Rec(StdMutex<Vec<String>>);
    impl AssistantSink for Rec {
        fn delta(&self, _: &str, t: &str) {
            self.0.lock().unwrap().push(format!("delta:{t}"));
        }
        fn suggestion(&self, _: &str, i: usize, s: &crate::db::assistant::StoredSuggestion) {
            self.0
                .lock()
                .unwrap()
                .push(format!("suggestion:{i}:{}", s.summary));
        }
        fn done(&self, _: &str, c: bool) {
            self.0.lock().unwrap().push(format!("done:{c}"));
        }
        fn error(&self, _: &str, e: &AppError) {
            self.0
                .lock()
                .unwrap()
                .push(format!("error:{}:{e}", e.code()));
        }
    }

    let runtime_dir = PathBuf::from(std::env::var("GAZOOO_LLM_E2E_RUNTIME_DIR").unwrap());
    let model_src = PathBuf::from(std::env::var("GAZOOO_LLM_E2E_MODEL").unwrap());

    let root = tmp("e2e");
    let m = Manifest::embedded().unwrap();
    let bin = root.join("bin").join(&m.runtime.build);
    std::fs::create_dir_all(&bin).unwrap();
    for e in std::fs::read_dir(&runtime_dir).unwrap().flatten() {
        if e.path().is_file() {
            std::fs::copy(e.path(), bin.join(e.file_name())).unwrap();
        }
    }
    std::fs::create_dir_all(root.join("models")).unwrap();
    // 数 GB のコピーを避ける(同じボリューム上のハードリンク)
    std::fs::hard_link(&model_src, root.join("models").join(&m.model.file_name)).unwrap();
    let rec = InstallRecord {
        manifest_version: m.manifest_version.clone(),
        runtime_build: m.runtime.build.clone(),
        runtime_sha256: m.runtime.windows_x64.source.sha256.clone(),
        model_file: m.model.file_name.clone(),
        model_sha256: m.model.source.sha256.clone(),
    };
    std::fs::write(
        root.join(".gazooo-llm.json"),
        serde_json::to_string(&rec).unwrap(),
    )
    .unwrap();

    let db = Db::open_in_memory().unwrap();
    let svc = LlmService::new(
        db.clone(),
        root.clone(),
        m,
        Arc::new(TestSink::default()),
        None,
    );
    assert!(svc.is_installed());

    let sink = Arc::new(Rec::default());
    let assistant =
        AssistantService::new(db.clone(), AnthropicClient::new().unwrap(), sink.clone());
    assistant.set_runtime(svc.clone());
    assistant
        .update(AssistantPatch {
            provider: Some(crate::db::assistant::Provider::Local),
            ..Default::default()
        })
        .await
        .unwrap();

    let started = Instant::now();
    assistant
        .ask(
            None,
            "顔が崩れます。steps を 40 にして、ネガティブに bad anatomy を足す提案をしてください。"
                .into(),
            sample(ModelKind::Sdxl),
        )
        .await
        .unwrap();
    for _ in 0..1800 {
        if sink
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("done:") || l.starts_with("error:"))
        {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let log = sink.0.lock().unwrap().clone();
    println!("elapsed: {:?}", started.elapsed());
    let text: String = log
        .iter()
        .filter_map(|l| l.strip_prefix("delta:"))
        .collect();
    println!("answer: {text}");
    println!(
        "events: {:?}",
        log.iter()
            .filter(|l| !l.starts_with("delta:"))
            .collect::<Vec<_>>()
    );
    assert!(log.contains(&"done:false".to_string()), "{log:?}");
    assert!(!text.trim().is_empty());
    assert!(!text.contains("<think>"));
    assert_eq!(svc.status().await.unwrap().state, LlmState::Ready);

    // アイドル停止 → プロセスが消える
    assert!(
        svc.idle_tick(Instant::now() + Duration::from_secs(10 * 60))
            .await
    );
    assert!(!svc.proc.has_child());
    assert_eq!(svc.status().await.unwrap().state, LlmState::Stopped);
}

#[test]
fn mt_spec_uses_the_translate_alias_and_no_qwen_thinking_switch() {
    let p = LaunchParams {
        exe: PathBuf::from("llama-server"),
        model: PathBuf::from("plamo.gguf"),
        port: 18080,
        gpu_layers: None,
    };
    let args: Vec<String> = mt_spec(&p)
        .args
        .iter()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    assert!(args.contains(&"--no-prefill-assistant".to_string()));
    assert!(args.contains(&MT_MODEL_ALIAS.to_string()));
    assert!(args.contains(&"4096".to_string()));
    assert!(!args.iter().any(|a| a.contains("enable_thinking")));
    // 本体の起動引数は変えていない
    let main: Vec<String> = default_spec(&p)
        .args
        .iter()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    assert!(main.iter().any(|a| a.contains("enable_thinking")));
}
