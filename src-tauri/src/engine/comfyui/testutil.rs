//! テスト用の ComfyUI 互換モックサーバー(HTTP + WebSocket)。
//!
//! ルートは `"METHOD /path"` をキーとし、同じキーの応答は呼ばれるたびに順に返す(最後の応答は繰り返す)。
//! 未定義のルートは 404 `{}`。`/ws?` への WebSocket 接続には、指定したテキストメッセージを送る。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::SinkExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

pub struct Route {
    key: String,
    responses: Vec<(u16, Vec<u8>)>,
}

impl Route {
    pub fn get(path: &str, body: &str) -> Self {
        Self::seq(&format!("GET {path}"), vec![(200, body)])
    }

    pub fn post(path: &str, status: u16, body: &str) -> Self {
        Self::seq(&format!("POST {path}"), vec![(status, body)])
    }

    pub fn seq(key: &str, responses: Vec<(u16, &str)>) -> Self {
        Self {
            key: key.to_string(),
            responses: responses
                .into_iter()
                .map(|(s, b)| (s, b.as_bytes().to_vec()))
                .collect(),
        }
    }

    pub fn bytes(key: &str, body: Vec<u8>) -> Self {
        Self {
            key: key.to_string(),
            responses: vec![(200, body)],
        }
    }
}

pub struct Mock {
    pub url: String,
    /// 受信したリクエスト(`"METHOD /path?query"`、POST は末尾に ` <body>`)。
    pub requests: Arc<Mutex<Vec<String>>>,
}

pub async fn spawn(routes: Vec<Route>) -> Mock {
    spawn_with_ws(routes, vec![]).await
}

pub async fn spawn_with_ws(routes: Vec<Route>, ws_messages: Vec<String>) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let routes = Arc::new(routes);
    let counters: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
    let requests: Arc<Mutex<Vec<String>>> = Arc::default();
    let ws_messages = Arc::new(ws_messages);

    let reqs = Arc::clone(&requests);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let (routes, counters, reqs, ws) = (
                Arc::clone(&routes),
                Arc::clone(&counters),
                Arc::clone(&reqs),
                Arc::clone(&ws_messages),
            );
            tokio::spawn(async move {
                let mut peek = [0u8; 256];
                let n = stream.peek(&mut peek).await.unwrap_or(0);
                if String::from_utf8_lossy(&peek[..n]).contains("/ws?") {
                    serve_ws(stream, &ws).await;
                } else {
                    serve_http(stream, &routes, &counters, &reqs).await;
                }
            });
        }
    });

    Mock {
        url: format!("http://{addr}"),
        requests,
    }
}

async fn serve_ws(stream: TcpStream, messages: &[String]) {
    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    for m in messages {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if ws.send(Message::Text(m.clone().into())).await.is_err() {
            return;
        }
    }
    // クライアントが切るまで保持する
    tokio::time::sleep(Duration::from_secs(30)).await;
}

async fn serve_http(
    mut sock: TcpStream,
    routes: &[Route],
    counters: &Mutex<HashMap<String, usize>>,
    reqs: &Mutex<Vec<String>>,
) {
    // ヘッダ + Content-Length 分の本文を読む
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    let header_end = loop {
        let n = sock.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&data[..header_end]).to_string();
    let content_len = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    while data.len() < header_end + content_len {
        let n = sock.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
    }
    let body = String::from_utf8_lossy(&data[header_end..]).to_string();

    let mut parts = head.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    let key = format!("{method} {target}");
    reqs.lock().unwrap().push(if body.is_empty() {
        key.clone()
    } else {
        format!("{key} {body}")
    });

    let route = routes
        .iter()
        .find(|r| r.key == key || key.starts_with(&format!("{}?", r.key)));
    let (status, payload) = match route {
        Some(r) => {
            let mut c = counters.lock().unwrap();
            let i = c.entry(r.key.clone()).or_insert(0);
            let resp = r.responses[(*i).min(r.responses.len() - 1)].clone();
            *i += 1;
            resp
        }
        None => (404, b"{}".to_vec()),
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let _ = sock.write_all(head.as_bytes()).await;
    let _ = sock.write_all(&payload).await;
    let _ = sock.shutdown().await;
}
