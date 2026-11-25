use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::db::assistant::{AssistantStatus, Conversation, ConversationSummary};
use crate::domain::params::GenerationParams;
use crate::error::AppResult;
use crate::services::assistant::openai_compat::LocalCheck;
use crate::services::assistant::{
    AskStarted, AssistantPatch, AssistantService, PromptState, TranslatePatch, TranslateStatus,
    capabilities,
    context::ModelContext,
    translate::{Mode, TranslateTarget},
};
use crate::services::engine::EngineService;

type Svc<'a> = State<'a, Arc<AssistantService>>;

#[derive(Serialize)]
pub struct KeyResult {
    pub verified: bool,
}

#[derive(Serialize)]
pub struct DeleteResult {
    pub deleted: usize,
}

#[tauri::command]
pub async fn assistant_status(svc: Svc<'_>) -> AppResult<AssistantStatus> {
    svc.status().await
}

#[tauri::command]
pub async fn assistant_set_key(svc: Svc<'_>, key: String) -> AppResult<KeyResult> {
    Ok(KeyResult {
        verified: svc.set_key(key).await?,
    })
}

#[tauri::command]
pub async fn assistant_clear_key(svc: Svc<'_>) -> AppResult<()> {
    svc.clear_key().await
}

#[tauri::command]
pub async fn assistant_update(svc: Svc<'_>, patch: AssistantPatch) -> AppResult<AssistantStatus> {
    svc.update(patch).await
}

#[derive(Serialize)]
pub struct PromptSaved {
    pub customized: bool,
}

#[derive(Serialize)]
pub struct PromptReset {
    pub text: String,
}

/// 外部サーバ(Ollama 等)への接続の確認(docs/11 §13.4)。省略した項目は保存値を使う。
#[tauri::command]
pub async fn assistant_local_check(
    svc: Svc<'_>,
    base_url: Option<String>,
    model: Option<String>,
) -> AppResult<LocalCheck> {
    svc.local_check(base_url, model).await
}

#[tauri::command]
pub async fn assistant_prompt_get(svc: Svc<'_>) -> AppResult<PromptState> {
    svc.prompt_get().await
}

#[tauri::command]
pub async fn assistant_prompt_set(svc: Svc<'_>, text: String) -> AppResult<PromptSaved> {
    Ok(PromptSaved {
        customized: svc.prompt_set(text).await?,
    })
}

#[tauri::command]
pub async fn assistant_prompt_reset(svc: Svc<'_>) -> AppResult<PromptReset> {
    Ok(PromptReset {
        text: svc.prompt_reset().await?,
    })
}

#[tauri::command]
pub async fn assistant_ask(
    svc: Svc<'_>,
    engine: State<'_, Arc<EngineService>>,
    conversation_id: Option<String>,
    text: String,
    params: GenerationParams,
    models: Option<ModelContext>,
) -> AppResult<AskStarted> {
    // 今の ComfyUI で使える機能も添える(通信はしない。docs/13 §4.2)
    let caps = capabilities::capabilities_json(engine.last_features().as_ref());
    svc.inner()
        .ask_with(conversation_id, text, params, caps, models)
        .await
}

#[tauri::command]
pub fn assistant_cancel(svc: Svc<'_>, request_id: String) {
    svc.cancel(&request_id);
}

#[tauri::command]
pub async fn assistant_conversation_list(svc: Svc<'_>) -> AppResult<Vec<ConversationSummary>> {
    svc.conversation_list().await
}

#[tauri::command]
pub async fn assistant_conversation_get(svc: Svc<'_>, id: String) -> AppResult<Conversation> {
    svc.conversation_get(id).await
}

/// `ids` が `null` のときはすべて削除する。
#[tauri::command]
pub async fn assistant_conversation_delete(
    svc: Svc<'_>,
    ids: Option<Vec<String>>,
) -> AppResult<DeleteResult> {
    Ok(DeleteResult {
        deleted: svc.conversation_delete(ids).await?,
    })
}

#[tauri::command]
pub async fn assistant_suggestion_applied(
    svc: Svc<'_>,
    message_id: String,
    index: usize,
) -> AppResult<()> {
    svc.suggestion_applied(message_id, index).await
}

#[derive(Serialize)]
pub struct TranslateResult {
    /// 中断されたときは `None`。
    pub text: Option<String>,
    /// `polish_failed`: 校正できず、翻訳のみを返した(docs/34 §9.2)。
    pub notice: Option<&'static str>,
}

#[tauri::command]
pub async fn translate_status(svc: Svc<'_>) -> AppResult<TranslateStatus> {
    svc.translate_status().await
}

#[tauri::command]
pub async fn translate_update(svc: Svc<'_>, patch: TranslatePatch) -> AppResult<TranslateStatus> {
    svc.translate_update(patch).await?;
    svc.translate_status().await
}

/// 日本語の入力を、選択中のモデルに合ったプロンプトへ翻訳・校正する(docs/34)。
#[tauri::command]
pub async fn translate_run(
    svc: Svc<'_>,
    request_id: String,
    text: String,
    mode: Mode,
    target: Option<TranslateTarget>,
) -> AppResult<TranslateResult> {
    let out = svc
        .translate(request_id, text, mode, target.unwrap_or_default())
        .await?;
    Ok(TranslateResult {
        text: out.text,
        notice: out.notice,
    })
}

#[tauri::command]
pub fn translate_cancel(svc: Svc<'_>, request_id: String) {
    svc.translate_cancel(&request_id);
}
