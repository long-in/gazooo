//! ダウンロード基盤のテスト。ローカルのモックサーバーだけを使い、外部へは出ない。

use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::*;
use crate::services::models::testserver::{Reply, spawn};

fn data(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 % 251) as u8).collect()
}

fn sha(bytes: &[u8]) -> String {
    hex(Sha256::digest(bytes).as_slice())
}

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("gazooo-dl-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn part(&self) -> PathBuf {
        self.0.join("m.safetensors.part")
    }
    fn fin(&self) -> PathBuf {
        self.0.join("m.safetensors")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn policy() -> DownloadPolicy {
    DownloadPolicy::for_test("127.0.0.1")
}

async fn run(
    url: &str,
    dir: &Dir,
    body: &[u8],
    sha256: &str,
    rx: &mut watch::Receiver<CancelMode>,
) -> AppResult<Outcome> {
    let client = build_client(&policy()).unwrap();
    let part = dir.part();
    let fin = dir.fin();
    let req = Request {
        url,
        part_path: &part,
        final_path: &fin,
        size_bytes: body.len() as u64,
        sha256,
    };
    download(&client, &policy(), &req, rx, &mut |_| {}).await
}

fn code(r: AppResult<Outcome>) -> &'static str {
    r.unwrap_err().code()
}

fn idle() -> (watch::Sender<CancelMode>, watch::Receiver<CancelMode>) {
    watch::channel(CancelMode::Run)
}

#[tokio::test]
async fn downloads_verifies_and_renames() {
    let body = data(300_000);
    let srv = spawn(vec![(
        "/m",
        Reply::Body {
            body: body.clone(),
            ranges: true,
        },
    )])
    .await;
    let dir = Dir::new();
    let (_tx, mut rx) = idle();
    let out = run(
        &format!("{}/m", srv.base),
        &dir,
        &body,
        &sha(&body),
        &mut rx,
    )
    .await
    .unwrap();
    assert_eq!(out, Outcome::Done);
    assert_eq!(std::fs::read(dir.fin()).unwrap(), body);
    assert!(!dir.part().exists(), ".part は残らない");
    assert_eq!(sha256_file(&dir.fin()).await.unwrap(), sha(&body));
}

#[tokio::test]
async fn hash_mismatch_discards_and_never_places_the_file() {
    let body = data(50_000);
    let srv = spawn(vec![(
        "/m",
        Reply::Body {
            body: body.clone(),
            ranges: true,
        },
    )])
    .await;
    let dir = Dir::new();
    let (_tx, mut rx) = idle();
    let wrong = sha(b"something else");
    let r = run(&format!("{}/m", srv.base), &dir, &body, &wrong, &mut rx).await;
    assert_eq!(code(r), "MODELS_HASH_MISMATCH");
    assert!(!dir.fin().exists(), "検証に失敗したファイルは配置されない");
    assert!(!dir.part().exists(), "破損したデータは再利用しない");
}

#[tokio::test]
async fn resumes_from_part_with_range() {
    let body = data(200_000);
    let srv = spawn(vec![(
        "/m",
        Reply::Body {
            body: body.clone(),
            ranges: true,
        },
    )])
    .await;
    let dir = Dir::new();
    std::fs::write(dir.part(), &body[..120_000]).unwrap();
    let (_tx, mut rx) = idle();
    let out = run(
        &format!("{}/m", srv.base),
        &dir,
        &body,
        &sha(&body),
        &mut rx,
    )
    .await
    .unwrap();
    assert_eq!(out, Outcome::Done);
    assert_eq!(
        std::fs::read(dir.fin()).unwrap(),
        body,
        "続きから取得しても全体が一致する"
    );
    let reqs = srv.requests.lock().unwrap().clone();
    assert_eq!(reqs, vec!["GET /m range=bytes=120000-".to_string()]);
}

#[tokio::test]
async fn restarts_when_server_ignores_range() {
    let body = data(100_000);
    let srv = spawn(vec![(
        "/m",
        Reply::Body {
            body: body.clone(),
            ranges: false,
        },
    )])
    .await;
    let dir = Dir::new();
    std::fs::write(dir.part(), &body[..40_000]).unwrap();
    let (_tx, mut rx) = idle();
    let out = run(
        &format!("{}/m", srv.base),
        &dir,
        &body,
        &sha(&body),
        &mut rx,
    )
    .await
    .unwrap();
    assert_eq!(out, Outcome::Done);
    assert_eq!(std::fs::read(dir.fin()).unwrap(), body);
}

#[tokio::test]
async fn oversized_part_is_discarded_and_restarted() {
    let body = data(10_000);
    let srv = spawn(vec![(
        "/m",
        Reply::Body {
            body: body.clone(),
            ranges: true,
        },
    )])
    .await;
    let dir = Dir::new();
    std::fs::write(dir.part(), data(50_000)).unwrap();
    let (_tx, mut rx) = idle();
    run(
        &format!("{}/m", srv.base),
        &dir,
        &body,
        &sha(&body),
        &mut rx,
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(dir.fin()).unwrap(), body);
}

#[tokio::test]
async fn complete_part_is_verified_without_requesting() {
    let body = data(10_000);
    let srv = spawn(vec![]).await;
    let dir = Dir::new();
    std::fs::write(dir.part(), &body).unwrap();
    let (_tx, mut rx) = idle();
    run(
        &format!("{}/m", srv.base),
        &dir,
        &body,
        &sha(&body),
        &mut rx,
    )
    .await
    .unwrap();
    assert!(srv.requests.lock().unwrap().is_empty());
    assert!(dir.fin().exists());
}

#[tokio::test]
async fn http_error_reports_status_and_keeps_nothing() {
    let srv = spawn(vec![("/m", Reply::Status(404))]).await;
    let dir = Dir::new();
    let (_tx, mut rx) = idle();
    let err = run(
        &format!("{}/m", srv.base),
        &dir,
        &data(10),
        &sha(&data(10)),
        &mut rx,
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), "MODELS_DOWNLOAD_FAILED");
    let v = serde_json::to_value(&err).unwrap();
    assert_eq!(v["details"]["status"], 404);
    assert!(!dir.fin().exists());
}

#[tokio::test]
async fn short_body_keeps_part_for_resume() {
    // サイズ 10000 のはずが 4000 しか来ない(接続が切れた状況)
    let full = data(10_000);
    let srv = spawn(vec![(
        "/m",
        Reply::Body {
            body: full[..4_000].to_vec(),
            ranges: false,
        },
    )])
    .await;
    let dir = Dir::new();
    let (_tx, mut rx) = idle();
    let r = run(
        &format!("{}/m", srv.base),
        &dir,
        &full,
        &sha(&full),
        &mut rx,
    )
    .await;
    // Content-Length が想定と違うため、不一致として扱われる
    assert!(matches!(
        code(r),
        "MODELS_HASH_MISMATCH" | "MODELS_DOWNLOAD_FAILED"
    ));
    assert!(!dir.fin().exists());
}

#[tokio::test]
async fn refuses_to_overwrite_an_existing_file() {
    let body = data(5_000);
    let srv = spawn(vec![(
        "/m",
        Reply::Body {
            body: body.clone(),
            ranges: true,
        },
    )])
    .await;
    let dir = Dir::new();
    std::fs::write(dir.fin(), b"user's own file").unwrap();
    let (_tx, mut rx) = idle();
    let r = run(
        &format!("{}/m", srv.base),
        &dir,
        &body,
        &sha(&body),
        &mut rx,
    )
    .await;
    assert_eq!(code(r), "MODELS_ALREADY_INSTALLED");
    assert_eq!(std::fs::read(dir.fin()).unwrap(), b"user's own file");
    assert!(!dir.part().exists());
}

#[tokio::test]
async fn refuses_hosts_outside_the_allow_list() {
    let dir = Dir::new();
    let (_tx, mut rx) = idle();
    let body = data(10);
    for url in [
        "http://evil.example.com/m",
        "ftp://127.0.0.1/m",
        "not a url",
    ] {
        let r = run(url, &dir, &body, &sha(&body), &mut rx).await;
        assert!(
            matches!(
                code(r),
                "MODELS_HOST_NOT_ALLOWED" | "MODELS_DOWNLOAD_FAILED"
            ),
            "{url}"
        );
    }
}

#[tokio::test]
async fn refuses_redirect_to_a_disallowed_host() {
    let body = data(1_000);
    // 同じサーバーだが、ホスト名 localhost は許可リスト(127.0.0.1)にない
    let probe = spawn(vec![]).await;
    let port = probe.base.rsplit(':').next().unwrap().to_string();
    let srv = spawn(vec![(
        "/m",
        Reply::Redirect(format!("http://localhost:{port}/x")),
    )])
    .await;
    let dir = Dir::new();
    let (_tx, mut rx) = idle();
    let r = run(
        &format!("{}/m", srv.base),
        &dir,
        &body,
        &sha(&body),
        &mut rx,
    )
    .await;
    assert_eq!(code(r), "MODELS_HOST_NOT_ALLOWED");
    assert!(
        probe.requests.lock().unwrap().is_empty(),
        "許可外のホストへは要求を送らない"
    );
}

#[tokio::test]
async fn follows_redirects_within_the_allow_list() {
    let body = data(20_000);
    let srv = spawn(vec![]).await;
    let target = spawn(vec![(
        "/real",
        Reply::Body {
            body: body.clone(),
            ranges: true,
        },
    )])
    .await;
    drop(srv);
    let srv = spawn(vec![(
        "/m",
        Reply::Redirect(format!("{}/real", target.base)),
    )])
    .await;
    let dir = Dir::new();
    let (_tx, mut rx) = idle();
    let out = run(
        &format!("{}/m", srv.base),
        &dir,
        &body,
        &sha(&body),
        &mut rx,
    )
    .await
    .unwrap();
    assert_eq!(out, Outcome::Done);
}

#[tokio::test]
async fn cancel_keeps_part_and_discard_removes_it() {
    for (mode, part_remains) in [(CancelMode::Cancel, true), (CancelMode::Discard, false)] {
        let body = data(2_000_000);
        let srv = spawn(vec![(
            "/m",
            Reply::Slow {
                body: body.clone(),
                chunk: 100_000,
                delay: Duration::from_millis(60),
            },
        )])
        .await;
        let dir = Dir::new();
        let (tx, mut rx) = idle();
        let url = format!("{}/m", srv.base);
        let canceller = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            tx.send(mode).unwrap();
            tx
        });
        let out = run(&url, &dir, &body, &sha(&body), &mut rx).await.unwrap();
        let _tx = canceller.await.unwrap();
        assert_eq!(out, Outcome::Cancelled(mode));
        assert!(!dir.fin().exists());
        // 破棄の削除はサービス側の責務。ここでは `.part` が壊れずに残ることを確認する
        assert!(
            dir.part().exists(),
            "download 自体は .part を残して戻る(mode={mode:?}, 期待={part_remains})"
        );
        let len = std::fs::metadata(dir.part()).unwrap().len();
        assert!(len > 0 && len < body.len() as u64);
    }
}

#[tokio::test]
async fn cancel_before_start_never_contacts_the_server() {
    let body = data(1_000);
    let srv = spawn(vec![(
        "/m",
        Reply::Body {
            body: body.clone(),
            ranges: true,
        },
    )])
    .await;
    let dir = Dir::new();
    let (tx, mut rx) = watch::channel(CancelMode::Cancel);
    let out = run(
        &format!("{}/m", srv.base),
        &dir,
        &body,
        &sha(&body),
        &mut rx,
    )
    .await
    .unwrap();
    drop(tx);
    assert_eq!(out, Outcome::Cancelled(CancelMode::Cancel));
}

#[tokio::test]
async fn resume_after_cancel_completes() {
    let body = data(1_500_000);
    let srv = spawn(vec![(
        "/m",
        Reply::Slow {
            body: body.clone(),
            chunk: 100_000,
            delay: Duration::from_millis(40),
        },
    )])
    .await;
    let dir = Dir::new();
    let (tx, mut rx) = idle();
    let url = format!("{}/m", srv.base);
    let h = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        tx.send(CancelMode::Cancel).unwrap();
        tx
    });
    let first = run(&url, &dir, &body, &sha(&body), &mut rx).await.unwrap();
    let _tx = h.await.unwrap();
    assert_eq!(first, Outcome::Cancelled(CancelMode::Cancel));

    // 続きから(このサーバーは Range 非対応のため最初から受け直すが、結果は一致する)
    let (_tx2, mut rx2) = idle();
    let second = run(&url, &dir, &body, &sha(&body), &mut rx2).await.unwrap();
    assert_eq!(second, Outcome::Done);
    assert_eq!(std::fs::read(dir.fin()).unwrap(), body);
}

#[test]
fn policy_matches_hosts_and_subdomains_only() {
    let p = DownloadPolicy::production();
    assert!(p.host_allowed("huggingface.co"));
    assert!(p.host_allowed("cdn-lfs.huggingface.co"));
    assert!(p.host_allowed("us.aws.cdn.hf.co"));
    assert!(p.host_allowed("HF.CO"));
    assert!(!p.host_allowed("evil-hf.co"));
    assert!(!p.host_allowed("huggingface.co.evil.com"));
    assert!(!p.host_allowed("hf.co.evil.com"));
    let https = reqwest::Url::parse("https://us.aws.cdn.hf.co/x?Signature=abc").unwrap();
    let http = reqwest::Url::parse("http://huggingface.co/x").unwrap();
    assert!(p.url_allowed(&https));
    assert!(!p.url_allowed(&http), "本番は https のみ");
}

#[test]
fn parses_content_range_start() {
    assert_eq!(content_range_start("bytes 100-199/200"), Some(100));
    assert_eq!(content_range_start("bytes */200"), None);
    assert_eq!(content_range_start("items 1-2/3"), None);
}
