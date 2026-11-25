use std::sync::Arc;

use tauri::State;

use crate::error::AppResult;
use crate::services::engine::EngineService;
use crate::services::inputs::InputService;
use crate::services::jobs::JobService;
use crate::services::tagger::{self, ImageTagResult, TagInput};

/// 取り込み済みの参照画像を解析し、タグ列を返す(docs/39。ジョブ・履歴には残さない)。
#[tauri::command]
#[allow(clippy::too_many_arguments)] // IPC の引数は 04 の定義(平らな形)に合わせる
pub async fn image_tag(
    engine: State<'_, Arc<EngineService>>,
    inputs: State<'_, Arc<InputService>>,
    jobs: State<'_, Arc<JobService>>,
    image: String,
    model: Option<String>,
    threshold: Option<f64>,
    character_threshold: Option<f64>,
    exclude_tags: Option<String>,
) -> AppResult<ImageTagResult> {
    tagger::tag_image(
        &engine,
        &inputs,
        &jobs,
        &image,
        TagInput {
            model,
            threshold,
            character_threshold,
            exclude_tags,
        },
    )
    .await
}
