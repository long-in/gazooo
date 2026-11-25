//! モデル取得のテスト用 HTTP サーバー(ローカルのみ。外部へは出ない)。
//!
//! `Range` 要求・リダイレクト・ゆっくりした配信・エラー応答を再現できる。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone)]
pub enum Reply {
    /// 本文を返す。`ranges = true` なら `Range` 要求に 206 で応える。
    Body {
        body: Vec<u8>,
        ranges: bool,
    },
    /// 302 で別の URL へ。
    Redirect(String),
    Status(u16),
    /// `chunk` バイトずつ、`delay` 置きに返す(キャンセルの試験用)。
    Slow {
        body: Vec<u8>,
        chunk: usize,
        delay: Duration,
    },
}

pub struct Server {
    pub base: String,
    /// 受信したリクエスト(`"GET /path"`、`Range` があれば ` range=<値>` を付ける)。
    pub requests: Arc<Mutex<Vec<String>>>,
}

pub async fn spawn(routes: Vec<(&str, Reply)>) -> Server {
    let routes: Arc<HashMap<String, Reply>> = Arc::new(
        routes
            .into_iter()
            .map(|(p, r)| (p.to_string(), r))
            .collect(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests: Arc<Mutex<Vec<String>>> = Arc::default();
    let reqs = Arc::clone(&requests);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let (routes, reqs) = (Arc::clone(&routes), Arc::clone(&reqs));
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buf = [0u8; 2048];
                while !data.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => data.extend_from_slice(&buf[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&data).to_string();
                let mut lines = text.lines();
                let path = lines
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let range = lines.find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("range:")
                        .map(|v| v.trim().to_string())
                });
                let mut line = format!("GET {path}");
                if let Some(r) = &range {
                    line.push_str(&format!(" range={r}"));
                }
                reqs.lock().unwrap().push(line);

                match routes.get(&path).cloned() {
                    None => respond(&mut sock, 404, &[], b"").await,
                    Some(Reply::Status(code)) => respond(&mut sock, code, &[], b"").await,
                    Some(Reply::Redirect(to)) => {
                        respond(&mut sock, 302, &[("Location", to)], b"").await
                    }
                    Some(Reply::Body { body, ranges }) => {
                        let start = range
                            .as_deref()
                            .and_then(|r| r.strip_prefix("bytes="))
                            .and_then(|r| r.split('-').next())
                            .and_then(|s| s.parse::<usize>().ok());
                        match (ranges, start) {
                            (true, Some(s)) if s < body.len() => {
                                let total = body.len();
                                respond(
                                    &mut sock,
                                    206,
                                    &[(
                                        "Content-Range",
                                        format!("bytes {s}-{}/{total}", total - 1),
                                    )],
                                    &body[s..],
                                )
                                .await
                            }
                            (true, Some(_)) => respond(&mut sock, 416, &[], b"").await,
                            _ => respond(&mut sock, 200, &[], &body).await,
                        }
                    }
                    Some(Reply::Slow { body, chunk, delay }) => {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        if sock.write_all(head.as_bytes()).await.is_err() {
                            return;
                        }
                        for part in body.chunks(chunk) {
                            if sock.write_all(part).await.is_err() {
                                return;
                            }
                            let _ = sock.flush().await;
                            tokio::time::sleep(delay).await;
                        }
                    }
                }
            });
        }
    });
    Server {
        base: format!("http://{addr}"),
        requests,
    }
}

async fn respond(
    sock: &mut tokio::net::TcpStream,
    code: u16,
    headers: &[(&str, String)],
    body: &[u8],
) {
    let mut head = format!(
        "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = sock.write_all(head.as_bytes()).await;
    let _ = sock.write_all(body).await;
    let _ = sock.flush().await;
}
