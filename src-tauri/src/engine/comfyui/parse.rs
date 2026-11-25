//! ComfyUI の JSON 応答の解釈(純粋関数)。ネットワークに依存しないのでユニットテストしやすい。

use serde_json::Value;

use crate::error::AppError;

pub struct SystemStats {
    pub version: Option<String>,
    pub device: Option<String>,
    pub vram_free: Option<u64>,
}

/// `/system_stats` を解釈する。ComfyUI の応答でなければ `None`。
pub fn parse_system_stats(v: &Value) -> Option<SystemStats> {
    let system = v.get("system")?;
    let device0 = v.get("devices").and_then(|d| d.get(0));
    Some(SystemStats {
        version: system
            .get("comfyui_version")
            .and_then(Value::as_str)
            .map(str::to_owned),
        device: device0
            .and_then(|d| d.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        vram_free: device0
            .and_then(|d| d.get("vram_free"))
            .and_then(|n| n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.0) as u64))),
    })
}

/// 選択肢の仕様を文字列配列にする。2 つの形式に対応する。
/// - 旧: `[["a", "b"], {...}]`
/// - 新: `["COMBO", {"options": ["a", "b"]}]`
fn options_of(spec: &Value) -> Vec<String> {
    let Some(arr) = spec.as_array() else {
        return Vec::new();
    };
    let list = match arr.first() {
        Some(first) if first.is_array() => first.as_array(),
        Some(first) if first.as_str() == Some("COMBO") => arr
            .get(1)
            .and_then(|o| o.get("options"))
            .and_then(Value::as_array),
        _ => None,
    };
    list.map(|l| {
        l.iter()
            .filter_map(|x| x.as_str().map(str::to_owned))
            .collect()
    })
    .unwrap_or_default()
}

/// `/object_info/<node>` の応答から、ノードの入力 `field` の選択肢を取り出す。
/// ノードが存在しない(応答が空 `{}`)場合は空配列。
pub fn extract_options(node_info: &Value, node: &str, field: &str) -> Vec<String> {
    let Some(input) = node_info.get(node).and_then(|n| n.get("input")) else {
        return Vec::new();
    };
    ["required", "optional"]
        .iter()
        .find_map(|group| input.get(group).and_then(|g| g.get(field)))
        .map(options_of)
        .unwrap_or_default()
}

/// ノードが存在するか。
pub fn has_node(node_info: &Value, node: &str) -> bool {
    node_info.get(node).is_some()
}

/// `TextEncodeQwenImage21` が参照画像つき生成に使える定義か(docs/24 §4.2)。
/// 参照画像の入力 `images`(`COMFY_AUTOGROW_V3`)と任意の `vae` を持ち、出力に `latent` があること。
/// ComfyUI のバージョンではなくノードの定義で判定する(古い版にはこの入力が無い)。
pub fn has_reference_inputs(node_info: &Value) -> bool {
    let Some(node) = node_info.get("TextEncodeQwenImage21") else {
        return false;
    };
    let input = &node["input"];
    let images = input["required"]["images"]
        .get(0)
        .and_then(Value::as_str)
        .or_else(|| input["optional"]["images"].get(0).and_then(Value::as_str));
    let has_vae = input["optional"].get("vae").is_some() || input["required"].get("vae").is_some();
    let has_latent = node["output_name"]
        .as_array()
        .is_some_and(|o| o.iter().any(|n| n.as_str() == Some("latent")));
    images == Some("COMFY_AUTOGROW_V3") && has_vae && has_latent
}

/// `POST /upload/image` の応答から、`LoadImage` に渡す名前(サブフォルダがあれば `sub/name`)を取り出す。
pub fn parse_upload_name(v: &Value) -> Option<String> {
    let name = v.get("name")?.as_str().filter(|n| !n.is_empty())?;
    match v.get("subfolder").and_then(Value::as_str) {
        Some(sub) if !sub.is_empty() => Some(format!("{sub}/{name}")),
        _ => Some(name.to_owned()),
    }
}

/// 結果画像の参照(`/view` のクエリ)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    pub filename: String,
    pub subfolder: String,
    pub kind: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HistoryState {
    /// 履歴にまだ載っていない(待機中 / 実行中)。
    Pending,
    Done(Vec<ImageRef>),
    Failed(String),
}

/// `/history/<prompt_id>` の応答を解釈する。
pub fn parse_history(v: &Value, prompt_id: &str) -> HistoryState {
    let Some(entry) = v.get(prompt_id) else {
        return HistoryState::Pending;
    };
    let status = entry.get("status");
    let status_str = status
        .and_then(|s| s.get("status_str"))
        .and_then(Value::as_str);

    if status_str == Some("error") {
        return HistoryState::Failed(history_error_message(status));
    }
    let completed = status
        .and_then(|s| s.get("completed"))
        .and_then(Value::as_bool);
    if completed == Some(false) {
        return HistoryState::Pending;
    }

    let mut images = Vec::new();
    if let Some(outputs) = entry.get("outputs").and_then(Value::as_object) {
        for output in outputs.values() {
            for img in output
                .get("images")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let kind = img.get("type").and_then(Value::as_str).unwrap_or("output");
                if kind != "output" {
                    continue; // プレビュー用の temp は対象外
                }
                if let Some(filename) = img.get("filename").and_then(Value::as_str) {
                    images.push(ImageRef {
                        filename: filename.to_owned(),
                        subfolder: img
                            .get("subfolder")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        kind: kind.to_owned(),
                    });
                }
            }
        }
    }
    HistoryState::Done(images)
}

/// 履歴の、指定した出力ノード(`PreviewAny`)が出した数値を読む。無ければ `None`。
/// 顔の入れ替えで、検出した顔の数を取り出すのに使う(docs/46 §6 E)。
pub fn parse_count_output(v: &Value, prompt_id: &str, node: &str) -> Option<u64> {
    v.get(prompt_id)?
        .get("outputs")?
        .get(node)?
        .get("text")?
        .as_array()?
        .first()?
        .as_str()?
        .trim()
        .parse()
        .ok()
}

fn history_error_message(status: Option<&Value>) -> String {
    let messages = status
        .and_then(|s| s.get("messages"))
        .and_then(Value::as_array);
    for m in messages.into_iter().flatten() {
        let name = m.get(0).and_then(Value::as_str);
        let data = m.get(1);
        match name {
            Some("execution_error") => {
                let msg = data
                    .and_then(|d| d.get("exception_message"))
                    .and_then(Value::as_str)
                    .unwrap_or("execution error")
                    .trim();
                let node = data
                    .and_then(|d| d.get("node_type"))
                    .and_then(Value::as_str);
                return match node {
                    Some(n) => format!("{n}: {msg}"),
                    None => msg.to_owned(),
                };
            }
            Some("execution_interrupted") => return "interrupted".to_owned(),
            _ => {}
        }
    }
    "execution failed".to_owned()
}

/// モデルを選ぶ入力名。ここに `value_not_in_list` が出たら「モデルが無い」と判断する。
const MODEL_INPUTS: &[&str] = &[
    "ckpt_name",
    "unet_name",
    "clip_name",
    "clip_name1",
    "clip_name2",
    "vae_name",
];

/// `POST /prompt` の 400 応答を分類する(ENGINE_MODEL_MISSING / ENGINE_NODE_MISSING / ENGINE_EXECUTION_FAILED)。
pub fn parse_prompt_error(body: &Value) -> AppError {
    let error = body.get("error");
    let message = error
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("prompt rejected");

    if error.and_then(|e| e.get("type")).and_then(Value::as_str) == Some("missing_node_type") {
        return AppError::EngineNodeMissing(message.to_owned());
    }

    if let Some(node_errors) = body.get("node_errors").and_then(Value::as_object) {
        for ne in node_errors.values() {
            let class = ne.get("class_type").and_then(Value::as_str).unwrap_or("");
            for e in ne
                .get("errors")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let kind = e.get("type").and_then(Value::as_str);
                let details = e.get("details").and_then(Value::as_str).unwrap_or("");
                if kind == Some("missing_node_type") {
                    return AppError::EngineNodeMissing(class.to_owned());
                }
                if kind == Some("value_not_in_list") {
                    let input = details.split(':').next().unwrap_or("").trim();
                    // `CLIPLoader` の type に krea2 / qwen_image が無いのは、ComfyUI が古いため(docs/17 §4)。
                    // モデルの不在ではなく、ComfyUI の更新が必要なものとして扱う
                    if class == "CLIPLoader" && input == "type" {
                        return AppError::EngineNodeMissing(
                            "CLIPLoader の type(ComfyUI の更新が必要)".to_owned(),
                        );
                    }
                    if MODEL_INPUTS.contains(&input) {
                        return AppError::EngineModelMissing(details.to_owned());
                    }
                }
            }
        }
        // 最初のノードエラーの詳細を添える
        if let Some(first) = node_errors.values().next().and_then(|ne| {
            ne.get("errors")
                .and_then(|e| e.get(0))
                .and_then(|e| e.get("details").or_else(|| e.get("message")))
                .and_then(Value::as_str)
        }) {
            return AppError::EngineExecutionFailed(format!("{message}: {first}"));
        }
    }
    AppError::EngineExecutionFailed(message.to_owned())
}

/// WebSocket のテキストメッセージから `progress` を取り出す(対象プロンプト以外は無視)。
pub fn parse_ws_progress(text: &str, prompt_id: &str) -> Option<(u32, u32, Option<String>)> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v.get("type")?.as_str()? != "progress" {
        return None;
    }
    let data = v.get("data")?;
    if let Some(pid) = data.get("prompt_id").and_then(Value::as_str)
        && pid != prompt_id
    {
        return None;
    }
    let step = u32::try_from(data.get("value")?.as_u64()?).ok()?;
    let total = u32::try_from(data.get("max")?.as_u64()?).ok()?;
    let node = data.get("node").and_then(Value::as_str).map(str::to_owned);
    Some((step, total, node))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ComfyUI 0.45.6 の `/object_info/TextEncodeQwenImage21`(2026-11-20 に実機で確認した形)を縮めたもの。
    fn qwen_encode_info() -> Value {
        json!({ "TextEncodeQwenImage21": {
            "input": {
                "required": {
                    "clip": ["CLIP", {}],
                    "prompt": ["STRING", {}],
                    "negative_prompt": ["STRING", {}],
                    "resolution": ["INT", {}],
                    "images": ["COMFY_AUTOGROW_V3", { "template": { "names": ["image_1", "image_2"] } }]
                },
                "optional": { "vae": ["VAE", {}] }
            },
            "output_name": ["positive", "negative", "latent"]
        }})
    }

    #[test]
    fn reference_inputs_are_detected_from_the_node_definition() {
        assert!(has_reference_inputs(&qwen_encode_info()));
        // ノードが無い(未導入・Null)
        assert!(!has_reference_inputs(&Value::Null));
        assert!(!has_reference_inputs(&json!({})));
        // 古い定義: 参照画像の入力・vae・latent 出力が無い(t2i だけのエンコーダ)
        let old = json!({ "TextEncodeQwenImage21": {
            "input": { "required": { "clip": ["CLIP", {}], "prompt": ["STRING", {}] } },
            "output_name": ["positive", "negative"]
        }});
        assert!(!has_reference_inputs(&old));
        let mut no_vae = qwen_encode_info();
        no_vae["TextEncodeQwenImage21"]["input"]["optional"] = json!({});
        assert!(!has_reference_inputs(&no_vae));
        let mut no_latent = qwen_encode_info();
        no_latent["TextEncodeQwenImage21"]["output_name"] = json!(["positive", "negative"]);
        assert!(!has_reference_inputs(&no_latent));
    }
    use serde_json::json;

    #[test]
    fn system_stats_full() {
        let v = json!({
            "system": {"comfyui_version": "0.3.50", "os": "darwin"},
            "devices": [{"name": "mps", "type": "mps", "vram_free": 1234.0}]
        });
        let s = parse_system_stats(&v).unwrap();
        assert_eq!(s.version.as_deref(), Some("0.3.50"));
        assert_eq!(s.device.as_deref(), Some("mps"));
        assert_eq!(s.vram_free, Some(1234));
    }

    #[test]
    fn system_stats_without_devices() {
        let s = parse_system_stats(&json!({"system": {}})).unwrap();
        assert!(s.version.is_none() && s.device.is_none() && s.vram_free.is_none());
    }

    #[test]
    fn system_stats_rejects_foreign_response() {
        assert!(parse_system_stats(&json!({"hello": "world"})).is_none());
    }

    #[test]
    fn options_legacy_format() {
        let v = json!({"CheckpointLoaderSimple": {"input": {"required": {
            "ckpt_name": [["a.safetensors", "b.ckpt"], {"tooltip": "x"}]
        }}}});
        assert_eq!(
            extract_options(&v, "CheckpointLoaderSimple", "ckpt_name"),
            ["a.safetensors", "b.ckpt"]
        );
    }

    #[test]
    fn options_combo_format() {
        let v = json!({"KSampler": {"input": {"required": {
            "sampler_name": ["COMBO", {"options": ["euler", "dpmpp_2m"]}]
        }}}});
        assert_eq!(
            extract_options(&v, "KSampler", "sampler_name"),
            ["euler", "dpmpp_2m"]
        );
    }

    #[test]
    fn options_from_optional_group_and_missing() {
        let v = json!({"N": {"input": {"optional": {"f": [["x"]]}}}});
        assert_eq!(extract_options(&v, "N", "f"), ["x"]);
        assert!(extract_options(&v, "N", "nope").is_empty());
        assert!(extract_options(&json!({}), "N", "f").is_empty());
        // 数値型などの選択肢でないものは空
        let v = json!({"N": {"input": {"required": {"steps": ["INT", {"default": 20}]}}}});
        assert!(extract_options(&v, "N", "steps").is_empty());
    }

    #[test]
    fn node_presence() {
        assert!(has_node(&json!({"UnetLoaderGGUF": {}}), "UnetLoaderGGUF"));
        assert!(!has_node(&json!({}), "UnetLoaderGGUF"));
    }

    #[test]
    fn history_pending_when_absent_or_incomplete() {
        assert_eq!(parse_history(&json!({}), "p1"), HistoryState::Pending);
        let v = json!({"p1": {"status": {"status_str": "success", "completed": false}}});
        assert_eq!(parse_history(&v, "p1"), HistoryState::Pending);
    }

    #[test]
    fn history_done_collects_output_images_only() {
        let v = json!({"p1": {
            "status": {"status_str": "success", "completed": true},
            "outputs": {
                "8": {"images": [
                    {"filename": "gazooo_00001_.png", "subfolder": "", "type": "output"},
                    {"filename": "gazooo_00002_.png", "subfolder": "sub", "type": "output"}
                ]},
                "9": {"images": [{"filename": "tmp.png", "subfolder": "", "type": "temp"}]}
            }
        }});
        let HistoryState::Done(images) = parse_history(&v, "p1") else {
            panic!("expected Done");
        };
        assert_eq!(images.len(), 2);
        assert_eq!(images[1].subfolder, "sub");
        assert_eq!(images[0].filename, "gazooo_00001_.png");
    }

    #[test]
    fn count_output_is_read_from_the_preview_any_node() {
        let v = json!({"p1": {"status": {"status_str": "success", "completed": true},
            "outputs": {"122": {"text": ["0"]}, "9": {"images": []}}}});
        assert_eq!(parse_count_output(&v, "p1", "122"), Some(0));
        let one = json!({"p1": {"outputs": {"122": {"text": ["1"]}}}});
        assert_eq!(parse_count_output(&one, "p1", "122"), Some(1));
        // ノードが無い・数でない・別のジョブ
        assert_eq!(parse_count_output(&one, "p1", "5"), None);
        assert_eq!(parse_count_output(&one, "p2", "122"), None);
        let text = json!({"p1": {"outputs": {"122": {"text": ["abc"]}}}});
        assert_eq!(parse_count_output(&text, "p1", "122"), None);
    }

    #[test]
    fn history_failed_extracts_exception() {
        let v = json!({"p1": {"status": {"status_str": "error", "completed": false, "messages": [
            ["execution_start", {}],
            ["execution_error", {"node_type": "KSampler", "exception_message": "out of memory\n"}]
        ]}}});
        assert_eq!(
            parse_history(&v, "p1"),
            HistoryState::Failed("KSampler: out of memory".into())
        );
        let v = json!({"p1": {"status": {"status_str": "error", "messages": [["execution_interrupted", {}]]}}});
        assert_eq!(
            parse_history(&v, "p1"),
            HistoryState::Failed("interrupted".into())
        );
    }

    #[test]
    fn prompt_error_model_missing() {
        let body = json!({
            "error": {"type": "prompt_outputs_failed_validation", "message": "Prompt outputs failed validation"},
            "node_errors": {"1": {"class_type": "CheckpointLoaderSimple", "errors": [
                {"type": "value_not_in_list", "message": "Value not in list",
                 "details": "ckpt_name: 'x.safetensors' not in []"}
            ]}}
        });
        let e = parse_prompt_error(&body);
        assert_eq!(e.code(), "ENGINE_MODEL_MISSING");
    }

    #[test]
    fn prompt_error_sampler_not_in_list_is_execution_failure() {
        let body = json!({
            "error": {"type": "prompt_outputs_failed_validation", "message": "Prompt outputs failed validation"},
            "node_errors": {"5": {"class_type": "KSampler", "errors": [
                {"type": "value_not_in_list", "details": "sampler_name: 'zzz' not in ['euler']"}
            ]}}
        });
        let e = parse_prompt_error(&body);
        assert_eq!(e.code(), "ENGINE_EXECUTION_FAILED");
        assert!(e.to_string().contains("sampler_name"));
    }

    #[test]
    fn prompt_error_node_missing() {
        let top = json!({"error": {"type": "missing_node_type", "message": "Node 'UnetLoaderGGUF' not found"}, "node_errors": {}});
        assert_eq!(parse_prompt_error(&top).code(), "ENGINE_NODE_MISSING");
        let nested = json!({"error": {"message": "x"}, "node_errors": {"1": {"class_type": "UnetLoaderGGUF",
            "errors": [{"type": "missing_node_type", "details": ""}]}}});
        assert_eq!(parse_prompt_error(&nested).code(), "ENGINE_NODE_MISSING");
        assert_eq!(
            parse_prompt_error(&json!({})).code(),
            "ENGINE_EXECUTION_FAILED"
        );
    }

    #[test]
    fn missing_text_encoder_is_a_missing_model_and_an_old_clip_type_needs_a_comfyui_update() {
        // docs/17: Anima / Qwen-Image 2.1 / Krea 2 は CLIPLoader の clip_name(1 つ)と type を使う
        let missing_te = json!({"error": {"message": "x"}, "node_errors": {"2": {"class_type": "CLIPLoader",
            "errors": [{"type": "value_not_in_list", "details": "clip_name: 'te.safetensors' not in []"}]}}});
        assert_eq!(
            parse_prompt_error(&missing_te).code(),
            "ENGINE_MODEL_MISSING"
        );

        let old_comfyui = json!({"error": {"message": "x"}, "node_errors": {"2": {"class_type": "CLIPLoader",
            "errors": [{"type": "value_not_in_list", "details": "type: 'krea2' not in ['stable_diffusion', 'stable_cascade']"}]}}});
        assert_eq!(
            parse_prompt_error(&old_comfyui).code(),
            "ENGINE_NODE_MISSING"
        );

        // type 以外の CLIPLoader の入力や、別のノードの type は、これまでどおり
        let other = json!({"error": {"message": "x"}, "node_errors": {"5": {"class_type": "KSampler",
            "errors": [{"type": "value_not_in_list", "details": "type: 'x' not in []"}]}}});
        assert_ne!(parse_prompt_error(&other).code(), "ENGINE_NODE_MISSING");
    }

    #[test]
    fn upload_name_with_and_without_subfolder() {
        assert_eq!(
            parse_upload_name(&json!({"name": "a.png", "subfolder": "", "type": "input"})),
            Some("a.png".into())
        );
        assert_eq!(
            parse_upload_name(&json!({"name": "a.png", "subfolder": "gazooo"})),
            Some("gazooo/a.png".into())
        );
        assert_eq!(parse_upload_name(&json!({"name": ""})), None);
        assert_eq!(parse_upload_name(&json!({"error": "x"})), None);
    }

    #[test]
    fn ws_progress_filters_by_prompt() {
        let msg = r#"{"type":"progress","data":{"value":3,"max":20,"prompt_id":"p1","node":"8"}}"#;
        assert_eq!(
            parse_ws_progress(msg, "p1"),
            Some((3, 20, Some("8".into())))
        );
        assert_eq!(parse_ws_progress(msg, "other"), None);
        assert_eq!(
            parse_ws_progress(r#"{"type":"status","data":{}}"#, "p1"),
            None
        );
        assert_eq!(parse_ws_progress("not json", "p1"), None);
        // prompt_id が無い古い形式は受け入れる
        let old = r#"{"type":"progress","data":{"value":1,"max":4}}"#;
        assert_eq!(parse_ws_progress(old, "p1"), Some((1, 4, None)));
    }
}
