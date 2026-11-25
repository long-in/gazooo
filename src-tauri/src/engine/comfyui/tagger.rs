//! 画像のタグ付け(ComfyUI-WD14-Tagger。docs/39)。
//!
//! `LoadImage` → `WD14Tagger|pysssss` だけのワークフローを投入し、履歴の出力(`tags`)から
//! タグ列を取り出す。ギャラリーには保存せず、ジョブ(`jobs`)としても扱わない。

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{ComfyUiClient, NODE_WD14, parse};
use crate::engine::TagRequest;
use crate::error::{AppError, AppResult};

/// ワークフロー内のノード ID。
const LOAD_NODE: &str = "1";
const TAG_NODE: &str = "2";

/// 解析の待機の上限。初回はノードが解析モデル(数百 MB)を取得するため長めにとる。
const MAX_WAIT: Duration = Duration::from_secs(15 * 60);

/// タグ付けのワークフロー(docs/39 §5)。`SaveImage` は使わない。
/// `replace_underscore` は true、`trailing_comma` は false に固定する(§4.2)。
pub(super) fn build_workflow(image_name: &str, req: &TagRequest) -> Value {
    json!({
        LOAD_NODE: {
            "class_type": "LoadImage",
            "inputs": { "image": image_name },
        },
        TAG_NODE: {
            "class_type": NODE_WD14,
            "inputs": {
                "image": [LOAD_NODE, 0],
                "model": req.model,
                "threshold": req.threshold,
                "character_threshold": req.character_threshold,
                "replace_underscore": true,
                "trailing_comma": false,
                "exclude_tags": req.exclude_tags,
            },
        },
    })
}

/// 履歴の状態(タグ版)。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum TagHistory {
    Pending,
    Done(String),
    Failed(String),
}

/// `/history/<id>` の応答から、タグ列を取り出す。
/// 出力は `outputs["2"].tags`(文字列の配列。1 枚なので先頭)。形が違えば `Err`(`ENGINE_BAD_RESPONSE`)。
pub(super) fn parse_tag_history(v: &Value, prompt_id: &str) -> AppResult<TagHistory> {
    let Some(entry) = v.get(prompt_id) else {
        return Ok(TagHistory::Pending);
    };
    let status = entry.get("status");
    if status
        .and_then(|s| s.get("status_str"))
        .and_then(Value::as_str)
        == Some("error")
    {
        // 失敗の文言は画像生成と同じ解釈を使う
        return Ok(match parse::parse_history(v, prompt_id) {
            parse::HistoryState::Failed(m) => TagHistory::Failed(m),
            _ => TagHistory::Failed("execution failed".into()),
        });
    }
    if status
        .and_then(|s| s.get("completed"))
        .and_then(Value::as_bool)
        == Some(false)
    {
        return Ok(TagHistory::Pending);
    }
    let tags = entry
        .get("outputs")
        .and_then(|o| o.get(TAG_NODE))
        .and_then(|o| o.get("tags"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::EngineBadResponse("/history: tags がありません".into()))?;
    Ok(TagHistory::Done(tags.to_owned()))
}

/// レーティング系のタグを除き、区切りを整える(docs/39 §4.2・Q-296)。
/// ノードは通常レーティングを返さないが、返す版・設定に備えて除く。
pub(crate) fn clean_tags(raw: &str) -> String {
    const RATINGS: [&str; 4] = ["general", "sensitive", "questionable", "explicit"];
    raw.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .filter(|t| {
            let lower = t.to_ascii_lowercase();
            !lower.starts_with("rating:") && !RATINGS.contains(&lower.as_str())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

impl ComfyUiClient {
    /// 画像をアップロードしてタグ付けし、整えたタグ列を返す。
    pub(super) async fn run_tagger(&self, png: Vec<u8>, req: &TagRequest) -> AppResult<String> {
        let name = self.upload_image(png).await?;
        let id = self.post_prompt(build_workflow(&name, req)).await?;
        let started = Instant::now();
        let mut ticker = tokio::time::interval(self.poll_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut failures = 0u32;
        loop {
            ticker.tick().await;
            if started.elapsed() > MAX_WAIT {
                return Err(AppError::EngineExecutionFailed(
                    "解析が時間内に終わりませんでした".into(),
                ));
            }
            match self.get_json(&format!("/history/{id}")).await {
                Ok(v) => {
                    failures = 0;
                    match parse_tag_history(&v.unwrap_or(Value::Null), &id)? {
                        TagHistory::Pending => {}
                        TagHistory::Failed(m) => return Err(AppError::EngineExecutionFailed(m)),
                        TagHistory::Done(raw) => {
                            // タグの中身・画像のパスはログに出さない(docs/39 §8)
                            tracing::debug!(
                                remote = %id,
                                elapsed_ms = started.elapsed().as_millis() as u64,
                                "tagging finished"
                            );
                            return Ok(clean_tags(&raw));
                        }
                    }
                }
                Err(e) => {
                    failures += 1;
                    if failures >= 20 {
                        return Err(e);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::testutil::{Route, spawn};
    use super::*;
    use crate::engine::GenerationEngine;

    fn req() -> TagRequest {
        TagRequest {
            model: "wd-v1-4-moat-tagger-v2".into(),
            threshold: 0.35,
            character_threshold: 0.85,
            exclude_tags: "blurry".into(),
        }
    }

    fn client(url: &str) -> ComfyUiClient {
        ComfyUiClient::new(url)
            .unwrap()
            .with_poll_interval(Duration::from_millis(20))
    }

    #[test]
    fn workflow_has_only_load_and_tagger_without_save() {
        let w = build_workflow("gazooo-x.png", &req());
        assert_eq!(w["1"]["class_type"], "LoadImage");
        assert_eq!(w["1"]["inputs"]["image"], "gazooo-x.png");
        let t = &w["2"];
        assert_eq!(t["class_type"], "WD14Tagger|pysssss");
        assert_eq!(t["inputs"]["image"], json!(["1", 0]));
        assert_eq!(t["inputs"]["model"], "wd-v1-4-moat-tagger-v2");
        assert_eq!(t["inputs"]["threshold"], 0.35);
        assert_eq!(t["inputs"]["character_threshold"], 0.85);
        assert_eq!(t["inputs"]["exclude_tags"], "blurry");
        // 固定値(§4.2)
        assert_eq!(t["inputs"]["replace_underscore"], true);
        assert_eq!(t["inputs"]["trailing_comma"], false);
        assert!(!w.to_string().contains("SaveImage"));
        assert_eq!(w.as_object().unwrap().len(), 2);
    }

    #[test]
    fn history_parsing() {
        let pending = json!({});
        assert_eq!(
            parse_tag_history(&pending, "p").unwrap(),
            TagHistory::Pending
        );
        let running = json!({"p":{"status":{"status_str":"success","completed":false}}});
        assert_eq!(
            parse_tag_history(&running, "p").unwrap(),
            TagHistory::Pending
        );
        let done = json!({"p":{"status":{"status_str":"success","completed":true},
            "outputs":{"2":{"tags":["1girl, long hair"]}}}});
        assert_eq!(
            parse_tag_history(&done, "p").unwrap(),
            TagHistory::Done("1girl, long hair".into())
        );
        let failed = json!({"p":{"status":{"status_str":"error","messages":[
            ["execution_error",{"node_type":"WD14Tagger|pysssss","exception_message":"boom"}]]}}});
        assert_eq!(
            parse_tag_history(&failed, "p").unwrap(),
            TagHistory::Failed("WD14Tagger|pysssss: boom".into())
        );
        // 完了しているのに tags が無い → 応答形式が想定と異なる
        let no_tags =
            json!({"p":{"status":{"status_str":"success","completed":true},"outputs":{}}});
        assert_eq!(
            parse_tag_history(&no_tags, "p").unwrap_err().code(),
            "ENGINE_BAD_RESPONSE"
        );
    }

    #[test]
    fn clean_tags_drops_ratings_and_normalizes() {
        assert_eq!(
            clean_tags("rating:safe,  1girl , General, solo,, explicit"),
            "1girl, solo"
        );
        assert_eq!(clean_tags(""), "");
        // エスケープされた括弧など、タグの中身は変えない
        assert_eq!(
            clean_tags("hair ornament, \\(cosplay\\)"),
            "hair ornament, \\(cosplay\\)"
        );
    }

    #[tokio::test]
    async fn tag_image_uploads_submits_and_reads_tags() {
        let done = r#"{"t1":{"status":{"status_str":"success","completed":true},
            "outputs":{"2":{"tags":["1girl, school uniform, rating:safe"]}}}}"#;
        let m = spawn(vec![
            Route::post(
                "/upload/image",
                200,
                r#"{"name":"gazooo-t.png","subfolder":"","type":"input"}"#,
            ),
            Route::post("/prompt", 200, r#"{"prompt_id":"t1"}"#),
            Route::seq("GET /history/t1", vec![(200, "{}"), (200, done)]),
        ])
        .await;
        let tags = client(&m.url)
            .tag_image(b"PNG".to_vec(), &req())
            .await
            .unwrap();
        assert_eq!(tags, "1girl, school uniform");

        let reqs = m.requests.lock().unwrap();
        let body = reqs
            .iter()
            .find(|r| r.starts_with("POST /prompt "))
            .unwrap();
        assert!(body.contains("WD14Tagger|pysssss") && body.contains("gazooo-t.png"));
    }

    #[tokio::test]
    async fn tag_image_maps_missing_node_and_execution_errors() {
        // ノード未導入: /prompt が 400
        let missing = r#"{"error":{"type":"missing_node_type","message":"Cannot execute because node WD14Tagger|pysssss does not exist."},"node_errors":{}}"#;
        let m = spawn(vec![
            Route::post(
                "/upload/image",
                200,
                r#"{"name":"a.png","subfolder":"","type":"input"}"#,
            ),
            Route::post("/prompt", 400, missing),
        ])
        .await;
        let err = client(&m.url).tag_image(vec![1], &req()).await.unwrap_err();
        assert_eq!(err.code(), "ENGINE_NODE_MISSING");

        // 実行エラー
        let failed = r#"{"t1":{"status":{"status_str":"error","messages":[
            ["execution_error",{"node_type":"WD14Tagger|pysssss","exception_message":"no network"}]]}}}"#;
        let m = spawn(vec![
            Route::post(
                "/upload/image",
                200,
                r#"{"name":"a.png","subfolder":"","type":"input"}"#,
            ),
            Route::post("/prompt", 200, r#"{"prompt_id":"t1"}"#),
            Route::get("/history/t1", failed),
        ])
        .await;
        let err = client(&m.url).tag_image(vec![1], &req()).await.unwrap_err();
        assert_eq!(err.code(), "ENGINE_EXECUTION_FAILED");
        assert!(err.to_string().contains("no network"));
    }

    /// 実機: WD14 Tagger が導入された ComfyUI で、実際にタグが返ること(通常は実行しない)。
    /// `COMFYUI_TAG_SRC`(解析する画像の PNG)が必要。初回は解析モデルの取得で時間がかかる。
    #[tokio::test]
    #[ignore = "requires a running ComfyUI with ComfyUI-WD14-Tagger and COMFYUI_TAG_SRC"]
    async fn real_comfyui_tags_an_image() {
        let url = std::env::var("COMFYUI_URL").unwrap_or_else(|_| "http://127.0.0.1:8188".into());
        let c = ComfyUiClient::new(&url).unwrap();
        let src = std::env::var("COMFYUI_TAG_SRC").expect("COMFYUI_TAG_SRC");
        let png = std::fs::read(src).expect("read");
        let models = c.list_models().await.unwrap();
        println!("taggers: {:?}", models.taggers);
        let model = std::env::var("COMFYUI_TAGGER_MODEL")
            .unwrap_or_else(|_| "wd-v1-4-moat-tagger-v2".into());
        let tags = c
            .tag_image(png, &TagRequest { model, ..req() })
            .await
            .expect("tag");
        println!("tags: {tags}");
        assert!(!tags.is_empty());
    }
}
