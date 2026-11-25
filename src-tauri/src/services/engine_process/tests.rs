//! 起動管理のテスト。実際の ComfyUI は使わず、短命・長命の小さな外部コマンドで状態遷移を確認する。

use std::path::PathBuf;
use std::sync::Mutex as StdMutex;

use super::*;

#[derive(Default)]
struct TestSink {
    states: StdMutex<Vec<ProcStatus>>,
}

impl ProcSink for TestSink {
    fn state(&self, s: &EngineProcState) {
        self.states.lock().unwrap().push(s.status);
    }
    fn log(&self, _line: &str) {}
}

fn probe_always(ok: bool) -> Probe {
    Arc::new(move |_| Box::pin(async move { ok }))
}

fn service(probe: Probe, ready_timeout: Duration) -> (Arc<EngineProcess>, Arc<TestSink>) {
    let sink = Arc::new(TestSink::default());
    (
        EngineProcess::with_options(sink.clone(), None, probe, ready_timeout),
        sink,
    )
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

/// 出力して、終了コード 3 ですぐ終わるコマンド。
fn exits_with_3() -> LaunchSpec {
    let (program, args): (&str, &[&str]) = if cfg!(windows) {
        ("cmd", &["/c", "echo hello-from-child & exit 3"])
    } else {
        ("sh", &["-c", "echo hello-from-child; exit 3"])
    };
    LaunchSpec {
        program: PathBuf::from(program),
        args: args.iter().map(Into::into).collect(),
        cwd: None,
    }
}

/// テスト用のポートの割り当て元。OS の一時ポート(49152 番台など)より下の範囲を、テスト同士で重複しないように使う。
/// (`bind(":0")` で得て手放したポートは、並列に走る別のテストが同じポートを得て待ち受けることがあり、不安定になる)
static NEXT_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(20_000);

/// ポートを確保して待ち受けたまま返す(このテストプロセス内では、同じ値を二度返さない)。
fn listen_on_free_port() -> (std::net::TcpListener, u16) {
    loop {
        let port = NEXT_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port)) {
            return (listener, port);
        }
    }
}

/// 空いているポートを 1 つ得る(このテストプロセス内では、同じ値を二度返さない)。
fn free_port() -> u16 {
    listen_on_free_port().1
}

async fn wait_status(svc: &EngineProcess, want: ProcStatus) {
    for _ in 0..100 {
        if svc.state().status == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("status {:?} にならなかった: {:?}", want, svc.state());
}

#[tokio::test]
async fn starts_becomes_running_then_stops() {
    let (svc, sink) = service(probe_always(true), Duration::from_secs(10));
    let s = svc.start_spec(long_running(), free_port()).await.unwrap();
    assert!(s.managed_by_app && s.pid.is_some());
    wait_status(&svc, ProcStatus::Running).await;

    let s = svc.stop().await.unwrap();
    assert_eq!(s.status, ProcStatus::Stopped);
    assert!(!s.managed_by_app);
    // 二重停止は何もしない
    assert_eq!(svc.stop().await.unwrap().status, ProcStatus::Stopped);

    let seen = sink.states.lock().unwrap().clone();
    assert_eq!(
        seen.iter().copied().fold(Vec::new(), |mut v, s| {
            if v.last() != Some(&s) {
                v.push(s);
            }
            v
        }),
        [
            ProcStatus::Starting,
            ProcStatus::Running,
            ProcStatus::Stopping,
            ProcStatus::Stopped
        ]
    );
}

#[tokio::test]
async fn unexpected_exit_is_reported_with_code_and_log() {
    let (svc, _) = service(probe_always(false), Duration::from_secs(10));
    svc.start_spec(exits_with_3(), free_port()).await.unwrap();
    wait_status(&svc, ProcStatus::Crashed).await;
    let s = svc.state();
    assert_eq!(s.exit_code, Some(3));
    assert!(!s.managed_by_app);
    // 出力の取り込みは非同期のため、少し待つ
    for _ in 0..20 {
        if svc.logs(10).iter().any(|l| l.contains("hello-from-child")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(svc.logs(10).iter().any(|l| l.contains("hello-from-child")));
    // 異常終了の表示は、停止操作で消える
    assert_eq!(svc.stop().await.unwrap().status, ProcStatus::Stopped);
}

#[tokio::test]
async fn start_timeout_terminates_the_child() {
    let (svc, _) = service(probe_always(false), Duration::from_millis(800));
    svc.start_spec(long_running(), free_port()).await.unwrap();
    wait_status(&svc, ProcStatus::Crashed).await;
    let s = svc.state();
    assert!(s.message.unwrap().contains("タイムアウト"));
    assert!(!s.managed_by_app);
}

#[tokio::test]
async fn refuses_to_start_when_the_port_is_taken_without_touching_the_owner() {
    // 確保したソケットをそのまま待ち受けに使う(手放して取り直すと、環境によって「使用中」になることがある)
    let (listener, port) = listen_on_free_port();
    let (svc, _) = service(probe_always(true), Duration::from_secs(5));

    let err = svc.start_spec(long_running(), port).await.unwrap_err();
    assert_eq!(err.code(), "ENGINE_PORT_IN_USE");
    assert!(!svc.has_child());
    // 待ち受けていた側はそのまま
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
    drop(listener);
}

#[tokio::test]
async fn launch_failure_is_reported() {
    let (svc, _) = service(probe_always(true), Duration::from_secs(5));
    let spec = LaunchSpec {
        program: PathBuf::from("gazooo-definitely-missing-program"),
        args: vec![],
        cwd: None,
    };
    let err = svc.start_spec(spec, free_port()).await.unwrap_err();
    assert_eq!(err.code(), "ENGINE_LAUNCH_FAILED");
    assert_eq!(svc.state().status, ProcStatus::Crashed);
    // 失敗のあとでも、続けて起動できる
    svc.start_spec(long_running(), free_port()).await.unwrap();
    wait_status(&svc, ProcStatus::Running).await;
    svc.stop().await.unwrap();
}

#[tokio::test]
async fn second_start_returns_the_running_process() {
    let (svc, _) = service(probe_always(true), Duration::from_secs(5));
    let port = free_port();
    let a = svc.start_spec(long_running(), port).await.unwrap();
    let b = svc.start_spec(long_running(), port).await.unwrap();
    assert_eq!(a.pid, b.pid, "同じプロセスを返し、二重に起動しない");
    svc.stop().await.unwrap();
}

#[tokio::test]
async fn log_ring_buffer_keeps_only_the_latest_lines() {
    let (svc, _) = service(probe_always(true), Duration::from_secs(5));
    for i in 0..(LOG_LINES + 100) {
        svc.push_log(&format!("line {i}"));
    }
    let all = svc.logs(10_000);
    assert_eq!(all.len(), LOG_LINES);
    assert_eq!(all.last().unwrap(), &format!("line {}", LOG_LINES + 99));
    assert_eq!(svc.logs(3).len(), 3);
    // 極端に長い行は切り詰める
    svc.push_log(&"x".repeat(LOG_LINE_MAX * 3));
    assert_eq!(svc.logs(1)[0].chars().count(), LOG_LINE_MAX);
}

#[tokio::test]
async fn restart_replaces_the_process() {
    let (svc, _) = service(probe_always(true), Duration::from_secs(10));
    let port = free_port();
    let first = svc.start_spec(long_running(), port).await.unwrap();
    wait_status(&svc, ProcStatus::Running).await;

    let second = svc.restart_spec(long_running(), port).await.unwrap();
    assert_ne!(first.pid, second.pid, "別のプロセスに入れ替わる");
    wait_status(&svc, ProcStatus::Running).await;
    svc.stop().await.unwrap();
    assert_eq!(svc.state().status, ProcStatus::Stopped);
}

#[tokio::test]
async fn stop_while_starting_still_ends_the_child() {
    let (svc, _) = service(probe_always(false), Duration::from_secs(30));
    svc.start_spec(long_running(), free_port()).await.unwrap();
    let s = svc.stop().await.unwrap();
    assert_eq!(s.status, ProcStatus::Stopped);
    assert!(!svc.has_child());
}

/// 実機の ComfyUI(08 のセットアップ済みの `comfyui/`)での起動・再起動・停止の確認。
/// `cargo test -- --ignored real_comfyui_process --nocapture`
#[tokio::test]
#[ignore = "実際の ComfyUI を起動する(08 のセットアップが必要)"]
async fn real_comfyui_process_start_restart_stop() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("comfyui");
    let root = root
        .canonicalize()
        .expect("comfyui/ が無い(npm run comfyui:setup)");
    let python = if cfg!(windows) {
        root.join("venv/Scripts/python.exe")
    } else {
        root.join("venv/bin/python")
    };
    let cfg = ManagedEngineConfig {
        enabled: true,
        main_py: root.join("ComfyUI/main.py").to_string_lossy().into_owned(),
        python: python.to_string_lossy().into_owned(),
        base_directory: Some(root.join("data").to_string_lossy().into_owned()),
        port: 8199,
        auto_start: false,
        auto_recover: false,
    };
    let sink = Arc::new(TestSink::default());
    let svc = EngineProcess::new(sink, None);

    let s = svc.start(&cfg).await.unwrap();
    println!("started pid={:?}", s.pid);
    for _ in 0..240 {
        if svc.state().status == ProcStatus::Running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(
        svc.state().status,
        ProcStatus::Running,
        "logs: {:?}",
        svc.logs(20)
    );
    let first = svc.state().pid;

    svc.restart(&cfg).await.unwrap();
    for _ in 0..240 {
        if svc.state().status == ProcStatus::Running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(
        svc.state().status,
        ProcStatus::Running,
        "logs: {:?}",
        svc.logs(20)
    );
    assert_ne!(first, svc.state().pid);

    svc.stop().await.unwrap();
    assert_eq!(svc.state().status, ProcStatus::Stopped);
    assert!(
        wait_port_free(8199, Duration::from_secs(10)).await,
        "停止後もポートが使われている"
    );
    println!("log tail: {:?}", svc.logs(5));
}

#[test]
fn ansi_colors_are_stripped_from_logs() {
    assert_eq!(strip_ansi("\u{1b}[32m[INFO]\u{1b}[0m ok"), "[INFO] ok");
    assert_eq!(strip_ansi("plain 日本語"), "plain 日本語");
}
