//! 1 ファイルのダウンロード(docs/12-model-downloader.md §6)。
//!
//! - 許可リストのホスト・https のみ(リダイレクト先も検査)
//! - `<file>.part` に書き、SHA-256 が固定値と一致したときだけ `rename` で確定する
//! - `.part` が残っていれば `Range` で再開する
//! - キャンセルは `watch` で受け取る

use std::error::Error as _;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

use crate::error::{AppError, AppResult};

const MAX_REDIRECTS: usize = 5;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// 受信の無通信がこの時間続いたら中断する。
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
const HASH_CHUNK: usize = 1024 * 1024;

/// 通信してよい取得元の条件。
#[derive(Debug, Clone)]
pub struct DownloadPolicy {
    allowed_hosts: Vec<String>,
    https_only: bool,
}

impl DownloadPolicy {
    /// 本番: Hugging Face(とその CDN のサブドメイン)への https のみ。
    pub fn production() -> Self {
        Self {
            allowed_hosts: vec!["huggingface.co".into(), "hf.co".into()],
            https_only: true,
        }
    }

    /// 指定ホスト(とそのサブドメイン)への https のみ(ComfyUI の導入。docs/23 §5)。
    pub fn with_hosts(hosts: Vec<String>) -> Self {
        Self {
            allowed_hosts: hosts,
            https_only: true,
        }
    }

    /// テスト用: 指定ホストへの http を許可する。
    #[cfg(test)]
    pub fn for_test(host: &str) -> Self {
        Self {
            allowed_hosts: vec![host.into()],
            https_only: false,
        }
    }

    /// ホスト名が許可リストと一致する(またはそのサブドメイン)か。
    pub fn host_allowed(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.allowed_hosts
            .iter()
            .any(|a| host == *a || host.ends_with(&format!(".{a}")))
    }

    pub fn url_allowed(&self, url: &reqwest::Url) -> bool {
        let scheme_ok = match url.scheme() {
            "https" => true,
            "http" => !self.https_only,
            _ => false,
        };
        scheme_ok && url.host_str().is_some_and(|h| self.host_allowed(h))
    }
}

/// リダイレクト先が許可リスト外だったことを示す(エラーの連鎖から見分けるための型)。
#[derive(Debug)]
struct HostNotAllowed(String);

impl std::fmt::Display for HostNotAllowed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "許可されていない取得先です: {}", self.0)
    }
}

impl std::error::Error for HostNotAllowed {}

/// ダウンロード用の HTTP クライアント。TLS 検証は無効化しない。
pub fn build_client(policy: &DownloadPolicy) -> AppResult<reqwest::Client> {
    // ComfyUI クライアントと同じく、reqwest を作る前に暗号プロバイダ(ring)を登録する。
    let _ = rustls::crypto::ring::default_provider().install_default();
    let policy = policy.clone();
    let redirect = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("リダイレクトが多すぎます");
        }
        if policy.url_allowed(attempt.url()) {
            attempt.follow()
        } else {
            let host = attempt.url().host_str().unwrap_or("?").to_string();
            attempt.error(HostNotAllowed(host))
        }
    });
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .redirect(redirect)
        // 識別情報を含めない(アプリ名とバージョンのみ)
        .user_agent(concat!("gazooo/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AppError::internal(e.to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelMode {
    Run,
    /// 中止する(`.part` は再開用に残す)。
    Cancel,
    /// 中止して `.part` も削除する。
    Discard,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Cancelled(CancelMode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Downloading,
    Verifying,
}

#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub phase: Phase,
    pub received: u64,
    pub bytes_per_sec: Option<u64>,
}

pub struct Request<'a> {
    pub url: &'a str,
    pub part_path: &'a Path,
    pub final_path: &'a Path,
    pub size_bytes: u64,
    /// 小文字 16 進の SHA-256。
    pub sha256: &'a str,
}

fn failed(message: impl Into<String>) -> AppError {
    AppError::models("MODELS_DOWNLOAD_FAILED", message)
}

fn io_error(e: std::io::Error) -> AppError {
    if e.kind() == std::io::ErrorKind::StorageFull {
        AppError::models("MODELS_DISK_FULL", "ディスクの空き容量が足りません")
    } else {
        AppError::Storage(e.to_string())
    }
}

fn host_not_allowed(host: &str) -> AppError {
    AppError::models_with(
        "MODELS_HOST_NOT_ALLOWED",
        HostNotAllowed(host.into()).to_string(),
        json!({ "host": host }),
    )
}

/// reqwest のエラーを分類する。**URL には署名つきの値が含まれうるため、メッセージに含めない**。
fn map_reqwest(e: reqwest::Error) -> AppError {
    let mut source = e.source();
    while let Some(s) = source {
        if let Some(h) = s.downcast_ref::<HostNotAllowed>() {
            return host_not_allowed(&h.0);
        }
        source = s.source();
    }
    let e = e.without_url();
    if e.is_timeout() {
        failed("通信がタイムアウトしました。ネットワークを確認して再試行してください")
    } else if e.is_connect() {
        failed("ダウンロード元に接続できません。ネットワークを確認してください")
    } else if e.is_redirect() {
        failed("リダイレクトの処理に失敗しました")
    } else {
        failed(format!("ダウンロードに失敗しました: {e}"))
    }
}

fn mismatch(message: &str) -> AppError {
    AppError::models("MODELS_HASH_MISMATCH", message)
}

/// ファイル全体の SHA-256(小文字 16 進)。数 GB でも詰まらないよう、ブロッキングスレッドで読む。
pub async fn sha256_file(path: &Path) -> AppResult<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> AppResult<String> {
        Ok(hex(hash_prefix_blocking(&path)?.finalize().as_slice()))
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

fn hash_prefix_blocking(path: &Path) -> AppResult<Sha256> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; HASH_CHUNK];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(hasher);
        }
        hasher.update(&buf[..n]);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// キャンセルが指示されるまで待つ(すでに指示されていれば即座に返る)。
async fn cancelled(rx: &mut watch::Receiver<CancelMode>) -> CancelMode {
    loop {
        let mode = *rx.borrow_and_update();
        if mode != CancelMode::Run {
            return mode;
        }
        if rx.changed().await.is_err() {
            // 送信側が無くなった = 誰もキャンセルできない。止まらず待ち続ける
            std::future::pending::<()>().await;
        }
    }
}

/// `Content-Range: bytes START-END/TOTAL` の START。
fn content_range_start(value: &str) -> Option<u64> {
    value
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .trim()
        .parse()
        .ok()
}

/// ダウンロードを実行する。完了時は検証済みのファイルが `final_path` にある。
pub async fn download(
    client: &reqwest::Client,
    policy: &DownloadPolicy,
    req: &Request<'_>,
    cancel: &mut watch::Receiver<CancelMode>,
    on_progress: &mut (dyn FnMut(Progress) + Send),
) -> AppResult<Outcome> {
    let url = reqwest::Url::parse(req.url).map_err(|_| failed("取得元 URL が不正です"))?;
    if !policy.url_allowed(&url) {
        return Err(host_not_allowed(url.host_str().unwrap_or("?")));
    }
    let total = req.size_bytes;

    // 前回の続き
    let mut existing = tokio::fs::metadata(req.part_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    if existing > total {
        let _ = tokio::fs::remove_file(req.part_path).await;
        existing = 0;
    }
    let mut hasher = Sha256::new();
    if existing > 0 {
        let part = req.part_path.to_path_buf();
        hasher = tokio::task::spawn_blocking(move || hash_prefix_blocking(&part))
            .await
            .map_err(|e| AppError::internal(e.to_string()))??;
    }
    let mut received = existing;
    let started = Instant::now();
    let started_at = existing;
    let mut last_emit = Instant::now() - PROGRESS_INTERVAL;
    let mut emit = |phase: Phase, received: u64, force: bool, last_emit: &mut Instant| {
        if force || last_emit.elapsed() >= PROGRESS_INTERVAL {
            let secs = started.elapsed().as_secs_f64();
            let bps = (secs > 0.5).then(|| ((received - started_at) as f64 / secs) as u64);
            on_progress(Progress {
                phase,
                received,

                bytes_per_sec: bps,
            });
            *last_emit = Instant::now();
        }
    };
    emit(Phase::Downloading, received, true, &mut last_emit);

    if received < total {
        let mut rb = client.get(url);
        if existing > 0 {
            rb = rb.header(RANGE, format!("bytes={existing}-"));
        }
        let resp = tokio::select! {
            biased;
            mode = cancelled(cancel) => return Ok(Outcome::Cancelled(mode)),
            r = rb.send() => r.map_err(map_reqwest)?,
        };
        let status = resp.status();
        let mut append = existing > 0;
        match status.as_u16() {
            206 => {
                let start = resp
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(content_range_start);
                if start != Some(existing) {
                    return Err(failed("再開位置が一致しません。もう一度お試しください"));
                }
            }
            200 => {
                // Range を無視された(または最初から): 全体を受け直す
                if resp
                    .headers()
                    .get(CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .is_some_and(|len| len != total)
                {
                    return Err(mismatch("ファイルのサイズが想定と一致しません"));
                }
                hasher = Sha256::new();
                received = 0;
                append = false;
            }
            416 => {
                // 続きを要求できない: 破棄して、次回は最初から取得する
                let _ = tokio::fs::remove_file(req.part_path).await;
                return Err(failed(
                    "続きからの取得に失敗しました。もう一度お試しください",
                ));
            }
            code => {
                return Err(AppError::models_with(
                    "MODELS_DOWNLOAD_FAILED",
                    format!("ダウンロード元がエラーを返しました(HTTP {code})"),
                    json!({ "status": code }),
                ));
            }
        }

        let mut file = if append {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(req.part_path)
                .await
        } else {
            tokio::fs::File::create(req.part_path).await
        }
        .map_err(io_error)?;

        let mut stream = resp.bytes_stream();
        loop {
            tokio::select! {
                biased;
                mode = cancelled(cancel) => {
                    let _ = file.flush().await;
                    return Ok(Outcome::Cancelled(mode));
                }
                chunk = stream.next() => match chunk {
                    None => break,
                    Some(Err(e)) => {
                        let _ = file.flush().await;
                        return Err(map_reqwest(e));
                    }
                    Some(Ok(bytes)) => {
                        received += bytes.len() as u64;
                        if received > total {
                            drop(file);
                            let _ = tokio::fs::remove_file(req.part_path).await;
                            return Err(mismatch("受信したデータが想定より大きいため中止しました"));
                        }
                        hasher.update(&bytes);
                        file.write_all(&bytes).await.map_err(io_error)?;
                        emit(Phase::Downloading, received, false, &mut last_emit);
                    }
                }
            }
        }
        file.flush().await.map_err(io_error)?;
        file.sync_all().await.map_err(io_error)?;
    }

    if received != total {
        // 途中で切れた。`.part` は残して再開できるようにする
        return Err(failed(
            "接続が途中で切れました。もう一度お試しください(続きから再開します)",
        ));
    }

    emit(Phase::Verifying, received, true, &mut last_emit);
    if hex(hasher.finalize().as_slice()) != req.sha256 {
        let _ = tokio::fs::remove_file(req.part_path).await;
        return Err(mismatch(
            "ダウンロードしたファイルのハッシュが一致しません(破損または改ざんの可能性があるため破棄しました)",
        ));
    }

    // 既存ファイルは上書きしない(取得中に別の手段で置かれた場合など)
    if tokio::fs::try_exists(req.final_path)
        .await
        .map_err(io_error)?
    {
        let _ = tokio::fs::remove_file(req.part_path).await;
        return Err(AppError::models(
            "MODELS_ALREADY_INSTALLED",
            "同名のファイルがすでにあるため、上書きしませんでした",
        ));
    }
    tokio::fs::rename(req.part_path, req.final_path)
        .await
        .map_err(io_error)?;
    Ok(Outcome::Done)
}

#[cfg(test)]
mod tests;
