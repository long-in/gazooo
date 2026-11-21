//! OpenAI 互換のローカルサーバ(llama.cpp の `llama-server`・Ollama 等)のクライアント
//! (docs/11 §13・docs/30)。通信先はこの PC 内(ループバック)に限る。
//!
//! ログやエラーメッセージには、プロンプト・回答本文を含めない(ステータスのみ)。

use std::collections::BTreeMap;
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::anthropic::{MAX_TOKENS, MessageRequest, SseParser, StreamEnd, StreamEvent, cancelled};
use crate::error::{AppError, AppResult};

/// 接続先の URL を検証して正規化する。ループバック(`127.0.0.1` / `localhost` / `::1`)の
/// http(s) だけを許可する(docs/11 §13.1)。末尾の `/` は取り除く。
pub fn validate_base_url(raw: &str) -> AppResult<String> {
    let bad = |m: &str| AppError::validation("localBaseUrl", m);
    let raw = raw.trim();
    let url = reqwest::Url::parse(raw).map_err(|_| bad("接続先 URL の形式が正しくありません"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(bad("接続先は http:// または https:// で指定してください"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(bad("接続先 URL にユーザー情報は含められません"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(bad("接続先 URL にクエリやフラグメントは含められません"));
    }
    // IPv6 は `[::1]` の形で返る。URL パーサが数値表記(`0x7f.1` など)を正規化済み
    let loopback = url.host_str().is_some_and(|h| {
        let h = h.trim_start_matches('[').trim_end_matches(']');
        match h.parse::<std::net::IpAddr>() {
            Ok(ip) => ip.is_loopback(),
            Err(_) => h.eq_ignore_ascii_case("localhost"),
        }
    });
    if !loopback {
        return Err(bad(
            "接続先はこの PC 内(127.0.0.1 / localhost / ::1)のみ指定できます",
        ));
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub struct OpenAiClient {
    http: reqwest::Client,
}

impl Default for OpenAiClient {
    fn default() -> Self {
        Self::new()
    }
}

/// `GET {base}/models` の結果(接続の確認。docs/11 §13.4)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalCheck {
    pub reachable: bool,
    pub model_found: bool,
    pub models: Vec<String>,
}

impl OpenAiClient {
    pub fn new() -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            // ローカルの推論は最初のトークンまでに時間がかかる(GPU が小さいと長い入力の処理に数分かかる)
            .read_timeout(Duration::from_secs(600))
            // ローカルへの通信にプロキシは使わない
            .no_proxy()
            .build()
            // ビルダーの設定は固定値のため、失敗は TLS 初期化の異常時のみ。既定のクライアントで代替する
            .unwrap_or_default();
        Self { http }
    }

    /// サーバに届くか、指定のモデルが一覧にあるかを確認する(トークンは消費しない)。
    /// 届かないときはエラーにせず `reachable = false` を返す。
    pub async fn check(&self, base: &str, model: &str) -> LocalCheck {
        let resp = self
            .http
            .get(format!("{base}/models"))
            .timeout(Duration::from_secs(5))
            .send()
            .await;
        let Ok(resp) = resp else {
            return LocalCheck {
                reachable: false,
                model_found: false,
                models: vec![],
            };
        };
        let models = if resp.status().is_success() {
            resp.json::<Value>()
                .await
                .ok()
                .map(|v| parse_models(&v))
                .unwrap_or_default()
        } else {
            vec![]
        };
        LocalCheck {
            reachable: true,
            model_found: models.iter().any(|m| m == model),
            models,
        }
    }

    /// メッセージを送り、ストリームを `on_event` へ流す。`cancel` が true になったら中断する。
    pub async fn stream(
        &self,
        base: &str,
        model: &str,
        req: &MessageRequest<'_>,
        cancel: &mut watch::Receiver<bool>,
        mut on_event: impl FnMut(StreamEvent),
    ) -> AppResult<StreamEnd> {
        let body = build_body(model, req);
        let resp = tokio::select! {
            _ = cancelled(cancel) => return Ok(StreamEnd::Cancelled),
            r = self
                .http
                .post(format!("{base}/chat/completions"))
                .json(&body)
                .send() => r.map_err(|e| network_error(&e, base))?,
        };
        let status = resp.status();
        if !status.is_success() {
            return Err(map_status(status.as_u16()));
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
            let chunk = chunk.map_err(|e| network_error(&e, base))?;
            for (_event, data) in parser.feed(&chunk) {
                for ev in state.handle(&data) {
                    on_event(ev);
                }
                if state.finished {
                    for ev in state.finish() {
                        on_event(ev);
                    }
                    return Ok(StreamEnd::Completed);
                }
            }
        }
        // `[DONE]` を受けずに閉じた場合も、ここまでの内容は有効として扱う
        for ev in state.finish() {
            on_event(ev);
        }
        Ok(StreamEnd::Completed)
    }
}

fn parse_models(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str().map(str::to_owned))
        .collect();
    // Ollama / llama-server の別形式(`models: [{ name | model }]`)にも対応する
    for m in v["models"].as_array().into_iter().flatten() {
        for k in ["model", "name"] {
            if let Some(s) = m[k].as_str()
                && !out.iter().any(|o| o == s)
            {
                out.push(s.to_string());
            }
        }
    }
    out
}

/// Anthropic 形式のツール定義(`name` / `description` / `input_schema`)を OpenAI 形式へ変換する。
pub fn convert_tools(tools: &Value) -> Value {
    let list: Vec<Value> = tools
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t["name"],
                    "description": t["description"],
                    "parameters": t["input_schema"],
                }
            })
        })
        .collect();
    Value::Array(list)
}

fn build_body(model: &str, req: &MessageRequest<'_>) -> Value {
    let mut messages = vec![json!({ "role": "system", "content": req.system })];
    messages.extend(
        req.messages
            .iter()
            .map(|(role, text)| json!({ "role": role, "content": text })),
    );
    let tools = convert_tools(&req.tools);
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "max_tokens": MAX_TOKENS,
        // Qwen3.5 の思考を無効化する(対応しないサーバは無視する。docs/11 §13.3)
        "chat_template_kwargs": { "enable_thinking": false },
    });
    // ツール無しの呼び出し(翻訳。docs/34)では `tools` を送らない
    if tools.as_array().is_none_or(|t| !t.is_empty()) {
        body["tools"] = tools;
    }
    body
}

fn network_error(e: &reqwest::Error, base: &str) -> AppError {
    if e.is_connect() || e.is_timeout() || e.is_request() || e.is_body() {
        AppError::assistant(
            "ASSISTANT_UNREACHABLE",
            format!("ローカル LLM サーバに接続できません({base})。起動しているか確認してください"),
        )
    } else {
        AppError::assistant(
            "ASSISTANT_API_ERROR",
            "ローカル LLM の呼び出しに失敗しました",
        )
    }
}

fn map_status(status: u16) -> AppError {
    match status {
        429 | 503 => AppError::assistant(
            "ASSISTANT_RATE_LIMIT",
            "ローカル LLM が混み合っています。しばらく待ってからもう一度お試しください",
        ),
        _ => AppError::assistant(
            "ASSISTANT_API_ERROR",
            format!(
                "ローカル LLM がエラーを返しました(HTTP {status})。モデル名と接続先を確認してください"
            ),
        ),
    }
}

/// `<think>…</think>` を取り除く(タグがチャンクをまたいでもよい)。docs/11 §13.3。
#[derive(Default)]
struct ThinkFilter {
    inside: bool,
    /// タグの途中かもしれない末尾(次のチャンクと合わせて判定する)。
    pending: String,
}

impl ThinkFilter {
    const OPEN: &'static str = "<think>";
    const CLOSE: &'static str = "</think>";

    fn push(&mut self, text: &str) -> String {
        let mut buf = std::mem::take(&mut self.pending);
        buf.push_str(text);
        let mut out = String::new();
        loop {
            let tag = if self.inside { Self::CLOSE } else { Self::OPEN };
            if let Some(i) = buf.find(tag) {
                if !self.inside {
                    out.push_str(&buf[..i]);
                }
                buf = buf[i + tag.len()..].to_string();
                self.inside = !self.inside;
                continue;
            }
            // タグの前半で終わっている可能性がある末尾は、次に持ち越す
            let keep = (1..tag.len())
                .rev()
                .find(|n| {
                    buf.is_char_boundary(buf.len().saturating_sub(*n))
                        && buf.len() >= *n
                        && tag.starts_with(&buf[buf.len() - n..])
                })
                .unwrap_or(0);
            let cut = buf.len() - keep;
            if !self.inside {
                out.push_str(&buf[..cut]);
            }
            self.pending = buf[cut..].to_string();
            return out;
        }
    }

    /// ストリームの終わりに残った分(タグでなかった持ち越し)。
    fn flush(&mut self) -> String {
        let rest = std::mem::take(&mut self.pending);
        if self.inside { String::new() } else { rest }
    }
}

#[derive(Default)]
struct ToolAcc {
    name: String,
    args: String,
}

#[derive(Default)]
pub struct StreamState {
    think: ThinkFilter,
    tools: BTreeMap<u64, ToolAcc>,
    pub finished: bool,
}

impl StreamState {
    /// SSE の 1 イベント(`data:` の中身)を処理する。
    pub fn handle(&mut self, data: &str) -> Vec<StreamEvent> {
        let data = data.trim();
        if data == "[DONE]" {
            self.finished = true;
            return vec![];
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            return vec![]; // 壊れたイベントは無視
        };
        let mut out = Vec::new();
        for choice in v["choices"].as_array().into_iter().flatten() {
            let delta = &choice["delta"];
            // `reasoning_content`(思考)は表示も保存もしない
            if let Some(t) = delta["content"].as_str() {
                let t = self.think.push(t);
                if !t.is_empty() {
                    out.push(StreamEvent::Text(t));
                }
            }
            for call in delta["tool_calls"].as_array().into_iter().flatten() {
                let index = call["index"].as_u64().unwrap_or(0);
                let acc = self.tools.entry(index).or_default();
                if let Some(n) = call["function"]["name"].as_str() {
                    acc.name.push_str(n);
                }
                if let Some(a) = call["function"]["arguments"].as_str() {
                    acc.args.push_str(a);
                }
            }
            if choice["finish_reason"].as_str().is_some() {
                self.finished = true;
            }
        }
        out
    }

    /// 終わりに、持ち越した文字とツール呼び出しを確定して返す。
    pub fn finish(&mut self) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        let rest = self.think.flush();
        if !rest.is_empty() {
            out.push(StreamEvent::Text(rest));
        }
        for (_, acc) in std::mem::take(&mut self.tools) {
            if acc.name.is_empty() {
                continue;
            }
            // 引数が壊れた JSON のときは提案を捨てる(回答テキストは通常どおり)
            if let Ok(input) = serde_json::from_str::<Value>(&acc.args) {
                out.push(StreamEvent::ToolUse {
                    name: acc.name,
                    input,
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_must_be_loopback() {
        for ok in [
            "http://127.0.0.1:11434/v1",
            "http://localhost:8080/v1/",
            "http://[::1]:8080/v1",
            "https://127.0.0.1/v1",
        ] {
            assert!(validate_base_url(ok).is_ok(), "{ok}");
        }
        assert_eq!(
            validate_base_url(" http://localhost:8080/v1/ ").unwrap(),
            "http://localhost:8080/v1"
        );
        for ng in [
            "http://192.168.0.5:11434/v1",
            "http://example.com/v1",
            "http://127.0.0.1.evil.com/v1",
            "http://localhost.evil.com/v1",
            "http://user:pw@127.0.0.1/v1",
            "ftp://127.0.0.1/v1",
            "http://0.0.0.0:8080/v1",
            "not a url",
            "",
        ] {
            assert!(validate_base_url(ng).is_err(), "{ng}");
        }
    }

    #[test]
    fn converts_tool_definitions() {
        let t = json!([{"name":"a","description":"d","input_schema":{"type":"object"}}]);
        let c = convert_tools(&t);
        assert_eq!(c[0]["type"], "function");
        assert_eq!(c[0]["function"]["name"], "a");
        assert_eq!(c[0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn body_has_system_first_and_disables_thinking() {
        let msgs = vec![("user".to_string(), "hi".to_string())];
        let req = MessageRequest {
            model: "m",
            system: "SYS",
            messages: &msgs,
            tools: json!([]),
        };
        let b = build_body("m", &req);
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][1]["content"], "hi");
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(b["stream"], true);
    }

    fn run(chunks: &[&str]) -> (String, Vec<(String, Value)>) {
        let mut st = StreamState::default();
        let (mut text, mut tools) = (String::new(), vec![]);
        let mut sink = |evs: Vec<StreamEvent>| {
            for e in evs {
                match e {
                    StreamEvent::Text(t) => text.push_str(&t),
                    StreamEvent::ToolUse { name, input } => tools.push((name, input)),
                    StreamEvent::Stop(_) => {}
                }
            }
        };
        for c in chunks {
            sink(st.handle(c));
        }
        sink(st.finish());
        (text, tools)
    }

    #[test]
    fn streams_text_and_assembles_fragmented_tool_calls() {
        let (text, tools) = run(&[
            r#"{"choices":[{"delta":{"content":"steps を"}}]}"#,
            r#"{"choices":[{"delta":{"content":"増やします"}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"suggest_params","arguments":"{\"summary\":\"増"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"やす\",\"changes\":{\"steps\":40}}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "[DONE]",
        ]);
        assert_eq!(text, "steps を増やします");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].0, "suggest_params");
        assert_eq!(tools[0].1["changes"]["steps"], 40);
    }

    #[test]
    fn broken_tool_arguments_are_dropped_but_text_stays() {
        let (text, tools) = run(&[
            r#"{"choices":[{"delta":{"content":"ok"}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"suggest_params","arguments":"{\"summary\":"}}]}}]}"#,
            "[DONE]",
        ]);
        assert_eq!(text, "ok");
        assert!(tools.is_empty());
    }

    #[test]
    fn thinking_is_stripped_across_chunk_boundaries() {
        let (text, _) = run(&[
            r#"{"choices":[{"delta":{"content":"<thi"}}]}"#,
            r#"{"choices":[{"delta":{"content":"nk>考え中…</th"}}]}"#,
            r#"{"choices":[{"delta":{"content":"ink>答えです"}}]}"#,
            "[DONE]",
        ]);
        assert_eq!(text, "答えです");
    }

    #[test]
    fn reasoning_content_is_ignored_and_plain_angle_brackets_survive() {
        let (text, _) = run(&[
            r#"{"choices":[{"delta":{"reasoning_content":"秘密の思考","content":""}}]}"#,
            r#"{"choices":[{"delta":{"content":"a < b と <"}}]}"#,
            r#"{"choices":[{"delta":{"content":"b> です"}}]}"#,
            "[DONE]",
        ]);
        assert_eq!(text, "a < b と <b> です");
    }

    #[test]
    fn unterminated_think_block_is_dropped() {
        let (text, _) = run(&[
            r#"{"choices":[{"delta":{"content":"前<think>途中"}}]}"#,
            "[DONE]",
        ]);
        assert_eq!(text, "前");
    }

    #[test]
    fn models_are_parsed_from_both_shapes() {
        let v = json!({"data":[{"id":"qwen3.5:9b"}],"models":[{"name":"qwen3.5:9b","model":"qwen3.5:9b"},{"model":"x"}]});
        assert_eq!(parse_models(&v), ["qwen3.5:9b", "x"]);
    }
}
