//! 画像からタグを作る(docs/39)。IPC `image_tag` の本体。
//!
//! 入力の検証 → 生成ジョブの実行中は断る(§4.3)→ 参照画像を読む → ノードの有無を確認 → エンジンで解析。
//! 結果は保存しない。ログにも画像のパス・タグの中身を出さない(§8)。

use serde::Serialize;

use crate::engine::{EngineModels, TagRequest};
use crate::error::{AppError, AppResult};
use crate::services::engine::EngineService;
use crate::services::inputs::InputService;
use crate::services::jobs::JobService;

/// 既定の解析モデル(候補にあればこれを選ぶ。docs/39 §4.2)。
pub const DEFAULT_MODEL: &str = "wd-v1-4-moat-tagger-v2";
pub const DEFAULT_THRESHOLD: f64 = 0.35;
pub const DEFAULT_CHARACTER_THRESHOLD: f64 = 0.85;
const MIN_THRESHOLD: f64 = 0.05;
const MAX_THRESHOLD: f64 = 1.0;
const MAX_EXCLUDE_LEN: usize = 1000;
const MAX_MODEL_LEN: usize = 128;

/// `image_tag` の入力(画像以外は任意)。
#[derive(Debug, Clone, Default)]
pub struct TagInput {
    pub model: Option<String>,
    pub threshold: Option<f64>,
    pub character_threshold: Option<f64>,
    pub exclude_tags: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImageTagResult {
    pub tags: String,
    pub model: String,
}

fn check_threshold(field: &str, v: f64) -> AppResult<f64> {
    if v.is_finite() && (MIN_THRESHOLD..=MAX_THRESHOLD).contains(&v) {
        Ok(v)
    } else {
        Err(AppError::validation(
            field,
            "しきい値は 0.05〜1.0 の範囲で指定してください",
        ))
    }
}

/// しきい値・除外タグの検証(通信の前に行える部分)。
fn check_values(input: &TagInput) -> AppResult<(f64, f64, String)> {
    let threshold = check_threshold("threshold", input.threshold.unwrap_or(DEFAULT_THRESHOLD))?;
    let character_threshold = check_threshold(
        "characterThreshold",
        input
            .character_threshold
            .unwrap_or(DEFAULT_CHARACTER_THRESHOLD),
    )?;
    let exclude_tags = input.exclude_tags.clone().unwrap_or_default();
    if exclude_tags.chars().count() > MAX_EXCLUDE_LEN {
        return Err(AppError::validation(
            "excludeTags",
            "除外するタグが長すぎます(1000 文字まで)",
        ));
    }
    Ok((threshold, character_threshold, exclude_tags))
}

/// 入力を検証し、解析モデルを決めて `TagRequest` にする。
/// `taggers` は導入済みのノードが返す解析モデルの候補(空ならノード未導入)。
pub fn resolve_request(input: &TagInput, taggers: &[String]) -> AppResult<TagRequest> {
    let (threshold, character_threshold, exclude_tags) = check_values(input)?;
    if taggers.is_empty() {
        return Err(AppError::EngineNodeMissing(
            crate::engine::comfyui::NODE_WD14.to_owned(),
        ));
    }
    let requested = input
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty());
    let model = match requested {
        Some(m) => {
            if m.chars().count() > MAX_MODEL_LEN || !taggers.iter().any(|t| t == m) {
                return Err(AppError::validation(
                    "model",
                    "選べない解析モデルです。一覧から選んでください",
                ));
            }
            m.to_owned()
        }
        None => taggers
            .iter()
            .find(|t| t.as_str() == DEFAULT_MODEL)
            .or_else(|| taggers.first())
            .cloned()
            .unwrap_or_default(),
    };
    Ok(TagRequest {
        model,
        threshold,
        character_threshold,
        exclude_tags,
    })
}

/// 生成ジョブが実行中・待機中なら断る(docs/39 §4.3)。
fn ensure_idle(running: usize, queued: usize) -> AppResult<()> {
    if running + queued > 0 {
        return Err(AppError::engine_proc(
            "ENGINE_BUSY",
            "生成が終わってからお試しください(生成中は解析できません)",
        ));
    }
    Ok(())
}

pub async fn tag_image(
    engine: &EngineService,
    inputs: &InputService,
    jobs: &JobService,
    image: &str,
    input: TagInput,
) -> AppResult<ImageTagResult> {
    // 範囲外の値は、通信する前に断る(モデルの候補は後で確認する)
    check_values(&input)?;
    let (running, queued) = jobs.active_counts().await?;
    ensure_idle(running, queued)?;
    let png = inputs.read(image).await?;
    let models: EngineModels = engine.list_models().await?;
    let req = resolve_request(&input, &models.taggers)?;
    tracing::debug!(model = %req.model, bytes = png.len(), "tagging requested");
    let tags = engine.engine()?.tag_image(png, &req).await?;
    Ok(ImageTagResult {
        tags,
        model: req.model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taggers() -> Vec<String> {
        vec!["wd-eva02-large-tagger-v3".into(), DEFAULT_MODEL.into()]
    }

    #[test]
    fn defaults_pick_the_standard_model_when_listed() {
        let r = resolve_request(&TagInput::default(), &taggers()).unwrap();
        assert_eq!(r.model, DEFAULT_MODEL);
        assert_eq!(r.threshold, 0.35);
        assert_eq!(r.character_threshold, 0.85);
        assert_eq!(r.exclude_tags, "");
    }

    #[test]
    fn falls_back_to_first_when_default_is_not_listed() {
        let list = vec!["wd-vit-tagger-v3".to_string()];
        let r = resolve_request(&TagInput::default(), &list).unwrap();
        assert_eq!(r.model, "wd-vit-tagger-v3");
    }

    #[test]
    fn missing_node_when_no_candidates() {
        let e = resolve_request(&TagInput::default(), &[]).unwrap_err();
        assert_eq!(e.code(), "ENGINE_NODE_MISSING");
    }

    #[test]
    fn rejects_out_of_range_and_unknown_values() {
        for t in [0.0, 0.04, 1.01, f64::NAN, f64::INFINITY] {
            let e = resolve_request(
                &TagInput {
                    threshold: Some(t),
                    ..Default::default()
                },
                &taggers(),
            )
            .unwrap_err();
            assert_eq!(e.code(), "VALIDATION", "threshold {t}");
            let e = resolve_request(
                &TagInput {
                    character_threshold: Some(t),
                    ..Default::default()
                },
                &taggers(),
            )
            .unwrap_err();
            assert_eq!(e.code(), "VALIDATION", "character {t}");
        }
        let e = resolve_request(
            &TagInput {
                model: Some("not-a-model".into()),
                ..Default::default()
            },
            &taggers(),
        )
        .unwrap_err();
        assert_eq!(e.code(), "VALIDATION");
        let e = resolve_request(
            &TagInput {
                exclude_tags: Some("a".repeat(1001)),
                ..Default::default()
            },
            &taggers(),
        )
        .unwrap_err();
        assert_eq!(e.code(), "VALIDATION");
    }

    #[test]
    fn accepts_boundaries_and_explicit_model() {
        let r = resolve_request(
            &TagInput {
                model: Some("wd-eva02-large-tagger-v3".into()),
                threshold: Some(0.05),
                character_threshold: Some(1.0),
                exclude_tags: Some("blurry, text".into()),
            },
            &taggers(),
        )
        .unwrap();
        assert_eq!(r.model, "wd-eva02-large-tagger-v3");
        assert_eq!((r.threshold, r.character_threshold), (0.05, 1.0));
        assert_eq!(r.exclude_tags, "blurry, text");
    }

    #[test]
    fn busy_when_jobs_are_running_or_queued() {
        assert!(ensure_idle(0, 0).is_ok());
        assert_eq!(ensure_idle(1, 0).unwrap_err().code(), "ENGINE_BUSY");
        assert_eq!(ensure_idle(0, 2).unwrap_err().code(), "ENGINE_BUSY");
    }
}
