use super::*;

#[derive(Default)]
struct TestSink {
    states: Mutex<Vec<InstallState>>,
    lines: Mutex<Vec<String>>,
}

impl InstallSink for TestSink {
    fn state(&self, state: &InstallState) {
        self.states.lock().unwrap().push(state.clone());
    }
    fn log(&self, line: &str) {
        self.lines.lock().unwrap().push(line.to_string());
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gazooo-inst-{name}-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn installer(root: &Path, platform: Option<Platform>) -> (Arc<EngineInstaller>, Arc<TestSink>) {
    let sink = Arc::new(TestSink::default());
    let m = Manifest::embedded().unwrap();
    let policy = m.policy();
    (
        EngineInstaller::with_options(root.to_path_buf(), m, platform, policy, sink.clone()),
        sink,
    )
}

fn noop_finalize() -> Finalize {
    Arc::new(|_| Box::pin(async { Ok(()) }))
}

fn echo_command() -> (PathBuf, Vec<std::ffi::OsString>) {
    if cfg!(windows) {
        (
            PathBuf::from("cmd"),
            vec!["/C".into(), "echo hello-install".into()],
        )
    } else {
        (
            PathBuf::from("sh"),
            vec!["-c".into(), "echo hello-install".into()],
        )
    }
}

#[test]
fn layout_matches_the_08_arrangement() {
    let l = Layout {
        root: PathBuf::from("root"),
    };
    assert_eq!(l.comfyui_dir(), Path::new("root").join("ComfyUI"));
    assert_eq!(l.venv_dir(), Path::new("root").join("venv"));
    assert_eq!(l.data_dir(), Path::new("root").join("data"));
    assert!(
        l.venv_python(Platform::WindowsX64)
            .ends_with("Scripts/python.exe")
    );
    assert!(l.venv_python(Platform::MacosArm64).ends_with("bin/python"));
    assert!(
        l.python_exe(Platform::WindowsX64)
            .ends_with("python/python.exe")
    );
}

#[test]
fn managed_config_points_into_the_install_dir() {
    let d = tmp("cfg");
    let l = Layout { root: d.clone() };
    let cfg = l.managed_config(Platform::WindowsX64);
    assert!(cfg.enabled && !cfg.auto_start);
    assert_eq!(cfg.port, DEFAULT_PORT);
    assert!(Path::new(&cfg.main_py).starts_with(&d));
    assert!(Path::new(&cfg.python).starts_with(&d));
    assert!(Path::new(cfg.base_directory.as_deref().unwrap()).starts_with(&d));
}

#[test]
fn record_roundtrips_and_tolerates_garbage() {
    let d = tmp("rec");
    let l = Layout { root: d.clone() };
    assert!(l.load_record().is_none());
    let mut r = InstallRecord {
        manifest_version: "x".into(),
        components: vec!["gguf".into()],
        complete: true,
        ..Default::default()
    };
    r.steps.insert("python".into(), "abc".into());
    l.save_record(&r).unwrap();
    assert_eq!(l.load_record().unwrap(), r);
    std::fs::write(l.marker(), "not json").unwrap();
    assert!(l.load_record().is_none());
}

#[test]
fn path_with_spaces_and_japanese_is_supported() {
    let base = tmp("jp");
    let root = base.join("ユーザー 名").join("comfyui");
    let l = Layout { root: root.clone() };
    l.save_record(&InstallRecord::default()).unwrap();
    assert!(l.load_record().is_some());
    assert!(root.join(MARKER).is_file());
}

#[tokio::test]
async fn preflight_reports_space_and_platform() {
    let d = tmp("pre");
    let (i, _) = installer(&d.join("comfyui"), Some(Platform::MacosArm64));
    let p = i.preflight(&["gguf".into(), "face".into()]).await.unwrap();
    assert!(p.supported);
    assert_eq!(p.gpu, GpuStatus::NotApplicable);
    let base = BASE_REQUIRED_MB * 1024 * 1024;
    assert!(
        p.required_bytes > base,
        "選択したコンポーネント分が加算される"
    );
    assert!(p.free_bytes.is_some());

    let (i, _) = installer(&d.join("comfyui"), None);
    let p = i.preflight(&[]).await.unwrap();
    assert!(!p.supported && p.unsupported_reason.is_some());
}

#[tokio::test]
async fn start_requires_consent_a_known_component_and_a_supported_os() {
    let d = tmp("start");
    let (i, _) = installer(&d.join("c"), Some(Platform::MacosArm64));
    let no_consent = StartRequest {
        components: vec![],
        accepted: false,
    };
    assert_eq!(
        i.start(no_consent, noop_finalize())
            .await
            .unwrap_err()
            .code(),
        "VALIDATION"
    );
    let unknown = StartRequest {
        components: vec!["nope".into()],
        accepted: true,
    };
    assert_eq!(
        i.start(unknown, noop_finalize()).await.unwrap_err().code(),
        "VALIDATION"
    );

    let (i, _) = installer(&d.join("c"), None);
    let ok = StartRequest {
        components: vec![],
        accepted: true,
    };
    assert_eq!(
        i.start(ok, noop_finalize()).await.unwrap_err().code(),
        "ENGINE_INSTALL_UNSUPPORTED"
    );
    assert_eq!(i.state().status, InstallStatus::Idle);
}

#[tokio::test]
async fn run_command_streams_output_to_the_log() {
    let d = tmp("cmd");
    let (i, sink) = installer(&d, Some(Platform::WindowsX64));
    let (prog, args) = echo_command();
    let (_tx, mut rx) = watch::channel(CancelMode::Run);
    i.run_command(&prog, &args, &d, &mut rx).await.unwrap();
    assert!(i.logs(50).iter().any(|l| l.contains("hello-install")));
    assert!(
        sink.lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("hello-install"))
    );
}

#[tokio::test]
async fn run_command_reports_failure_and_missing_program() {
    let d = tmp("cmdfail");
    let (i, _) = installer(&d, Some(Platform::WindowsX64));
    let (_tx, mut rx) = watch::channel(CancelMode::Run);
    let fail_args: (PathBuf, Vec<std::ffi::OsString>) = if cfg!(windows) {
        ("cmd".into(), vec!["/C".into(), "exit 3".into()])
    } else {
        ("sh".into(), vec!["-c".into(), "exit 3".into()])
    };
    let e = i
        .run_command(&fail_args.0, &fail_args.1, &d, &mut rx)
        .await
        .unwrap_err();
    assert_eq!(e.code(), "ENGINE_INSTALL_COMMAND_FAILED");
    assert!(e.to_string().contains('3'));
    let e = i
        .run_command(&d.join("no-such-program"), &[], &d, &mut rx)
        .await
        .unwrap_err();
    assert_eq!(e.code(), "ENGINE_INSTALL_COMMAND_FAILED");
}

#[tokio::test]
async fn run_command_can_be_cancelled() {
    let d = tmp("cmdcancel");
    let (i, _) = installer(&d, Some(Platform::WindowsX64));
    let (tx, mut rx) = watch::channel(CancelMode::Run);
    let (prog, args): (PathBuf, Vec<std::ffi::OsString>) = if cfg!(windows) {
        (
            "ping".into(),
            vec!["-n".into(), "30".into(), "127.0.0.1".into()],
        )
    } else {
        ("sleep".into(), vec!["30".into()])
    };
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = tx.send(CancelMode::Cancel);
        tx
    });
    let started = Instant::now();
    let e = i.run_command(&prog, &args, &d, &mut rx).await.unwrap_err();
    assert!(is_cancelled(&e));
    assert!(started.elapsed() < Duration::from_secs(10));
    let _ = canceller.await;
}

#[tokio::test]
async fn remove_keeps_or_deletes_data() {
    let d = tmp("rm");
    let root = d.join("comfyui");
    let l = Layout { root: root.clone() };
    for sub in ["python", "venv", "ComfyUI", "downloads", "data/models"] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    std::fs::write(root.join("data/models/m.safetensors"), "x").unwrap();
    l.save_record(&InstallRecord::default()).unwrap();

    let (i, _) = installer(&root, Some(Platform::WindowsX64));
    i.remove(true).await.unwrap();
    assert!(!root.join("python").exists() && !root.join("ComfyUI").exists());
    assert!(!l.marker().exists());
    assert!(
        root.join("data/models/m.safetensors").is_file(),
        "data は残る"
    );

    i.remove(false).await.unwrap();
    assert!(!root.exists());
    // 無いものを消してもエラーにならない
    i.remove(false).await.unwrap();
}

#[test]
fn replace_dir_overwrites_existing() {
    let d = tmp("replace");
    std::fs::create_dir_all(d.join("new")).unwrap();
    std::fs::write(d.join("new/a.txt"), "new").unwrap();
    std::fs::create_dir_all(d.join("dest")).unwrap();
    std::fs::write(d.join("dest/old.txt"), "old").unwrap();
    replace_dir(&d.join("new"), &d.join("dest")).unwrap();
    assert!(d.join("dest/a.txt").is_file() && !d.join("dest/old.txt").exists());
    assert!(!d.join("new").exists());
}

#[test]
fn missing_nodes_lists_what_is_absent() {
    let body = br#"{"KSampler":{},"UnetLoaderGGUF":{}}"#;
    let expect = vec!["UnetLoaderGGUF".to_string(), "FaceDetailer".to_string()];
    assert_eq!(
        missing_nodes(body, &expect).unwrap(),
        vec!["FaceDetailer".to_string()]
    );
    assert!(missing_nodes(b"not json", &expect).is_err());
}

#[test]
fn torch_args_are_fixed_and_use_the_cuda_index_only_on_windows() {
    let d = tmp("torch");
    let (i, _) = installer(&d, Some(Platform::WindowsX64));
    let win = display_args(&i.torch_args(Platform::WindowsX64));
    assert!(
        win.contains("torch==2.14.1")
            && win.contains("--index-url https://download.pytorch.org/whl/cu130")
    );
    assert!(!win.contains("--extra-index-url"));
    let mac = display_args(&i.torch_args(Platform::MacosArm64));
    assert!(mac.contains("torch==2.14.1") && !mac.contains("--index-url"));
}

#[test]
fn download_errors_are_mapped_to_install_codes() {
    let e = map_download_error(AppError::models("MODELS_HASH_MISMATCH", "x"));
    assert_eq!(e.code(), "ENGINE_INSTALL_HASH_MISMATCH");
    let e = map_download_error(AppError::models("MODELS_DISK_FULL", "x"));
    assert_eq!(e.code(), "ENGINE_INSTALL_DISK_FULL");
    let e = map_download_error(AppError::models("MODELS_DOWNLOAD_FAILED", "x"));
    assert_eq!(e.code(), "ENGINE_INSTALL_DOWNLOAD_FAILED");
}

#[test]
fn strip_ansi_removes_colour_codes() {
    assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
}

/// 実際にダウンロードして導入する(数 GB・数十分)。通常の `cargo test` では実行しない。
/// `GAZOOO_INSTALL_TEST_DIR` に導入先(空のフォルダ)を指定する。
/// 例: `GAZOOO_INSTALL_TEST_DIR=C:/gz-inst cargo test real_engine_install -- --ignored --nocapture`
#[tokio::test]
#[ignore = "実際にダウンロードして導入する(数 GB)"]
async fn real_engine_install() {
    let root = PathBuf::from(
        std::env::var("GAZOOO_INSTALL_TEST_DIR")
            .expect("GAZOOO_INSTALL_TEST_DIR を指定してください"),
    );
    let (i, _) = installer(&root, Platform::current());
    let req = StartRequest {
        components: vec!["gguf".into()],
        accepted: true,
    };
    i.start(req, noop_finalize()).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(60 * 60);
    let mut last = String::new();
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let s = i.state();
        let line = format!(
            "{:?} step {}/{} {:?} {:?}/{:?}",
            s.status, s.step, s.total_steps, s.step_id, s.received, s.total
        );
        if line != last {
            println!("{line}");
            last = line;
        }
        match s.status {
            InstallStatus::Done => break,
            InstallStatus::Failed | InstallStatus::Cancelled => {
                panic!("導入に失敗: {:?}\n{}", s.error, i.logs(60).join("\n"))
            }
            _ => {}
        }
        assert!(Instant::now() < deadline, "時間切れ");
    }
    let l = Layout { root };
    let rec = l.load_record().expect("記録がある");
    assert!(rec.complete);
    assert!(l.main_py().is_file());
    assert!(l.venv_python(Platform::current().unwrap()).is_file());
    assert!(l.nodes_dir().join("ComfyUI-GGUF").is_dir());
    println!("{}", i.logs(15).join("\n"));
}

#[test]
fn detects_installed_onnxruntime_variants() {
    let root = std::env::temp_dir().join(format!("gazooo-ort-{}", uuid::Uuid::now_v7()));
    // 何も無い
    assert!(!onnxruntime_installed(&root));
    // Windows の配置(GPU 版)
    let win = root.join("Lib").join("site-packages");
    std::fs::create_dir_all(win.join("onnxruntime_gpu-1.32.0.dist-info")).unwrap();
    assert!(onnxruntime_installed(&root));
    std::fs::remove_dir_all(&root).unwrap();
    // macOS の配置(CPU 版)。無関係なパッケージだけでは false
    let mac = root.join("lib").join("python3.12").join("site-packages");
    std::fs::create_dir_all(mac.join("numpy-2.0.0.dist-info")).unwrap();
    assert!(!onnxruntime_installed(&root));
    std::fs::create_dir_all(mac.join("onnxruntime-1.20.0.dist-info")).unwrap();
    assert!(onnxruntime_installed(&root));
    let _ = std::fs::remove_dir_all(&root);
}
