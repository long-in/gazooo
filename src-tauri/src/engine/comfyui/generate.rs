//! 生成の投入・完了待ち・キャンセル(ComfyUI の `/prompt` `/history` `/view` `/ws`)。

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::parse::{self, HistoryState, ImageRef};
use super::workflow;
use super::{ComfyUiClient, error_chain, log_http_failure};
use crate::domain::job::ProgressPhase;
use crate::domain::params::GenerationParams;
use crate::engine::{ImageData, Progress};
use crate::error::{AppError, AppResult};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 履歴の取得が連続して失敗したら、エンジンが落ちたとみなして諦める回数。
const MAX_CONSECUTIVE_POLL_FAILURES: u32 = 20;
/// 顔の有無の事前確認(検出のみ。数秒で終わる)を待つ上限。検出モデルの初回読み込みを見込む。
const FACE_PROBE_TIMEOUT: Duration = Duration::from_secs(120);

fn unreachable(e: reqwest::Error) -> AppError {
    log_http_failure(&e);
    AppError::EngineUnreachable(e.without_url().to_string())
}

impl ComfyUiClient {
    /// 参照画像を `POST /upload/image`(multipart、`type=input`)で送り、`LoadImage` に渡す名前を返す。
    /// ファイル名は UUID を含めて衝突・上書きを避ける(ComfyUI 側の input に残るファイルは削除しない)。
    pub(super) async fn upload_image(&self, png: Vec<u8>) -> AppResult<String> {
        let filename = format!("gazooo-{}.png", uuid::Uuid::now_v7());
        let part = reqwest::multipart::Part::bytes(png)
            .file_name(filename)
            .mime_str("image/png")
            .map_err(|e| AppError::internal(e.to_string()))?;
        let form = reqwest::multipart::Form::new()
            .part("image", part)
            .text("type", "input")
            .text("overwrite", "false");
        let resp = self
            .http
            .post(format!("{}/upload/image", self.base))
            .multipart(form)
            .send()
            .await
            .map_err(unreachable)?;
        if !resp.status().is_success() {
            return Err(AppError::EngineBadResponse(format!(
                "/upload/image: HTTP {}",
                resp.status()
            )));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| AppError::EngineBadResponse(e.without_url().to_string()))?;
        parse::parse_upload_name(&v)
            .ok_or_else(|| AppError::EngineBadResponse("/upload/image: name がありません".into()))
    }

    pub(super) async fn submit_prompt(
        &self,
        params: &GenerationParams,
        inputs: &[String],
    ) -> AppResult<String> {
        let needed = params.upload_count();
        if inputs.len() != needed {
            return Err(AppError::internal(format!(
                "参照画像の数が一致しません(必要 {needed}、指定 {})",
                inputs.len()
            )));
        }
        // 顔の入れ替えで後段合成を使うときは、重い生成の前に、対象画像に顔があるかを確認する
        // (顔のない画像で生成を始めると ComfyUI が止まることがあった。docs/46 §6 E)
        if let Some(f) = params
            .face_swap
            .as_ref()
            .filter(|f| f.enabled && f.keep_outside)
            && let Some(target) = inputs.first()
            && self.count_faces(target, &f.detector).await? == 0
        {
            return Err(AppError::FaceNotDetected);
        }
        self.post_prompt(workflow::build(params, "gazooo", inputs))
            .await
    }

    /// 対象画像の顔の数だけを調べる(モデル不要の軽いワークフロー。数秒で終わる)。
    async fn count_faces(&self, image: &str, detector: &str) -> AppResult<u64> {
        let id = self
            .post_prompt(workflow::build_face_probe(image, detector))
            .await?;
        let started = Instant::now();
        loop {
            let v = self.get_json(&format!("/history/{id}")).await?;
            let history = v.unwrap_or(Value::Null);
            match parse::parse_history(&history, &id) {
                HistoryState::Pending => {}
                HistoryState::Failed(msg) => return Err(AppError::EngineExecutionFailed(msg)),
                HistoryState::Done(_) => {
                    return parse::parse_count_output(&history, &id, workflow::FACE_PROBE_OUTPUT)
                        .ok_or_else(|| {
                            AppError::EngineBadResponse("顔の数を取得できませんでした".into())
                        });
                }
            }
            if started.elapsed() > FACE_PROBE_TIMEOUT {
                return Err(AppError::EngineExecutionFailed(
                    "顔の確認が時間内に終わりませんでした".into(),
                ));
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }

    /// 組み立て済みのワークフローを `POST /prompt` で投入し、エンジン側の ID を返す。
    pub(super) async fn post_prompt(&self, prompt: Value) -> AppResult<String> {
        let body = json!({
            "prompt": prompt,
            "client_id": self.client_id,
        });
        let resp = self
            .http
            .post(format!("{}/prompt", self.base))
            .json(&body)
            .send()
            .await
            .map_err(unreachable)?;

        let status = resp.status();
        if status == reqwest::StatusCode::BAD_REQUEST {
            // 検証エラー(モデル不在、ノード不在など)。本文で分類する。
            return Err(match resp.json::<Value>().await {
                Ok(v) => parse::parse_prompt_error(&v),
                Err(_) => AppError::EngineExecutionFailed(format!("HTTP {status}")),
            });
        }
        if !status.is_success() {
            return Err(AppError::EngineBadResponse(format!("HTTP {status}")));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| AppError::EngineBadResponse(e.without_url().to_string()))?;
        v.get("prompt_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| AppError::EngineBadResponse("/prompt: prompt_id がありません".into()))
    }

    pub(super) async fn wait_for_result(
        &self,
        id: &str,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> AppResult<Vec<ImageData>> {
        // 完了判定は履歴のポーリングで確実に行い、WebSocket は進捗のベストエフォート。
        let started = Instant::now();
        let mut ws = self.connect_ws().await;
        let mut ws_messages = 0u64;
        let mut last_progress = Instant::now();
        let mut ticker = tokio::time::interval(self.poll_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut failures = 0u32;

        loop {
            tokio::select! {
                msg = next_ws_message(&mut ws) => match msg {
                    Some(Ok(Message::Text(text))) => {
                        ws_messages += 1;
                        if let Some((step, total, node)) = parse::parse_ws_progress(text.as_str(), id) {
                            last_progress = Instant::now();
                            let phase = if node.as_deref() == Some(workflow::FACE_DETAILER_NODE) {
                                ProgressPhase::FaceDetail
                            } else {
                                ProgressPhase::Generate
                            };
                            on_progress(Progress {
                                step,
                                total,
                                node,
                                phase: Some(phase),
                            });
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        tracing::debug!(remote = id, ?frame, ws_messages, "websocket close frame received");
                    }
                    Some(Ok(_)) => ws_messages += 1,
                    // 切断・エラー: 以後はポーリングのみで続行する
                    Some(Err(e)) => {
                        tracing::debug!(
                            remote = id,
                            error = %error_chain(&e),
                            ws_messages,
                            since_last_progress_ms = last_progress.elapsed().as_millis() as u64,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "websocket error; falling back to polling"
                        );
                        ws = None;
                    }
                    None => {
                        tracing::debug!(
                            remote = id,
                            ws_messages,
                            since_last_progress_ms = last_progress.elapsed().as_millis() as u64,
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "websocket stream ended; falling back to polling"
                        );
                        ws = None;
                    }
                },
                _ = ticker.tick() => {
                    match self.get_json(&format!("/history/{id}")).await {
                        Ok(v) => {
                            if failures > 0 {
                                tracing::debug!(remote = id, failures, "history polling recovered");
                            }
                            failures = 0;
                            let history = v.unwrap_or(Value::Null);
                            match parse::parse_history(&history, id) {
                                HistoryState::Pending => {}
                                HistoryState::Failed(msg) => {
                                    return Err(AppError::EngineExecutionFailed(msg));
                                }
                                HistoryState::Done(images) => {
                                    // 顔の入れ替えの後段合成で、顔が 1 つも検出されなかった(元画像のままの出力は成功にしない)
                                    if parse::parse_count_output(&history, id, workflow::FACE_SWAP_COUNT_OUTPUT)
                                        == Some(0)
                                    {
                                        return Err(AppError::FaceNotDetected);
                                    }
                                    if images.is_empty() {
                                        return Err(AppError::EngineExecutionFailed(
                                            "画像が生成されませんでした".into(),
                                        ));
                                    }
                                    tracing::debug!(
                                        remote = id,
                                        images = images.len(),
                                        elapsed_ms = started.elapsed().as_millis() as u64,
                                        "generation finished; fetching images"
                                    );
                                    return self.fetch_images(&images).await;
                                }
                            }
                        }
                        Err(e) => {
                            failures += 1;
                            tracing::debug!(
                                remote = id,
                                failures,
                                code = e.code(),
                                error = %e,
                                elapsed_ms = started.elapsed().as_millis() as u64,
                                "history polling failed"
                            );
                            if failures >= MAX_CONSECUTIVE_POLL_FAILURES {
                                tracing::warn!(remote = id, failures, "giving up waiting: engine unreachable");
                                return Err(e);
                            }
                        }
                    }
                }
            }
        }
    }

    pub(super) async fn cancel_prompt(&self, id: &str) -> AppResult<()> {
        // 待機中なら取り消し、実行中なら中断する(このアプリは同時に 1 ジョブしか投入しない)
        let _ = self
            .http
            .post(format!("{}/queue", self.base))
            .json(&json!({ "delete": [id] }))
            .send()
            .await;
        let resp = self
            .http
            .post(format!("{}/interrupt", self.base))
            .send()
            .await
            .map_err(unreachable)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(AppError::EngineBadResponse(format!(
                "/interrupt: HTTP {}",
                resp.status()
            )))
        }
    }

    /// WebSocket に接続する。http のみ対応(https はポーリングだけで進捗なし)。失敗時は `None`。
    async fn connect_ws(&self) -> Option<WsStream> {
        let host = self.base.strip_prefix("http://")?;
        let url = format!("ws://{host}/ws?clientId={}", self.client_id);
        let started = Instant::now();
        match tokio::time::timeout(
            Duration::from_secs(3),
            tokio_tungstenite::connect_async(url),
        )
        .await
        {
            Ok(Ok((ws, _))) => {
                tracing::debug!(
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "websocket connected"
                );
                Some(ws)
            }
            Ok(Err(e)) => {
                tracing::debug!(error = %error_chain(&e), "websocket connect failed; progress via polling only");
                None
            }
            Err(_) => {
                tracing::debug!("websocket connect timed out; progress via polling only");
                None
            }
        }
    }

    async fn fetch_images(&self, refs: &[ImageRef]) -> AppResult<Vec<ImageData>> {
        let mut out = Vec::with_capacity(refs.len());
        for r in refs {
            let mut url = reqwest::Url::parse(&format!("{}/view", self.base))
                .map_err(|e| AppError::internal(e.to_string()))?;
            url.query_pairs_mut()
                .append_pair("filename", &r.filename)
                .append_pair("subfolder", &r.subfolder)
                .append_pair("type", &r.kind);
            let resp = self.http.get(url).send().await.map_err(unreachable)?;
            if !resp.status().is_success() {
                return Err(AppError::EngineBadResponse(format!(
                    "/view: HTTP {}",
                    resp.status()
                )));
            }
            let bytes = resp.bytes().await.map_err(unreachable)?;
            out.push(ImageData {
                filename: r.filename.clone(),
                bytes: bytes.to_vec(),
            });
        }
        Ok(out)
    }
}

/// WebSocket が無い(または切れた)場合は永遠に待つ(`select!` で他の枝だけが動く)。
async fn next_ws_message(
    ws: &mut Option<WsStream>,
) -> Option<Result<Message, tokio_tungstenite::tungstenite::Error>> {
    match ws {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::super::testutil::{Route, spawn, spawn_with_ws};
    use super::*;
    use crate::domain::params::{ModelKind, sample};
    use crate::engine::GenerationEngine;

    fn client(url: &str) -> ComfyUiClient {
        ComfyUiClient::new(url)
            .unwrap()
            .with_poll_interval(Duration::from_millis(20))
    }

    #[tokio::test]
    async fn submit_posts_workflow_and_returns_prompt_id() {
        let m = spawn(vec![Route::post(
            "/prompt",
            200,
            r#"{"prompt_id":"abc","number":1}"#,
        )])
        .await;
        let c = client(&m.url);
        let id = c.submit(&sample(ModelKind::Sdxl), &[]).await.unwrap();
        assert_eq!(id, "abc");

        let reqs = m.requests.lock().unwrap();
        let body = reqs
            .iter()
            .find(|r| r.starts_with("POST /prompt "))
            .unwrap();
        assert!(body.contains("CheckpointLoaderSimple"));
        assert!(body.contains(&c.client_id), "client_id should be sent");
    }

    #[tokio::test]
    async fn upload_sends_multipart_and_returns_name() {
        let m = spawn(vec![Route::post(
            "/upload/image",
            200,
            r#"{"name":"gazooo-x.png","subfolder":"","type":"input"}"#,
        )])
        .await;
        let name = client(&m.url)
            .upload_input(b"PNGDATA".to_vec())
            .await
            .unwrap();
        assert_eq!(name, "gazooo-x.png");

        let reqs = m.requests.lock().unwrap();
        let req = reqs
            .iter()
            .find(|r| r.starts_with("POST /upload/image"))
            .unwrap();
        assert!(req.contains("name=\"image\""), "{req}");
        assert!(req.contains("PNGDATA"));
        assert!(req.contains("name=\"type\"") && req.contains("input"));
        assert!(req.contains("filename=\"gazooo-"), "ファイル名は UUID 入り");
    }

    #[tokio::test]
    async fn upload_failures_are_classified() {
        let m = spawn(vec![Route::post("/upload/image", 500, "oops")]).await;
        let err = client(&m.url).upload_input(vec![1]).await.unwrap_err();
        assert_eq!(err.code(), "ENGINE_BAD_RESPONSE");
        let m = spawn(vec![Route::post("/upload/image", 200, r#"{"x":1}"#)]).await;
        let err = client(&m.url).upload_input(vec![1]).await.unwrap_err();
        assert_eq!(err.code(), "ENGINE_BAD_RESPONSE");
    }

    #[tokio::test]
    async fn submit_with_controlnet_embeds_uploaded_name_and_checks_count() {
        use crate::domain::params::{ControlnetParams, ControlnetUnit, Preprocessor};
        let mut p = sample(ModelKind::Sd15);
        p.controlnet = Some(ControlnetParams {
            enabled: true,
            units: vec![ControlnetUnit {
                image: "inputs/a.png".into(),
                model: "canny.safetensors".into(),
                preprocessor: Preprocessor::Canny,
                strength: 1.0,
                start_percent: 0.0,
                end_percent: 1.0,
            }],
        });
        let m = spawn(vec![Route::post("/prompt", 200, r#"{"prompt_id":"cn1"}"#)]).await;
        let c = client(&m.url);

        // 参照画像の数が合わなければ、エンジンへ送る前に断る
        assert_eq!(c.submit(&p, &[]).await.unwrap_err().code(), "INTERNAL");
        assert!(m.requests.lock().unwrap().is_empty());

        let id = c.submit(&p, &["gazooo-x.png".to_string()]).await.unwrap();
        assert_eq!(id, "cn1");
        let reqs = m.requests.lock().unwrap();
        let body = reqs
            .iter()
            .find(|r| r.starts_with("POST /prompt "))
            .unwrap();
        assert!(body.contains("ControlNetApplyAdvanced"));
        assert!(body.contains("gazooo-x.png"));
    }

    #[tokio::test]
    async fn submit_maps_validation_errors() {
        let model_missing = r#"{"error":{"type":"prompt_outputs_failed_validation","message":"Prompt outputs failed validation"},
            "node_errors":{"1":{"class_type":"CheckpointLoaderSimple","errors":[
            {"type":"value_not_in_list","details":"ckpt_name: 'x' not in []"}]}}}"#;
        let m = spawn(vec![Route::post("/prompt", 400, model_missing)]).await;
        let err = client(&m.url)
            .submit(&sample(ModelKind::Sd15), &[])
            .await
            .unwrap_err();
        assert_eq!(err.code(), "ENGINE_MODEL_MISSING");

        let m = spawn(vec![Route::post("/prompt", 500, "oops")]).await;
        let err = client(&m.url)
            .submit(&sample(ModelKind::Sd15), &[])
            .await
            .unwrap_err();
        assert_eq!(err.code(), "ENGINE_BAD_RESPONSE");
    }

    #[tokio::test]
    async fn wait_polls_until_done_and_fetches_images() {
        let done = r#"{"p1":{"status":{"status_str":"success","completed":true},
            "outputs":{"8":{"images":[{"filename":"a.png","subfolder":"","type":"output"}]}}}}"#;
        let m = spawn(vec![
            Route::seq(
                "GET /history/p1",
                vec![(200, "{}"), (200, "{}"), (200, done)],
            ),
            Route::bytes("GET /view", b"PNGDATA".to_vec()),
        ])
        .await;
        let images = client(&m.url).wait("p1", &|_| {}).await.unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].filename, "a.png");
        assert_eq!(images[0].bytes, b"PNGDATA");

        let reqs = m.requests.lock().unwrap();
        assert!(
            reqs.iter()
                .any(|r| r.starts_with("GET /view?filename=a.png&subfolder=&type=output"))
        );
    }

    #[tokio::test]
    async fn wait_reports_progress_from_websocket() {
        let progress =
            r#"{"type":"progress","data":{"value":5,"max":20,"prompt_id":"p1","node":"5"}}"#;
        let other = r#"{"type":"progress","data":{"value":9,"max":9,"prompt_id":"someone-else"}}"#;
        let done = r#"{"p1":{"status":{"status_str":"success","completed":true},
            "outputs":{"8":{"images":[{"filename":"a.png","subfolder":"","type":"output"}]}}}}"#;
        let m = spawn_with_ws(
            vec![
                // ポーリングは WebSocket 通知を受け取る時間を取るため、数回は空で返す
                Route::seq(
                    "GET /history/p1",
                    vec![
                        (200, "{}"),
                        (200, "{}"),
                        (200, "{}"),
                        (200, "{}"),
                        (200, done),
                    ],
                ),
                Route::bytes("GET /view", b"x".to_vec()),
            ],
            vec![other.to_string(), progress.to_string()],
        )
        .await;

        let seen: Arc<Mutex<Vec<Progress>>> = Arc::default();
        let sink = Arc::clone(&seen);
        client(&m.url)
            .wait("p1", &move |p| sink.lock().unwrap().push(p))
            .await
            .unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(
            *seen,
            [Progress {
                step: 5,
                total: 20,
                node: Some("5".into()),
                phase: Some(ProgressPhase::Generate),
            }],
            "他のジョブの進捗は無視され、自分の進捗だけが通知される"
        );
    }

    #[tokio::test]
    async fn progress_on_face_detailer_node_is_tagged_as_face_phase() {
        let main_pass =
            r#"{"type":"progress","data":{"value":20,"max":20,"prompt_id":"p1","node":"5"}}"#;
        let face_pass = format!(
            r#"{{"type":"progress","data":{{"value":3,"max":10,"prompt_id":"p1","node":"{}"}}}}"#,
            workflow::FACE_DETAILER_NODE
        );
        let done = r#"{"p1":{"status":{"status_str":"success","completed":true},
            "outputs":{"8":{"images":[{"filename":"a.png","subfolder":"","type":"output"}]}}}}"#;
        let m = spawn_with_ws(
            vec![
                Route::seq(
                    "GET /history/p1",
                    vec![
                        (200, "{}"),
                        (200, "{}"),
                        (200, "{}"),
                        (200, "{}"),
                        (200, done),
                    ],
                ),
                Route::bytes("GET /view", b"x".to_vec()),
            ],
            vec![main_pass.to_string(), face_pass],
        )
        .await;

        let seen: Arc<Mutex<Vec<Progress>>> = Arc::default();
        let sink = Arc::clone(&seen);
        client(&m.url)
            .wait("p1", &move |p| sink.lock().unwrap().push(p))
            .await
            .unwrap();
        let phases: Vec<_> = seen.lock().unwrap().iter().map(|p| p.phase).collect();
        assert_eq!(
            phases,
            [
                Some(ProgressPhase::Generate),
                Some(ProgressPhase::FaceDetail)
            ]
        );
    }

    #[tokio::test]
    async fn wait_returns_execution_error() {
        let failed = r#"{"p1":{"status":{"status_str":"error","completed":false,"messages":[
            ["execution_error",{"node_type":"KSampler","exception_message":"MPS backend out of memory"}]]}}}"#;
        let m = spawn(vec![Route::get("/history/p1", failed)]).await;
        let err = client(&m.url).wait("p1", &|_| {}).await.unwrap_err();
        assert_eq!(err.code(), "ENGINE_EXECUTION_FAILED");
        assert!(err.to_string().contains("out of memory"));
    }

    #[tokio::test]
    async fn wait_fails_when_no_images_produced() {
        let empty = r#"{"p1":{"status":{"status_str":"success","completed":true},"outputs":{}}}"#;
        let m = spawn(vec![Route::get("/history/p1", empty)]).await;
        let err = client(&m.url).wait("p1", &|_| {}).await.unwrap_err();
        assert_eq!(err.code(), "ENGINE_EXECUTION_FAILED");
    }

    fn face_swap_job(keep_outside: bool) -> GenerationParams {
        use crate::domain::params::{FACE_SWAP_DETECTOR, FaceSwapParams, FaceSwapRegion};
        let mut p = sample(ModelKind::QwenImage);
        p.face_swap = Some(FaceSwapParams {
            enabled: true,
            target: "inputs/t.png".into(),
            face: "inputs/f.png".into(),
            keep_outside,
            region: FaceSwapRegion::Head,
            resolution: 1024,
            detector: FACE_SWAP_DETECTOR.into(),
        });
        p
    }

    fn history_with_count(count: u32) -> String {
        format!(
            r#"{{"probe":{{"status":{{"status_str":"success","completed":true}},"outputs":{{"4":{{"text":["{count}"]}}}}}}}}"#
        )
    }

    #[tokio::test]
    async fn face_swap_stops_before_generating_when_the_target_has_no_face() {
        let m = spawn(vec![
            Route::post("/prompt", 200, r#"{"prompt_id":"probe","number":1}"#),
            Route::get("/history/probe", &history_with_count(0)),
        ])
        .await;
        let err = client(&m.url)
            .submit(&face_swap_job(true), &["t.png".into(), "f.png".into()])
            .await
            .unwrap_err();
        assert_eq!(err.code(), "FACE_NOT_DETECTED");
        // 投入したのは顔の確認だけで、重い生成は投入していない
        let posts = m
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.starts_with("POST /prompt "))
            .count();
        assert_eq!(posts, 1);
    }

    #[tokio::test]
    async fn face_swap_generates_after_the_probe_finds_a_face() {
        let m = spawn(vec![
            Route::post("/prompt", 200, r#"{"prompt_id":"probe","number":1}"#),
            Route::get("/history/probe", &history_with_count(1)),
        ])
        .await;
        let id = client(&m.url)
            .submit(&face_swap_job(true), &["t.png".into(), "f.png".into()])
            .await
            .unwrap();
        assert_eq!(id, "probe");
        let posts = m
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.starts_with("POST /prompt "))
            .count();
        assert_eq!(posts, 2, "顔の確認 + 生成");
    }

    #[tokio::test]
    async fn face_swap_without_the_composite_skips_the_probe() {
        let m = spawn(vec![Route::post(
            "/prompt",
            200,
            r#"{"prompt_id":"g1","number":1}"#,
        )])
        .await;
        let id = client(&m.url)
            .submit(&face_swap_job(false), &["t.png".into(), "f.png".into()])
            .await
            .unwrap();
        assert_eq!(id, "g1");
    }

    #[tokio::test]
    async fn wait_fails_with_face_not_detected_when_the_detected_count_is_zero() {
        // 合成の結果の画像は出ているが、検出した顔が 0 件(元画像のままの出力を成功にしない。docs/46 §6 E)
        let none = r#"{"p1":{"status":{"status_str":"success","completed":true},
            "outputs":{"9":{"images":[{"filename":"a.png","subfolder":"","type":"output"}]},"122":{"text":["0"]}}}}"#;
        let m = spawn(vec![Route::get("/history/p1", none)]).await;
        let err = client(&m.url).wait("p1", &|_| {}).await.unwrap_err();
        assert_eq!(err.code(), "FACE_NOT_DETECTED");
    }

    #[tokio::test]
    async fn cancel_deletes_from_queue_and_interrupts() {
        let m = spawn(vec![
            Route::post("/queue", 200, "{}"),
            Route::post("/interrupt", 200, ""),
        ])
        .await;
        client(&m.url).cancel("p1").await.unwrap();
        let reqs = m.requests.lock().unwrap();
        assert!(
            reqs.iter()
                .any(|r| r.starts_with("POST /queue ") && r.contains(r#""delete":["p1"]"#))
        );
        assert!(reqs.iter().any(|r| r == "POST /interrupt"));
    }

    /// 実機: 存在しないモデルの投入が ENGINE_MODEL_MISSING に分類されること(通常は実行しない)。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI"]
    async fn real_comfyui_missing_model_is_classified() {
        let url = std::env::var("COMFYUI_URL").unwrap_or_else(|_| "http://127.0.0.1:8188".into());
        let mut p = sample(ModelKind::Sdxl);
        p.model.name = "definitely-not-installed.safetensors".into();
        let err = ComfyUiClient::new(&url)
            .unwrap()
            .submit(&p, &[])
            .await
            .unwrap_err();
        println!("{err}");
        assert_eq!(err.code(), "ENGINE_MODEL_MISSING");

        // GGUF ノードは導入済みなので、存在しない GGUF モデルもモデル不在になる
        let mut p = sample(ModelKind::FluxGguf);
        p.model.name = "nope.gguf".into();
        let err = ComfyUiClient::new(&url)
            .unwrap()
            .submit(&p, &[])
            .await
            .unwrap_err();
        println!("{err}");
        assert_eq!(err.code(), "ENGINE_MODEL_MISSING");
    }

    /// 実機: 7 種類すべてのワークフローが ComfyUI のバリデータに構造的に受理されること。
    /// モデルが未導入でも、検出されるエラーが「モデル名が一覧に無い」だけであれば、
    /// ノード種別・必須入力・リンクの型はすべて正しいと確認できる(通常は実行しない)。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI"]
    async fn real_comfyui_workflows_are_structurally_valid() {
        let url = std::env::var("COMFYUI_URL").unwrap_or_else(|_| "http://127.0.0.1:8188".into());
        let c = ComfyUiClient::new(&url).unwrap();
        let model_inputs = [
            "ckpt_name",
            "unet_name",
            "clip_name",
            "clip_name1",
            "clip_name2",
            "vae_name",
        ];

        for kind in [
            ModelKind::Sd15,
            ModelKind::Sdxl,
            ModelKind::Flux,
            ModelKind::FluxGguf,
            ModelKind::Anima,
            ModelKind::QwenImage,
            ModelKind::Krea2,
        ] {
            let mut p = sample(kind);
            p.model.name = "not-installed.safetensors".into();
            let body =
                json!({ "prompt": workflow::build(&p, "gazooo", &[]), "client_id": c.client_id });
            let resp = c
                .http
                .post(format!("{}/prompt", c.base))
                .json(&body)
                .send()
                .await
                .unwrap();
            let v: Value = resp.json().await.unwrap();

            let mut seen = vec![];
            for (node, ne) in v["node_errors"].as_object().expect("node_errors") {
                for e in ne["errors"].as_array().unwrap() {
                    let kind_str = e["type"].as_str().unwrap_or("?");
                    let detail = e["details"].as_str().unwrap_or("");
                    let input = detail.split(':').next().unwrap_or("");
                    seen.push(format!("{node}/{}: {kind_str} {input}", ne["class_type"]));
                    assert_eq!(kind_str, "value_not_in_list", "{kind:?} node {node}: {e}");
                    assert!(
                        model_inputs.contains(&input),
                        "{kind:?} node {node}: {detail}"
                    );
                }
            }
            assert!(
                !seen.is_empty(),
                "{kind:?}: ComfyUI accepted a non-existent model?! {v}"
            );
            println!("{kind:?}: {seen:?}");
        }
    }

    fn real_url() -> String {
        std::env::var("COMFYUI_URL").unwrap_or_else(|_| "http://127.0.0.1:8188".into())
    }

    /// 実機での検証用パラメータ(SD 1.5、小さく速い設定)。モデルは `COMFYUI_SD15_MODEL` で変更できる。
    fn real_sd15_params(steps: u32, seed: u64) -> GenerationParams {
        let mut p = sample(ModelKind::Sd15);
        p.model.name = std::env::var("COMFYUI_SD15_MODEL")
            .unwrap_or_else(|_| "v1-5-pruned-emaonly.safetensors".into());
        p.prompt = "a red apple on a wooden table, studio lighting".into();
        p.negative_prompt = "blurry, low quality".into();
        p.width = 512;
        p.height = 512;
        p.steps = steps;
        p.seed = seed;
        p
    }

    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    /// 実機: 画像が実際に生成され、進捗が通知され、PNG として取得できること。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with an SD 1.5 checkpoint"]
    async fn real_comfyui_generates_sd15_image_with_progress() {
        let c = ComfyUiClient::new(&real_url()).unwrap();
        let id = c
            .submit(&real_sd15_params(8, 1), &[])
            .await
            .expect("submit");

        let seen: Arc<Mutex<Vec<Progress>>> = Arc::default();
        let sink = Arc::clone(&seen);
        let images = c
            .wait(&id, &move |p| sink.lock().unwrap().push(p))
            .await
            .expect("wait");

        assert_eq!(images.len(), 1);
        assert_eq!(&images[0].bytes[..8], &PNG_MAGIC, "PNG として取得できる");
        assert!(images[0].bytes.len() > 10_000, "画像として妥当なサイズ");
        let decoded = image::load_from_memory(&images[0].bytes).expect("decodable");
        assert_eq!((decoded.width(), decoded.height()), (512, 512));

        let seen = seen.lock().unwrap();
        println!("progress events: {}", seen.len());
        assert!(!seen.is_empty(), "WebSocket から進捗を受信できる");
        assert_eq!(seen.last().unwrap().total, 8);
    }

    /// 実機: Anima(docs/17)で画像が生成されること。アプリの既定値(1024×1024・30 ステップ・CFG 4・euler / simple)と、
    /// 公式のファイル名のモデルを使う。`COMFYUI_SAVE_PNG` に出力先を指定すると、生成した PNG を保存する。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with the Anima files (docs/17 §2)"]
    async fn real_comfyui_generates_anima_image() {
        let c = ComfyUiClient::new(&real_url()).unwrap();
        let mut p = sample(ModelKind::Anima);
        p.model.name = "anima-base-v1.0.safetensors".into();
        p.model.clip = vec!["qwen_3_06b_base.safetensors".into()];
        p.model.vae = Some("qwen_image_vae.safetensors".into());
        p.prompt =
            "masterpiece, best quality, score_7, safe, 1girl, solo, silver hair, blue eyes, \
                    school uniform, standing in a flower field, smile, looking at viewer"
                .into();
        p.negative_prompt = "worst quality, low quality, score_1, score_2, score_3, artist name, blurry, jpeg artifacts"
            .into();
        p.width = 1024;
        p.height = 1024;
        p.steps = 30;
        p.cfg = 4.0;
        p.sampler = "euler".into();
        p.scheduler = "simple".into();
        p.seed = 20_261_119;

        let id = c.submit(&p, &[]).await.expect("submit");
        let seen: Arc<Mutex<Vec<Progress>>> = Arc::default();
        let sink = Arc::clone(&seen);
        let images = c
            .wait(&id, &move |pr| sink.lock().unwrap().push(pr))
            .await
            .expect("wait");

        assert_eq!(images.len(), 1);
        assert_eq!(&images[0].bytes[..8], &PNG_MAGIC, "PNG として取得できる");
        let decoded = image::load_from_memory(&images[0].bytes).expect("decodable");
        assert_eq!((decoded.width(), decoded.height()), (1024, 1024));
        assert!(
            images[0].bytes.len() > 100_000,
            "画像として妥当なサイズ(真っ黒・空ではない)"
        );
        let seen = seen.lock().unwrap();
        println!("progress events: {}, last: {:?}", seen.len(), seen.last());
        assert!(!seen.is_empty(), "WebSocket から進捗を受信できる");
        assert_eq!(seen.last().unwrap().total, 30);
        if let Ok(path) = std::env::var("COMFYUI_SAVE_PNG") {
            std::fs::write(&path, &images[0].bytes).expect("save png");
            println!("saved {path}");
        }
    }

    /// 実機: Qwen-Image 2.1(docs/17)で画像が生成されること。アプリの既定値(1024×1024・25 ステップ・CFG 1・euler / simple)と、
    /// INT8 convrot 版(推奨)のファイルを使う。`COMFYUI_SAVE_PNG` に出力先を指定すると、生成した PNG を保存する。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with the Qwen-Image 2.1 files (docs/17 §2)"]
    async fn real_comfyui_generates_qwen_image_21() {
        let c = ComfyUiClient::new(&real_url()).unwrap();
        let mut p = sample(ModelKind::QwenImage);
        p.model.name = "qwen_image_2.1_int8_convrot.safetensors".into();
        p.model.clip = vec!["qwen3vl_8b_int8_convrot.safetensors".into()];
        p.model.vae = Some("qwen_image_2.1_vae_bf16.safetensors".into());
        p.prompt = "A small wooden cafe on a rainy street at dusk, warm light in the windows, \
                    a hand-painted sign that reads \"OPEN\", reflections on the wet pavement, cinematic"
            .into();
        p.negative_prompt = String::new();
        p.width = 1024;
        p.height = 1024;
        p.steps = 25;
        p.cfg = 1.0;
        p.sampler = "euler".into();
        p.scheduler = "simple".into();
        p.seed = 20_261_119;

        let id = c.submit(&p, &[]).await.expect("submit");
        let seen: Arc<Mutex<Vec<Progress>>> = Arc::default();
        let sink = Arc::clone(&seen);
        let images = c
            .wait(&id, &move |pr| sink.lock().unwrap().push(pr))
            .await
            .expect("wait");

        assert_eq!(images.len(), 1);
        assert_eq!(&images[0].bytes[..8], &PNG_MAGIC, "PNG として取得できる");
        let decoded = image::load_from_memory(&images[0].bytes).expect("decodable");
        assert_eq!((decoded.width(), decoded.height()), (1024, 1024));
        assert!(
            images[0].bytes.len() > 100_000,
            "画像として妥当なサイズ(真っ黒・空ではない)"
        );
        let seen = seen.lock().unwrap();
        println!("progress events: {}, last: {:?}", seen.len(), seen.last());
        assert!(!seen.is_empty(), "WebSocket から進捗を受信できる");
        assert_eq!(seen.last().unwrap().total, 25);
        if let Ok(path) = std::env::var("COMFYUI_SAVE_PNG") {
            std::fs::write(&path, &images[0].bytes).expect("save png");
            println!("saved {path}");
        }
    }

    /// 実機: Qwen-Image 2.1 の参照画像つき生成(docs/24)が、参照 2 枚で完了すること。出力サイズが 1 枚目の
    /// 縦横比に従い、32 の倍数であることも確かめる。時間を抑えるため 8 ステップ・`resolution` 768 で行う。
    /// `COMFYUI_SAVE_PNG` に出力先を指定すると、生成した PNG を保存する。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with the Qwen-Image 2.1 files (docs/17 §2, docs/24 §9)"]
    async fn real_comfyui_qwen_image_21_reference_edit_with_two_images() {
        use crate::domain::params::ReferenceParams;
        use image::{ImageFormat, Rgba, RgbaImage};

        let png = |w: u32, h: u32, tint: u8| {
            let img = RgbaImage::from_fn(w, h, |x, y| {
                let (dx, dy) = (x as i32 - (w / 2) as i32, y as i32 - (h / 3) as i32);
                let face = dx * dx + dy * dy < 90 * 90;
                let v = (y * 255 / h) as u8;
                if face {
                    Rgba([240, 200, 170, 255])
                } else {
                    Rgba([v, tint, 255 - v, 255])
                }
            });
            let mut out = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
                .unwrap();
            out
        };

        let c = ComfyUiClient::new(&real_url()).unwrap();
        let first = c.upload_input(png(512, 768, 90)).await.expect("upload 1");
        let second = c.upload_input(png(512, 512, 200)).await.expect("upload 2");

        let mut p = sample(ModelKind::QwenImage);
        p.model.name = "qwen_image_2.1_int8_convrot.safetensors".into();
        p.model.clip = vec!["qwen3vl_8b_int8_convrot.safetensors".into()];
        p.model.vae = Some("qwen_image_2.1_vae_bf16.safetensors".into());
        p.prompt =
            "Redraw <image1> as an anime-style illustration, and use the colors of <image2> \
                    for the background."
                .into();
        p.negative_prompt = String::new();
        p.width = 512;
        p.height = 768;
        p.steps = 8;
        p.cfg = 1.0;
        p.sampler = "euler".into();
        p.scheduler = "simple".into();
        p.seed = 20_261_120;
        p.reference = Some(ReferenceParams {
            enabled: true,
            images: vec!["inputs/a.png".into(), "inputs/b.png".into()],
            resolution: 768,
        });
        assert_eq!(p.upload_count(), 2);

        let id = c.submit(&p, &[first, second]).await.expect("submit");
        let images = c.wait(&id, &|_| {}).await.expect("wait");

        assert_eq!(images.len(), 1);
        assert_eq!(&images[0].bytes[..8], &PNG_MAGIC, "PNG として取得できる");
        let decoded = image::load_from_memory(&images[0].bytes).expect("decodable");
        let (w, h) = (decoded.width(), decoded.height());
        println!("reference edit output: {w}x{h}");
        assert_eq!((w % 32, h % 32), (0, 0), "32 の倍数");
        assert!(h > w, "1 枚目(縦長)の縦横比に従う");
        assert!(
            (f64::from(w) / f64::from(h) - 512.0 / 768.0).abs() < 0.06,
            "1 枚目の縦横比に近い"
        );
        if let Ok(path) = std::env::var("COMFYUI_SAVE_PNG") {
            std::fs::write(&path, &images[0].bytes).expect("save png");
            println!("saved {path}");
        }
    }

    /// 実機: 顔の入れ替え(docs/42)。参照 2 枚 + 顔入れ替え LoRA + 後段合成で画像が生成され、出力が元の対象画像と同じ大きさになること。
    /// 環境変数 `FACE_SWAP_TARGET`(対象)と `FACE_SWAP_FACE`(顔)に PNG のパスを指定する(実在しない人物の画像を使うこと)。
    /// `COMFYUI_SAVE_PNG` に出力先を指定すると、生成した PNG を保存する。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with Qwen-Image 2.1, the BFS head-swap LoRA and the face detector (docs/42 §9)"]
    async fn real_comfyui_face_swap_with_lora_and_composite() {
        real_face_swap_run(&real_url()).await;
    }

    /// 実機: アプリと同じ起動経路(`EngineProcess`。出力のパイプ・Job Object・コンソールなし)で ComfyUI を起動し、
    /// 顔の入れ替えを実行する。アプリ経由の起動でだけ ComfyUI が応答しなくなる件の再現用(docs/46 §13)。
    /// `COMFYUI_PYTHON`(venv の python.exe)・`COMFYUI_MAIN`(main.py)・`COMFYUI_BASE_DIR` を指定する。
    /// 8188 番ポートの ComfyUI は事前に止めておくこと。
    #[tokio::test]
    #[ignore = "starts a real ComfyUI through the app launcher; needs COMFYUI_PYTHON / COMFYUI_MAIN / COMFYUI_BASE_DIR"]
    async fn real_comfyui_face_swap_through_the_app_launcher() {
        use crate::services::engine_process::config::LaunchSpec;
        use crate::services::engine_process::{EngineProcess, ProcSink, ProcStatus};
        struct CountSink(std::sync::atomic::AtomicUsize);
        impl ProcSink for CountSink {
            fn state(&self, _: &crate::services::engine_process::EngineProcState) {}
            fn log(&self, _: &str) {
                self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} を指定してください"));
        let (main, base) = (env("COMFYUI_MAIN"), env("COMFYUI_BASE_DIR"));
        let main_path = std::path::PathBuf::from(&main);
        let spec = LaunchSpec {
            program: env("COMFYUI_PYTHON").into(),
            args: vec![
                main.clone().into(),
                "--listen".into(),
                "127.0.0.1".into(),
                "--port".into(),
                "8188".into(),
                "--base-directory".into(),
                base.into(),
                "--disable-auto-launch".into(),
            ],
            cwd: main_path.parent().map(std::path::Path::to_path_buf),
        };
        let sink = Arc::new(CountSink(Default::default()));
        let proc = EngineProcess::new(sink.clone(), None);
        proc.start_spec(spec, 8188).await.expect("start");
        for _ in 0..240 {
            if proc.state().status == ProcStatus::Running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert_eq!(proc.state().status, ProcStatus::Running, "起動できた");

        // アプリと同じヘルスチェック(`health()`。5 秒ごと)を生成と並行して回す。`NO_HEALTH=1` で無効にする
        let health = (std::env::var("NO_HEALTH").as_deref() != Ok("1")).then(|| {
            tokio::spawn(async {
                let c = ComfyUiClient::new("http://127.0.0.1:8188").unwrap();
                let started = Instant::now();
                loop {
                    let t = Instant::now();
                    let r = c.health().await;
                    println!(
                        "[{:>4}s] health {} {}ms",
                        started.elapsed().as_secs(),
                        if r.is_ok() { "ok" } else { "FAILED" },
                        t.elapsed().as_millis()
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            })
        });

        real_face_swap_run("http://127.0.0.1:8188").await;
        if let Some(h) = health {
            h.abort();
        }
        println!(
            "engine log lines received: {}",
            sink.0.load(std::sync::atomic::Ordering::Relaxed)
        );
        proc.stop().await.expect("stop");
    }

    async fn real_face_swap_run(url: &str) {
        use crate::domain::params::{FACE_SWAP_DETECTOR, FaceSwapParams, FaceSwapRegion};

        let read = |var: &str| {
            let path = std::env::var(var).unwrap_or_else(|_| panic!("{var} を指定してください"));
            std::fs::read(path).expect("read png")
        };
        let (target_png, face_png) = (read("FACE_SWAP_TARGET"), read("FACE_SWAP_FACE"));
        let original = image::load_from_memory(&target_png).expect("decodable target");

        let c = ComfyUiClient::new(url).unwrap();
        let target = c.upload_input(target_png).await.expect("upload target");
        let face = c.upload_input(face_png).await.expect("upload face");

        let mut p = sample(ModelKind::QwenImage);
        p.model.name = "qwen_image_2.1_int8_convrot.safetensors".into();
        p.model.clip = vec!["qwen3vl_8b_int8_convrot.safetensors".into()];
        p.model.vae = Some("qwen_image_2.1_vae_bf16.safetensors".into());
        p.prompt = "head_swap: start with <image1> as the base image, keeping its lighting, environment, and \
                    background. remove the head from <image1> completely and replace it with the head from \
                    <image2>, strictly preserving the hair, eye color, nose structure from <image2>. copy the \
                    direction of the eye, head rotation, micro expressions from <image1>, high quality, sharp \
                    details, 4k"
            .into();
        p.negative_prompt = String::new();
        p.steps = 8;
        p.cfg = 1.0;
        p.sampler = "euler".into();
        p.scheduler = "simple".into();
        p.seed = 20_261_123;
        p.face_swap = Some(FaceSwapParams {
            enabled: true,
            target: "inputs/t.png".into(),
            face: "inputs/f.png".into(),
            // `FACE_SWAP_KEEP_OUTSIDE=0` で後段合成なし(切り分け用)
            keep_outside: std::env::var("FACE_SWAP_KEEP_OUTSIDE").as_deref() != Ok("0"),
            region: FaceSwapRegion::Head,
            resolution: 1024,
            detector: FACE_SWAP_DETECTOR.into(),
        });
        assert_eq!(p.upload_count(), 2);

        let submitted = c.submit(&p, &[target, face]).await;
        // `FACE_SWAP_EXPECT_NO_FACE=1`: 顔のない対象画像は、生成を始める前に `FACE_NOT_DETECTED` で失敗すること
        // (事前確認。docs/46 §6 E)。`keep_outside` を切ると事前確認は無いので、生成のあとの判定は無い
        if std::env::var("FACE_SWAP_EXPECT_NO_FACE").as_deref() == Ok("1") {
            assert_eq!(submitted.expect_err("no face").code(), "FACE_NOT_DETECTED");
            return;
        }
        let id = submitted.expect("submit");
        let images = c.wait(&id, &|_| {}).await.expect("wait");

        assert_eq!(images.len(), 1);
        assert_eq!(&images[0].bytes[..8], &PNG_MAGIC, "PNG として取得できる");
        let decoded = image::load_from_memory(&images[0].bytes).expect("decodable");
        println!("face swap output: {}x{}", decoded.width(), decoded.height());
        assert_eq!(
            (decoded.width(), decoded.height()),
            (original.width(), original.height()),
            "合成ありは元の対象画像と同じ大きさ"
        );
        if let Ok(path) = std::env::var("COMFYUI_SAVE_PNG") {
            std::fs::write(&path, &images[0].bytes).expect("save png");
            println!("saved {path}");
        }
    }

    /// 実機: Krea 2 Turbo(docs/17)で画像が生成されること。アプリの既定値(1024×1024・8 ステップ・CFG 1・euler / simple)と、
    /// FP8 scaled 版(推奨)のファイルを使う。`COMFYUI_SAVE_PNG` に出力先を指定すると、生成した PNG を保存する。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with the Krea 2 files (docs/17 §2)"]
    async fn real_comfyui_generates_krea2() {
        let c = ComfyUiClient::new(&real_url()).unwrap();
        let mut p = sample(ModelKind::Krea2);
        p.model.name = "krea2_turbo_fp8_scaled.safetensors".into();
        p.model.clip = vec!["qwen3vl_4b_fp8_scaled.safetensors".into()];
        p.model.vae = Some("qwen_image_vae.safetensors".into());
        p.prompt = "A small wooden cafe on a rainy street at dusk, warm light in the windows, \
                    a hand-painted sign that reads \"OPEN\", reflections on the wet pavement, cinematic"
            .into();
        p.negative_prompt = String::new();
        p.width = 1024;
        p.height = 1024;
        p.steps = 8;
        p.cfg = 1.0;
        p.sampler = "euler".into();
        p.scheduler = "simple".into();
        p.seed = 20_261_119;

        let id = c.submit(&p, &[]).await.expect("submit");
        let seen: Arc<Mutex<Vec<Progress>>> = Arc::default();
        let sink = Arc::clone(&seen);
        let images = c
            .wait(&id, &move |pr| sink.lock().unwrap().push(pr))
            .await
            .expect("wait");

        assert_eq!(images.len(), 1);
        assert_eq!(&images[0].bytes[..8], &PNG_MAGIC, "PNG として取得できる");
        let decoded = image::load_from_memory(&images[0].bytes).expect("decodable");
        assert_eq!((decoded.width(), decoded.height()), (1024, 1024));
        assert!(
            images[0].bytes.len() > 100_000,
            "画像として妥当なサイズ(真っ黒・空ではない)"
        );
        let seen = seen.lock().unwrap();
        println!("progress events: {}, last: {:?}", seen.len(), seen.last());
        assert!(!seen.is_empty(), "WebSocket から進捗を受信できる");
        assert_eq!(seen.last().unwrap().total, 8);
        if let Ok(path) = std::env::var("COMFYUI_SAVE_PNG") {
            std::fs::write(&path, &images[0].bytes).expect("save png");
            println!("saved {path}");
        }
    }

    /// 実機: 画像から生成(img2img。docs/19)が完了し、構図の固定(ControlNet Canny)と併用できること。
    /// `COMFYUI_IMG2IMG_SRC`(元画像の PNG)、`COMFYUI_SDXL_MODEL`(既定 sd_xl_base_1.0.safetensors)、
    /// `COMFYUI_CONTROLNET_SDXL`(既定 controlnet-canny-sdxl-1.0_fp16.safetensors)を使う。
    /// `COMFYUI_SAVE_PNG` に出力先を指定すると、生成した PNG を保存する。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with an SDXL checkpoint, an SDXL Canny ControlNet and COMFYUI_IMG2IMG_SRC"]
    async fn real_comfyui_img2img_with_composition_lock() {
        use crate::domain::params::{
            ControlnetParams, ControlnetUnit, Img2imgParams, Preprocessor,
        };
        let c = ComfyUiClient::new(&real_url()).unwrap();
        let src_path = std::env::var("COMFYUI_IMG2IMG_SRC").expect("COMFYUI_IMG2IMG_SRC");
        let src_bytes = std::fs::read(&src_path).expect("read source");
        let source = image::load_from_memory(&src_bytes).expect("decode source");
        let (w, h) = (1024u32, 576u32);
        let name = c
            .upload_input(src_bytes.clone())
            .await
            .expect("upload (control)");
        let name2 = c.upload_input(src_bytes).await.expect("upload (source)");

        let mut p = sample(ModelKind::Sdxl);
        p.model.name = std::env::var("COMFYUI_SDXL_MODEL")
            .unwrap_or_else(|_| "sd_xl_base_1.0.safetensors".into());
        p.prompt = "anime style, masterpiece, a small cafe on a rainy street at dusk, warm lights, cel shading, flat colors"
            .into();
        p.negative_prompt = "photo, realistic, blurry, low quality".into();
        p.width = w;
        p.height = h;
        p.steps = 25;
        p.cfg = 6.0;
        p.sampler = "euler".into();
        p.scheduler = "normal".into();
        p.seed = 20_261_119;
        p.img2img = Some(Img2imgParams {
            enabled: true,
            image: "inputs/src.png".into(),
            denoise: 0.65,
        });
        p.controlnet = Some(ControlnetParams {
            enabled: true,
            units: vec![ControlnetUnit {
                image: "inputs/src.png".into(),
                model: std::env::var("COMFYUI_CONTROLNET_SDXL")
                    .unwrap_or_else(|_| "controlnet-canny-sdxl-1.0_fp16.safetensors".into()),
                preprocessor: Preprocessor::Canny,
                strength: 0.8,
                start_percent: 0.0,
                end_percent: 0.8,
            }],
        });
        // 画像の並び: ControlNet のユニット、img2img の元画像
        let id = c.submit(&p, &[name, name2]).await.expect("submit");
        let images = c.wait(&id, &|_| {}).await.expect("wait");

        assert_eq!(images.len(), 1);
        assert_eq!(&images[0].bytes[..8], &PNG_MAGIC, "PNG として取得できる");
        let got = image::load_from_memory(&images[0].bytes).expect("decodable");
        assert_eq!((got.width(), got.height()), (w, h));
        // 元画像そのままではない(描き直されている)
        let resized = source
            .resize_exact(w, h, image::imageops::FilterType::Triangle)
            .to_rgb8();
        let diff: u64 = got
            .to_rgb8()
            .pixels()
            .zip(resized.pixels())
            .filter(|(a, b)| a != b)
            .count() as u64;
        assert!(
            diff > u64::from(w * h) / 2,
            "元画像から描き直されている: {diff}"
        );
        if let Ok(path) = std::env::var("COMFYUI_SAVE_PNG") {
            std::fs::write(&path, &images[0].bytes).expect("save png");
            println!("saved {path}");
        }
    }

    /// 実機: 同じパラメータ・同じシードなら同じ画像になること(ギャラリーの「この設定で生成」の前提)。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with an SD 1.5 checkpoint"]
    async fn real_comfyui_same_seed_reproduces_same_pixels() {
        let c = ComfyUiClient::new(&real_url()).unwrap();
        let mut pixels = Vec::new();
        for _ in 0..2 {
            let id = c
                .submit(&real_sd15_params(6, 777), &[])
                .await
                .expect("submit");
            let images = c.wait(&id, &|_| {}).await.expect("wait");
            pixels.push(
                image::load_from_memory(&images[0].bytes)
                    .unwrap()
                    .to_rgb8()
                    .into_raw(),
            );
        }
        assert_eq!(pixels[0], pixels[1], "同じシードで画素が一致しない");

        // 別のシードなら別の画像になる(シードが実際に効いている)
        let id = c
            .submit(&real_sd15_params(6, 778), &[])
            .await
            .expect("submit");
        let other = c.wait(&id, &|_| {}).await.expect("wait");
        let other = image::load_from_memory(&other[0].bytes)
            .unwrap()
            .to_rgb8()
            .into_raw();
        assert_ne!(pixels[0], other, "シードを変えても同じ画像になっている");
    }

    /// 実機: 顔補正(FaceDetailer)が動き、進捗が「顔補正」の段階で通知されること。
    /// 顔が写らない絵では補正をスキップして正常終了し、バッチ(2 枚)でも全枚が返ること。
    /// Impact Pack / Subpack と `bbox/face_yolov8m.pt` が必要(docs/08 §6.1)。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with Impact Pack, a detector model and an SD 1.5 checkpoint"]
    async fn real_comfyui_face_detailer_runs_and_skips_when_no_face() {
        use crate::domain::params::FaceDetailerParams;
        let c = ComfyUiClient::new(&real_url()).unwrap();
        let face = |p: &mut GenerationParams| {
            p.face_detailer = Some(FaceDetailerParams {
                enabled: true,
                detector: "bbox/face_yolov8m.pt".into(),
                denoise: 0.4,
                prompt: String::new(),
                confidence: 0.3,
            });
        };

        // 顔あり: 補正の段階の進捗が来る
        let mut p = real_sd15_params(20, 3);
        p.prompt = "portrait photo of a woman, detailed face, looking at camera".into();
        p.width = 512;
        p.height = 512;
        face(&mut p);
        let id = c.submit(&p, &[]).await.expect("submit");
        let seen: Arc<Mutex<Vec<Progress>>> = Arc::default();
        let sink = Arc::clone(&seen);
        let images = c
            .wait(&id, &move |p| sink.lock().unwrap().push(p))
            .await
            .expect("wait");
        assert_eq!(images.len(), 1);
        assert_eq!(&images[0].bytes[..8], &PNG_MAGIC);
        let phases: Vec<_> = seen
            .lock()
            .unwrap()
            .iter()
            .filter_map(|p| p.phase)
            .collect();
        println!("phases: {phases:?}");
        assert!(phases.contains(&ProgressPhase::Generate));
        assert!(
            phases.contains(&ProgressPhase::FaceDetail),
            "顔補正の進捗が通知されない"
        );

        // 顔なし(リンゴ): 検出ゼロでもエラーにならず、画像が返る。バッチ 2 枚
        let mut p = real_sd15_params(8, 5);
        p.batch_size = 2;
        face(&mut p);
        let id = c.submit(&p, &[]).await.expect("submit");
        let images = c.wait(&id, &|_| {}).await.expect("wait");
        assert_eq!(images.len(), 2, "バッチの全枚が返る");
    }

    /// 参照画像用の合成画像(黒地に白い円と四角。輪郭がはっきりしていて Canny で抽出できる)。
    fn reference_png() -> Vec<u8> {
        use image::{ImageFormat, Rgba, RgbaImage};
        let img = RgbaImage::from_fn(512, 512, |x, y| {
            let (dx, dy) = (x as i32 - 180, y as i32 - 200);
            let circle = dx * dx + dy * dy < 100 * 100;
            let square = (300..460).contains(&x) && (280..440).contains(&y);
            let v = if circle || square { 255 } else { 0 };
            Rgba([v, v, v, 255])
        });
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    fn cn_unit(
        model: &str,
        pre: crate::domain::params::Preprocessor,
    ) -> crate::domain::params::ControlnetParams {
        use crate::domain::params::{ControlnetParams, ControlnetUnit};
        ControlnetParams {
            enabled: true,
            units: vec![ControlnetUnit {
                image: "inputs/ref.png".into(),
                model: model.into(),
                preprocessor: pre,
                strength: 1.0,
                start_percent: 0.0,
                end_percent: 1.0,
            }],
        }
    }

    /// 実機: 参照画像のアップロード → ControlNet(Canny)付きの生成が完了すること。
    /// 同じシードでも、ControlNet の有無で画像が変わる(条件が効いている)こと。
    /// `COMFYUI_CONTROLNET_MODEL`(既定 control_v11p_sd15_canny_fp16.safetensors)が必要(docs/08 §6.1)。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with an SD 1.5 checkpoint and a canny ControlNet model"]
    async fn real_comfyui_controlnet_canny_generates_with_uploaded_reference() {
        use crate::domain::params::Preprocessor;
        let c = ComfyUiClient::new(&real_url()).unwrap();
        let model = std::env::var("COMFYUI_CONTROLNET_MODEL")
            .unwrap_or_else(|_| "control_v11p_sd15_canny_fp16.safetensors".into());

        let name = c.upload_input(reference_png()).await.expect("upload");
        println!("uploaded as {name}");

        let mut p = real_sd15_params(12, 11);
        p.prompt = "a still life with a glowing orb and a wooden box, studio photo".into();
        p.controlnet = Some(cn_unit(&model, Preprocessor::Canny));
        let id = c
            .submit(&p, std::slice::from_ref(&name))
            .await
            .expect("submit");
        let with_cn = c.wait(&id, &|_| {}).await.expect("wait");
        assert_eq!(with_cn.len(), 1);
        assert_eq!(&with_cn[0].bytes[..8], &PNG_MAGIC);

        let mut plain = p.clone();
        plain.controlnet = None;
        let id = c.submit(&plain, &[]).await.expect("submit");
        let without = c.wait(&id, &|_| {}).await.expect("wait");
        let px = |d: &ImageData| {
            image::load_from_memory(&d.bytes)
                .unwrap()
                .to_rgb8()
                .into_raw()
        };
        assert_ne!(
            px(&with_cn[0]),
            px(&without[0]),
            "ControlNet が効いていない"
        );
    }

    /// 実機: Inpaint が完了し、合成ありならマスク外が元画像と一致、マスク内は変わること。
    /// SDXL のみ導入されている環境向けに `COMFYUI_SD15_MODEL` と `COMFYUI_INPAINT_SIZE` を使える。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with a checkpoint (COMFYUI_SD15_MODEL)"]
    async fn real_comfyui_inpaint_keeps_unmasked_pixels_identical() {
        use crate::domain::params::InpaintParams;
        use image::{GrayImage, ImageFormat, Luma};
        let c = ComfyUiClient::new(&real_url()).unwrap();
        let size: u32 = std::env::var("COMFYUI_INPAINT_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(512);

        // 参照画像(size に拡大)と、左半分を描き直すマスク
        let reference = image::load_from_memory(&reference_png())
            .unwrap()
            .resize_exact(size, size, image::imageops::FilterType::Nearest);
        let mut ref_png = Vec::new();
        reference
            .write_to(&mut std::io::Cursor::new(&mut ref_png), ImageFormat::Png)
            .unwrap();
        let mask = GrayImage::from_fn(size, size, |x, _| {
            Luma([if x < size / 2 { 255 } else { 0 }])
        });
        let mut mask_png = Vec::new();
        mask.write_to(&mut std::io::Cursor::new(&mut mask_png), ImageFormat::Png)
            .unwrap();
        let image_name = c.upload_input(ref_png).await.expect("upload image");
        let mask_name = c.upload_input(mask_png).await.expect("upload mask");

        let mut p = real_sd15_params(12, 5);
        p.width = size;
        p.height = size;
        p.prompt = "a wooden table with a bowl of fruit, studio photo".into();
        p.inpaint = Some(InpaintParams {
            enabled: true,
            image: "inputs/ref.png".into(),
            mask: "inputs/mask.png".into(),
            denoise: 0.8,
            grow_mask: 0,
            feather: 0,
            composite_back: true,
        });
        let id = c
            .submit(&p, &[image_name, mask_name])
            .await
            .expect("submit");
        let out = c.wait(&id, &|_| {}).await.expect("wait");
        assert_eq!(out.len(), 1);
        let got = image::load_from_memory(&out[0].bytes).unwrap().to_rgb8();
        let orig = reference.to_rgb8();
        assert_eq!(got.dimensions(), orig.dimensions());
        // 右半分(マスク外。境界から離れた範囲)は元画像と完全一致
        let mut diff_left = 0u64;
        for y in 0..size {
            for x in 0..size {
                let same = got.get_pixel(x, y) == orig.get_pixel(x, y);
                if x >= size / 2 + 2 {
                    assert!(same, "マスク外の画素が変わった: ({x}, {y})");
                } else if x < size / 2 - 2 && !same {
                    diff_left += 1;
                }
            }
        }
        assert!(
            diff_left > 1000,
            "マスク内が描き直されていない: {diff_left}"
        );
    }

    /// 実機: Depth / OpenPose の前処理を含むワークフローが、バリデータに構造的に受理されること。
    /// ControlNet モデル名だけ存在しないものにし、検出されるエラーがそれだけなら、
    /// ほかのノード種別・必須入力・リンクの型は正しい(前処理の重みは実行時に取得されるため、実行はしない)。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with controlnet_aux and an SD 1.5 checkpoint"]
    async fn real_comfyui_controlnet_preprocessor_workflows_are_structurally_valid() {
        use crate::domain::params::Preprocessor;
        let c = ComfyUiClient::new(&real_url()).unwrap();
        let name = c.upload_input(reference_png()).await.expect("upload");

        for pre in [
            Preprocessor::Canny,
            Preprocessor::Depth,
            Preprocessor::Openpose,
            Preprocessor::None,
        ] {
            let mut p = real_sd15_params(8, 1);
            p.controlnet = Some(cn_unit("definitely-not-installed.safetensors", pre));
            let body = json!({
                "prompt": workflow::build(&p, "gazooo", std::slice::from_ref(&name)),
                "client_id": c.client_id,
            });
            let v: Value = c
                .http
                .post(format!("{}/prompt", c.base))
                .json(&body)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let errors: Vec<String> = v["node_errors"]
                .as_object()
                .expect("node_errors")
                .iter()
                .flat_map(|(node, ne)| {
                    ne["errors"].as_array().unwrap().iter().map(move |e| {
                        format!(
                            "{node}/{}: {} {}",
                            ne["class_type"], e["type"], e["details"]
                        )
                    })
                })
                .collect();
            println!("{pre:?}: {errors:?}");
            assert_eq!(errors.len(), 1, "{pre:?}: {errors:?}");
            assert!(
                errors[0].starts_with("33/\"ControlNetLoader\": \"value_not_in_list\""),
                "{pre:?}: {errors:?}"
            );
        }
    }
}
