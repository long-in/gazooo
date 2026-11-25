//! Anthropic Messages API クライアント(docs/11-ai-assistant.md)。Rust 側だけが外部へ通信する。
//!
//! ログやエラーメッセージには、キー・プロンプト・回答本文を含めない(ステータスとリクエスト ID のみ)。

use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::error::{AppError, AppResult};

pub const DEFAULT_BASE: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";
pub const MAX_TOKENS: u32 = 2048;

/// 受信したストリームのうち、呼び出し側が扱うもの。
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Text(String),
    ToolUse {
        name: String,
        input: Value,
    },
    /// 停止理由(`end_turn` / `max_tokens` / `refusal` など)。診断用。
    Stop(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum StreamEnd {
    Completed,
    Cancelled,
}

pub struct MessageRequest<'a> {
    pub model: &'a str,
    pub system: &'a str,
    /// (role, text)。先頭は user、以降は交互とは限らない(連続する user は API が結合する)。
    pub messages: &'a [(String, String)],
    pub tools: Value,
}

pub struct AnthropicClient {
    http: reqwest::Client,
    base: String,
}

impl AnthropicClient {
    pub fn new() -> AppResult<Self> {
        Self::with_base(DEFAULT_BASE)
    }

    pub fn with_base(base: &str) -> AppResult<Self> {
        // ComfyUI クライアントと同じく、reqwest を作る前に暗号プロバイダ(ring)を登録する。
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // ストリーミングのため全体のタイムアウトは設けず、無通信だけを検出する
            .read_timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AppError::internal(e.to_string()))?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
        })
    }

    /// キーを検証する(`GET /v1/models`。トークンは消費しない)。
    /// 無効なら `ASSISTANT_AUTH`。ネットワーク不可や一時的なエラーは `Ok(false)`(未検証)。
    pub async fn verify_key(&self, key: &str) -> AppResult<bool> {
        let resp = self
            .http
            .get(format!("{}/v1/models?limit=1", self.base))
            .header("x-api-key", key)
            .header("anthropic-version", API_VERSION)
            .timeout(Duration::from_secs(10))
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => Ok(true),
            Ok(r) if matches!(r.status().as_u16(), 401 | 403) => Err(auth_error()),
            _ => Ok(false),
        }
    }

    /// メッセージを送り、ストリームを `on_event` へ流す。`cancel` が true になったら中断する。
    pub async fn stream(
        &self,
        key: &str,
        req: &MessageRequest<'_>,
        cancel: &mut watch::Receiver<bool>,
        mut on_event: impl FnMut(StreamEvent),
    ) -> AppResult<StreamEnd> {
        let messages: Vec<Value> = req
            .messages
            .iter()
            .map(|(role, text)| json!({ "role": role, "content": text }))
            .collect();
        let mut body = json!({
            "model": req.model,
            "max_tokens": MAX_TOKENS,
            // 固定文なのでキャッシュする(相談ごとの入力コストを抑える。docs/13 §4.3)
            "system": [{
                "type": "text",
                "text": req.system,
                "cache_control": { "type": "ephemeral" }
            }],
            "messages": messages,
            "stream": true,
        });
        // ツール無しの呼び出し(翻訳。docs/34)では `tools` を送らない
        if req.tools.as_array().is_none_or(|t| !t.is_empty()) {
            body["tools"] = req.tools.clone();
        }

        let resp = tokio::select! {
            _ = cancelled(cancel) => return Ok(StreamEnd::Cancelled),
            r = self
                .http
                .post(format!("{}/v1/messages", self.base))
                .header("x-api-key", key)
                .header("anthropic-version", API_VERSION)
                .json(&body)
                .send() => r.map_err(network_error)?,
        };
        let status = resp.status();
        if !status.is_success() {
            let request_id = resp
                .headers()
                .get("request-id")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            return Err(map_status(status.as_u16(), request_id.as_deref()));
        }

        let mut parser = SseParser::default();
        let mut state = StreamState::default();
        let mut stream = resp.bytes_stream();
        loop {
            let chunk = tokio::select! {
                _ = cancelled(cancel) => return Ok(StreamEnd::Cancelled),
                c = stream.next() => c,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(network_error)?;
            for (event, data) in parser.feed(&chunk) {
                for ev in state.handle(&event, &data)? {
                    on_event(ev);
                }
                if state.finished {
                    return Ok(StreamEnd::Completed);
                }
            }
        }
        // message_stop を受けずに閉じた場合も、ここまでの内容は有効として扱う
        Ok(StreamEnd::Completed)
    }
}

/// `cancel` が true になるまで待つ(送信側が破棄されたら永遠に待つ)。
pub(super) async fn cancelled(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow() {
            return;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

fn auth_error() -> AppError {
    AppError::assistant(
        "ASSISTANT_AUTH",
        "API キーが無効です。キーを確認して登録し直してください",
    )
}

fn network_error(e: reqwest::Error) -> AppError {
    if e.is_connect() || e.is_timeout() || e.is_request() || e.is_body() {
        AppError::assistant(
            "ASSISTANT_UNREACHABLE",
            "Anthropic の API に接続できません。ネットワークを確認してください",
        )
    } else {
        AppError::assistant("ASSISTANT_API_ERROR", "API の呼び出しに失敗しました")
    }
}

pub fn map_status(status: u16, request_id: Option<&str>) -> AppError {
    match status {
        401 | 403 => auth_error(),
        429 | 529 => AppError::assistant(
            "ASSISTANT_RATE_LIMIT",
            "利用制限または混雑のため応答できません。しばらく待ってからもう一度お試しください",
        ),
        _ => AppError::assistant(
            "ASSISTANT_API_ERROR",
            format!(
                "API エラーが発生しました(HTTP {status}{})",
                request_id
                    .map(|id| format!("、リクエスト ID: {id}"))
                    .unwrap_or_default()
            ),
        ),
    }
}

/// Server-Sent Events のパーサ。チャンクの境界は任意で、UTF-8 の途中で切れてもよい。
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    /// 完結したイベントを `(event, data)` で返す。
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<(String, String)> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some((end, sep)) = find_event_end(&self.buf) {
            let raw: Vec<u8> = self.buf.drain(..end + sep).collect();
            let text = String::from_utf8_lossy(&raw[..end]).replace('\r', "");
            let (mut event, mut data) = (String::new(), Vec::<&str>::new());
            for line in text.lines() {
                if let Some(v) = line.strip_prefix("event:") {
                    event = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("data:") {
                    data.push(v.strip_prefix(' ').unwrap_or(v));
                }
            }
            if !data.is_empty() {
                out.push((event, data.join("\n")));
            }
        }
        out
    }
}

/// 空行(`\n\n` または `\r\n\r\n`)の位置と区切りの長さ。
fn find_event_end(buf: &[u8]) -> Option<(usize, usize)> {
    (0..buf.len()).find_map(|i| {
        if buf[i..].starts_with(b"\n\n") {
            Some((i, 2))
        } else if buf[i..].starts_with(b"\r\n\r\n") {
            Some((i, 4))
        } else {
            None
        }
    })
}

enum Block {
    Text,
    Tool { name: String, json: String },
}

#[derive(Default)]
pub struct StreamState {
    blocks: HashMap<u64, Block>,
    pub finished: bool,
}

impl StreamState {
    pub fn handle(&mut self, event: &str, data: &str) -> AppResult<Vec<StreamEvent>> {
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            return Ok(vec![]); // 壊れたイベントは無視(ping など)
        };
        let kind = v["type"].as_str().unwrap_or(event);
        let index = v["index"].as_u64().unwrap_or(0);
        let mut out = Vec::new();
        match kind {
            "content_block_start" => {
                let cb = &v["content_block"];
                let block = match cb["type"].as_str() {
                    Some("tool_use") => Block::Tool {
                        name: cb["name"].as_str().unwrap_or_default().to_string(),
                        json: String::new(),
                    },
                    _ => Block::Text,
                };
                self.blocks.insert(index, block);
                if let Some(t) = cb["text"].as_str().filter(|t| !t.is_empty()) {
                    out.push(StreamEvent::Text(t.to_string()));
                }
            }
            "content_block_delta" => {
                let d = &v["delta"];
                match d["type"].as_str() {
                    Some("text_delta") => {
                        if let Some(t) = d["text"].as_str() {
                            out.push(StreamEvent::Text(t.to_string()));
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(Block::Tool { json, .. }) = self.blocks.get_mut(&index) {
                            json.push_str(d["partial_json"].as_str().unwrap_or_default());
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(Block::Tool { name, json }) = self.blocks.remove(&index) {
                    let input = if json.trim().is_empty() {
                        json!({})
                    } else {
                        // 壊れた入力は提案なしとして扱う(回答テキストは有効)
                        serde_json::from_str(&json).unwrap_or(Value::Null)
                    };
                    if !input.is_null() {
                        out.push(StreamEvent::ToolUse { name, input });
                    }
                }
            }
            "message_delta" => {
                if let Some(r) = v["delta"]["stop_reason"].as_str() {
                    out.push(StreamEvent::Stop(r.to_string()));
                }
            }
            "message_stop" => self.finished = true,
            "error" => {
                let ty = v["error"]["type"].as_str().unwrap_or_default();
                return Err(match ty {
                    "overloaded_error" | "rate_limit_error" => map_status(429, None),
                    "authentication_error" | "permission_error" => auth_error(),
                    _ => AppError::assistant(
                        "ASSISTANT_API_ERROR",
                        "API エラーが発生しました(ストリーム中)",
                    ),
                });
            }
            _ => {}
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_parser_handles_split_chunks_and_crlf() {
        let mut p = SseParser::default();
        assert!(p.feed(b"event: a\ndata: {\"x\"").is_empty());
        let out = p.feed(b":1}\n\nevent: b\r\ndata: 2\r\n\r\n");
        assert_eq!(
            out,
            vec![
                ("a".to_string(), "{\"x\":1}".to_string()),
                ("b".to_string(), "2".to_string())
            ]
        );
    }

    #[test]
    fn sse_parser_survives_utf8_split_across_chunks() {
        let full = "event: t\ndata: こんにちは\n\n".as_bytes();
        let mut p = SseParser::default();
        let mid = full.len() - 7; // 日本語の途中
        assert!(p.feed(&full[..mid]).is_empty());
        let out = p.feed(&full[mid..]);
        assert_eq!(out, vec![("t".to_string(), "こんにちは".to_string())]);
    }

    fn run(events: &[(&str, &str)]) -> AppResult<Vec<StreamEvent>> {
        let mut st = StreamState::default();
        let mut all = Vec::new();
        for (e, d) in events {
            all.extend(st.handle(e, d)?);
        }
        Ok(all)
    }

    #[test]
    fn collects_text_and_tool_use() {
        let out = run(&[
            ("content_block_start", r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#),
            ("content_block_delta", r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"こんに"}}"#),
            ("content_block_delta", r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ちは"}}"#),
            ("content_block_stop", r#"{"type":"content_block_stop","index":0}"#),
            ("content_block_start", r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"t","name":"suggest_params","input":{}}}"#),
            ("content_block_delta", r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"summary\":\"a\","}}"#),
            ("content_block_delta", r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"changes\":{\"steps\":30}}"}}"#),
            ("content_block_stop", r#"{"type":"content_block_stop","index":1}"#),
        ])
        .unwrap();
        assert_eq!(
            out,
            vec![
                StreamEvent::Text("こんに".into()),
                StreamEvent::Text("ちは".into()),
                StreamEvent::ToolUse {
                    name: "suggest_params".into(),
                    input: json!({"summary":"a","changes":{"steps":30}})
                }
            ]
        );
    }

    #[test]
    fn broken_tool_json_yields_no_suggestion() {
        let out = run(&[
            ("content_block_start", r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","name":"suggest_params"}}"#),
            ("content_block_delta", r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"sum"}}"#),
            ("content_block_stop", r#"{"type":"content_block_stop","index":0}"#),
        ])
        .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn stream_error_event_maps_to_codes() {
        let e = run(&[(
            "error",
            r#"{"type":"error","error":{"type":"overloaded_error","message":"x"}}"#,
        )])
        .unwrap_err();
        assert_eq!(e.code(), "ASSISTANT_RATE_LIMIT");
        let e = run(&[(
            "error",
            r#"{"type":"error","error":{"type":"api_error","message":"secret detail"}}"#,
        )])
        .unwrap_err();
        assert_eq!(e.code(), "ASSISTANT_API_ERROR");
        assert!(!e.to_string().contains("secret"), "本文は出さない");
    }

    #[test]
    fn status_mapping() {
        assert_eq!(map_status(401, None).code(), "ASSISTANT_AUTH");
        assert_eq!(map_status(403, None).code(), "ASSISTANT_AUTH");
        assert_eq!(map_status(429, None).code(), "ASSISTANT_RATE_LIMIT");
        assert_eq!(map_status(529, None).code(), "ASSISTANT_RATE_LIMIT");
        let e = map_status(500, Some("req_1"));
        assert_eq!(e.code(), "ASSISTANT_API_ERROR");
        assert!(e.to_string().contains("500") && e.to_string().contains("req_1"));
    }

    /// 実際の Anthropic API に対する疎通確認(通常は実行されない)。
    /// `ANTHROPIC_API_KEY` が必要で、少量のトークン(最安のモデル)を消費する。
    #[tokio::test]
    #[ignore = "実際の API キーが必要(ANTHROPIC_API_KEY)"]
    async fn real_anthropic_verify_and_stream() {
        let key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY");
        let c = AnthropicClient::new().unwrap();
        assert!(c.verify_key(&key).await.unwrap());
        let (_tx, mut rx) = watch::channel(false);
        let messages = vec![("user".to_string(), "「OK」とだけ答えてください".to_string())];
        let mut text = String::new();
        let end = c
            .stream(
                &key,
                &MessageRequest {
                    model: "claude-haiku-5-5",
                    system: "簡潔に答える",
                    messages: &messages,
                    tools: json!([]),
                },
                &mut rx,
                |e| {
                    if let StreamEvent::Text(t) = e {
                        text.push_str(&t);
                    }
                },
            )
            .await
            .unwrap();
        assert_eq!(end, StreamEnd::Completed);
        assert!(!text.is_empty());
    }
}
