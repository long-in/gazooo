//! AI サポート(docs/11-ai-assistant.md)。キーの管理、会話の保存、Claude への問い合わせ。

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::db::Db;
use crate::db::assistant::{
    self as store, AssistantStatus, Conversation, ConversationSummary, StoredSuggestion,
};
use crate::domain::params::GenerationParams;
use crate::error::{AppError, AppResult};

pub mod anthropic;
pub mod capabilities;
pub mod context;
pub mod openai_compat;
pub mod translate;

use anthropic::{AnthropicClient, MessageRequest, StreamEnd, StreamEvent};
use openai_compat::{LocalCheck, OpenAiClient};
use store::Provider;

const MAX_QUESTION_CHARS: usize = 8000;
const MAX_KEY_CHARS: usize = 512;
/// カスタムのシステムプロンプトの最大文字数(docs/11 §12.2)。
pub const MAX_SYSTEM_PROMPT_CHARS: usize = 30_000;

/// フロントエンドへの通知(docs/04「イベント」)。テストで差し替えられるようトレイトにしている。
pub trait AssistantSink: Send + Sync {
    fn delta(&self, request_id: &str, text: &str);
    fn suggestion(&self, request_id: &str, index: usize, suggestion: &StoredSuggestion);
    fn done(&self, request_id: &str, cancelled: bool);
    fn error(&self, request_id: &str, error: &AppError);
}

/// アプリ管理のローカル LLM の接続先(docs/30 §6)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEndpoint {
    /// `http://127.0.0.1:<port>/v1`
    pub base_url: String,
    pub model: String,
}

/// アプリが管理するローカル LLM(`services::llm`)。テストで差し替えられるようトレイトにしている。
#[async_trait::async_trait]
pub trait LocalRuntime: Send + Sync {
    /// 導入済みか(未導入なら送信前に `LLM_NOT_INSTALLED`)。
    fn installed(&self) -> bool;
    /// 止まっていれば起動し、準備完了まで待つ。
    async fn ensure_ready(&self) -> AppResult<LocalEndpoint>;
    /// 回答の生成の開始・終了(実行中はアイドル停止しない)。
    fn use_started(&self);
    fn use_finished(&self);
}

/// 送信先(`prepare` で、設定から決める)。
enum Target {
    Anthropic { key: String, model: String },
    External { base: String, model: String },
    Managed,
}

/// プロンプト翻訳の設定と状態(docs/34 §5)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateStatus {
    pub provider: Provider,
    pub model: String,
    pub ready: bool,
    /// 実行できない理由(`no_key` / `no_consent` / `local_not_ready`)。
    pub reason: Option<&'static str>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslatePatch {
    pub provider: Option<Provider>,
    pub model: Option<String>,
}

/// 翻訳の送信先を、設定と共有の前提(キー・同意・ローカル LLM。docs/34 §4)から決める。
/// 実行できないときは、(理由の識別子, エラー)を返す。
fn translate_target(
    c: &rusqlite::Connection,
    provider: Provider,
    model: &str,
    local_installed: bool,
) -> Result<Target, (&'static str, AppError)> {
    let status = store::status(c).map_err(|e| ("error", e))?;
    match provider {
        Provider::Anthropic => {
            if !status.consent {
                return Err((
                    "no_consent",
                    AppError::assistant(
                        "ASSISTANT_NO_CONSENT",
                        "AI サポートの利用に同意してください(設定の「AI サポート」)",
                    ),
                ));
            }
            let key = store::api_key(c).map_err(|e| ("error", e))?.ok_or((
                "no_key",
                AppError::assistant("ASSISTANT_NO_KEY", "API キーが登録されていません"),
            ))?;
            Ok(Target::Anthropic {
                key,
                model: model.to_string(),
            })
        }
        Provider::LocalExternal => Ok(Target::External {
            base: openai_compat::validate_base_url(&status.local_base_url)
                .map_err(|e| ("error", e))?,
            model: status.local_model,
        }),
        Provider::Local => {
            if !local_installed {
                return Err((
                    "local_not_ready",
                    AppError::assistant(
                        "LLM_NOT_INSTALLED",
                        "ローカル LLM が導入されていません。設定の「AI サポート」から導入してください",
                    ),
                ));
            }
            Ok(Target::Managed)
        }
    }
}

/// 編集欄に出す内容(docs/11 §12.5)。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptState {
    pub text: String,
    pub default_text: String,
    pub customized: bool,
    pub model_guide: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantPatch {
    pub model: Option<String>,
    pub consent: Option<bool>,
    pub auto_delete: Option<bool>,
    pub provider: Option<Provider>,
    pub local_base_url: Option<String>,
    pub local_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskStarted {
    pub request_id: String,
    pub conversation_id: String,
    /// この回答として保存されるメッセージの ID(提案の適用済み記録に使う)。
    pub assistant_message_id: String,
}

struct Running {
    request_id: String,
    cancel: watch::Sender<bool>,
}

pub struct AssistantService {
    db: Db,
    client: AnthropicClient,
    openai: OpenAiClient,
    sink: Arc<dyn AssistantSink>,
    running: Mutex<Option<Running>>,
    /// アプリ管理のローカル LLM(起動後に設定する)。
    runtime: std::sync::OnceLock<Arc<dyn LocalRuntime>>,
    /// 実行中のプロンプト翻訳(相談とは独立。docs/34)。
    translating: Mutex<Option<Running>>,
}

impl AssistantService {
    pub fn new(db: Db, client: AnthropicClient, sink: Arc<dyn AssistantSink>) -> Arc<Self> {
        Arc::new(Self {
            db,
            client,
            openai: OpenAiClient::new(),
            sink,
            running: Mutex::new(None),
            runtime: std::sync::OnceLock::new(),
            translating: Mutex::new(None),
        })
    }

    /// アプリ管理のローカル LLM を接続する(起動時に 1 回)。
    pub fn set_runtime(&self, runtime: Arc<dyn LocalRuntime>) {
        let _ = self.runtime.set(runtime);
    }

    /// 外部サーバへの接続を確認する(docs/11 §13.4)。省略した項目は保存値を使う。
    pub async fn local_check(
        &self,
        base_url: Option<String>,
        model: Option<String>,
    ) -> AppResult<LocalCheck> {
        let status = self.status().await?;
        let base = openai_compat::validate_base_url(
            base_url.as_deref().unwrap_or(&status.local_base_url),
        )?;
        let model = model.unwrap_or(status.local_model);
        Ok(self.openai.check(&base, model.trim()).await)
    }

    pub async fn status(&self) -> AppResult<AssistantStatus> {
        self.db.call(|c| store::status(c)).await
    }

    pub async fn set_key(&self, key: String) -> AppResult<bool> {
        let key = key.trim().to_string();
        if key.is_empty() || key.len() > MAX_KEY_CHARS || !key.chars().all(|c| c.is_ascii_graphic())
        {
            return Err(AppError::validation(
                "key",
                "API キーの形式が正しくありません",
            ));
        }
        // 無効なキー(401/403)は保存しない。ネットワーク不可のときは未検証として保存する
        let verified = self.client.verify_key(&key).await?;
        self.db
            .call(move |c| store::set_api_key(c, &key, verified))
            .await?;
        Ok(verified)
    }

    pub async fn clear_key(&self) -> AppResult<()> {
        self.db.call(|c| store::clear_api_key(c)).await
    }

    pub async fn update(&self, patch: AssistantPatch) -> AppResult<AssistantStatus> {
        self.db
            .call(move |c| {
                if let Some(m) = &patch.model {
                    store::set_model(c, m)?;
                }
                if let Some(v) = patch.consent {
                    store::set_consent(c, v)?;
                }
                if let Some(v) = patch.auto_delete {
                    store::set_auto_delete(c, v)?;
                    if v {
                        // 有効に戻した時点で、上限を超えた分をすぐ削除する(確認はフロントの責務)
                        store::enforce_limit(c)?;
                    }
                }
                if let Some(p) = patch.provider {
                    store::set_provider(c, p)?;
                }
                if patch.local_base_url.is_some() || patch.local_model.is_some() {
                    let url = patch
                        .local_base_url
                        .as_deref()
                        .map(openai_compat::validate_base_url)
                        .transpose()?;
                    let model = patch.local_model.as_deref().map(str::trim);
                    if let Some(m) = model
                        && (m.is_empty()
                            || m.chars().count() > 200
                            || m.chars().any(char::is_control))
                    {
                        return Err(AppError::validation(
                            "localModel",
                            "モデル名は 1〜200 文字で入力してください",
                        ));
                    }
                    store::set_local_server(c, url.as_deref(), model)?;
                }
                store::status(c)
            })
            .await
    }

    /// 編集欄に出す内容(docs/11 §12.5)。
    pub async fn prompt_get(&self) -> AppResult<PromptState> {
        let custom = self.db.call(|c| store::system_prompt(c)).await?;
        let default_text = capabilities::default_prompt();
        Ok(PromptState {
            customized: custom.is_some(),
            text: custom.unwrap_or_else(|| default_text.clone()),
            default_text,
            model_guide: capabilities::model_guide_text(),
        })
    }

    /// 検証して保存する。初期値と完全一致なら未設定に戻す(= 未編集扱い)。
    pub async fn prompt_set(&self, text: String) -> AppResult<bool> {
        if text.trim().is_empty() {
            return Err(AppError::validation(
                "text",
                "システムプロンプトを空にはできません(初期値に戻すには「初期値に戻す」を使ってください)",
            ));
        }
        if text.chars().count() > MAX_SYSTEM_PROMPT_CHARS {
            return Err(AppError::validation(
                "text",
                "システムプロンプトが長すぎます(30,000 文字まで)",
            ));
        }
        let same = text == capabilities::default_prompt();
        self.db
            .call(move |c| {
                if same {
                    store::clear_system_prompt(c)
                } else {
                    store::set_system_prompt(c, &text)
                }
            })
            .await?;
        Ok(!same)
    }

    pub async fn prompt_reset(&self) -> AppResult<String> {
        self.db.call(|c| store::clear_system_prompt(c)).await?;
        Ok(capabilities::default_prompt())
    }

    pub async fn conversation_list(&self) -> AppResult<Vec<ConversationSummary>> {
        self.db.call(|c| store::list_conversations(c)).await
    }

    pub async fn conversation_get(&self, id: String) -> AppResult<Conversation> {
        self.db.call(move |c| store::get_conversation(c, &id)).await
    }

    /// `ids = None` はすべて削除。
    pub async fn conversation_delete(&self, ids: Option<Vec<String>>) -> AppResult<usize> {
        self.db
            .call(move |c| match ids {
                Some(ids) => store::delete_conversations(c, &ids),
                None => store::delete_all_conversations(c),
            })
            .await
    }

    pub async fn suggestion_applied(&self, message_id: String, index: usize) -> AppResult<()> {
        self.db
            .call(move |c| store::mark_suggestion_applied(c, &message_id, index))
            .await
    }

    pub fn cancel(&self, request_id: &str) {
        if let Ok(g) = self.running.lock()
            && let Some(r) = g.as_ref()
            && r.request_id == request_id
        {
            let _ = r.cancel.send(true);
        }
    }

    /// 相談を開始する。応答はイベントで流れ、完了後に会話へ保存される。
    #[cfg(test)]
    pub async fn ask(
        self: &Arc<Self>,
        conversation_id: Option<String>,
        text: String,
        params: GenerationParams,
    ) -> AppResult<AskStarted> {
        self.ask_with(conversation_id, text, params, None, None)
            .await
    }

    /// `capabilities`: 今の ComfyUI で使える機能(docs/13 §4.2)。接続状態が不明なら `None`。
    /// `models`: 選択中・導入済みのモデルのファミリー ID(docs/18 §6.2)。無ければ `None`。
    pub async fn ask_with(
        self: &Arc<Self>,
        conversation_id: Option<String>,
        text: String,
        params: GenerationParams,
        capabilities: Option<serde_json::Value>,
        models: Option<context::ModelContext>,
    ) -> AppResult<AskStarted> {
        let text = text.trim().to_string();
        if text.is_empty() || text.chars().count() > MAX_QUESTION_CHARS {
            return Err(AppError::validation(
                "text",
                format!("質問は 1〜{MAX_QUESTION_CHARS} 文字で入力してください"),
            ));
        }

        // 同時に実行できるのは 1 件
        let request_id = uuid::Uuid::now_v7().to_string();
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        {
            let mut g = self
                .running
                .lock()
                .map_err(|_| AppError::internal("lock poisoned"))?;
            if g.is_some() {
                return Err(AppError::assistant(
                    "ASSISTANT_BUSY",
                    "別の相談が実行中です。完了するか停止してください",
                ));
            }
            *g = Some(Running {
                request_id: request_id.clone(),
                cancel: cancel_tx,
            });
        }

        let prepared = self.prepare(conversation_id, &text, &params).await;
        let (conversation_id, target, history, custom_prompt) = match prepared {
            Ok(p) => p,
            Err(e) => {
                self.release();
                return Err(e);
            }
        };

        let started = AskStarted {
            request_id: request_id.clone(),
            conversation_id: conversation_id.clone(),
            assistant_message_id: uuid::Uuid::now_v7().to_string(),
        };
        let me = Arc::clone(self);
        let assistant_message_id = started.assistant_message_id.clone();
        // 選択中のモデルは、カタログにあるファミリー ID だけに絞る(ファイル名・パスは送らない)
        let catalog = capabilities::catalog();
        let current_family = models
            .as_ref()
            .and_then(|m| m.family_id.clone())
            .filter(|id| catalog.is_some_and(|c| c.families.iter().any(|f| &f.id == id)));
        let models_value = match (&models, catalog) {
            (Some(m), Some(c)) => Some(m.to_value(c)),
            _ => None,
        };
        let mut messages = history;
        messages.push((
            "user".to_string(),
            context::user_content(
                &context::snapshot(&params),
                capabilities.as_ref(),
                models_value.as_ref(),
                &text,
            ),
        ));
        tokio::spawn(async move {
            let tools = context::tool_definition();
            let system = capabilities::system_prompt(custom_prompt.as_deref());
            let mut answer = String::new();
            let mut suggestions: Vec<StoredSuggestion> = Vec::new();
            let sink = Arc::clone(&me.sink);
            let rid = request_id.clone();
            let mut on_event = |ev: StreamEvent| match ev {
                StreamEvent::Text(t) => {
                    answer.push_str(&t);
                    sink.delta(&rid, &t);
                }
                StreamEvent::Stop(_) => {}
                StreamEvent::ToolUse { name, input } => {
                    // 回答 1 回につき、設定の提案とモデルの提案をそれぞれ 1 件まで
                    let has_params = suggestions.iter().any(|s| s.model.is_none());
                    let has_model = suggestions.iter().any(|s| s.model.is_some());
                    let accepted = if name == context::TOOL_NAME && !has_params {
                        context::sanitize(&input, &params)
                    } else if name == context::TOOL_MODEL_NAME
                        && !has_model
                        && let Some(c) = capabilities::catalog()
                    {
                        context::sanitize_model(&input, c, current_family.as_deref())
                    } else {
                        None
                    };
                    if let Some(s) = accepted {
                        sink.suggestion(&rid, suggestions.len(), &s);
                        suggestions.push(s);
                    }
                }
            };
            let result = me
                .dispatch(
                    &target,
                    &system,
                    &messages,
                    tools,
                    &mut cancel_rx,
                    &mut on_event,
                )
                .await;

            // 途中までの内容も含め、あれば保存する
            if !answer.trim().is_empty() || !suggestions.is_empty() {
                let (cid, mid, text, sugg) = (
                    conversation_id.clone(),
                    assistant_message_id,
                    answer.clone(),
                    suggestions.clone(),
                );
                if let Err(e) = me
                    .db
                    .call(move |c| {
                        store::append_message(c, &cid, &mid, "assistant", &text, None, &sugg)
                    })
                    .await
                {
                    tracing::warn!(error = %e, "assistant message save failed");
                }
            }
            match result {
                Ok(end) => me.sink.done(&request_id, end == StreamEnd::Cancelled),
                Err(e) => {
                    tracing::warn!(code = e.code(), "assistant request failed");
                    me.sink.error(&request_id, &e);
                }
            }
            me.release();
        });
        Ok(started)
    }

    /// 送信先へメッセージを送り、ストリームを `on_event` へ流す(相談・翻訳で共通)。
    async fn dispatch(
        &self,
        target: &Target,
        system: &str,
        messages: &[(String, String)],
        tools: serde_json::Value,
        cancel_rx: &mut watch::Receiver<bool>,
        on_event: impl FnMut(StreamEvent),
    ) -> AppResult<StreamEnd> {
        match target {
            Target::Anthropic { key, model } => {
                let req = MessageRequest {
                    model,
                    system,
                    messages,
                    tools,
                };
                self.client.stream(key, &req, cancel_rx, on_event).await
            }
            Target::External { base, model } => {
                let req = MessageRequest {
                    model,
                    system,
                    messages,
                    tools,
                };
                self.openai
                    .stream(base, model, &req, cancel_rx, on_event)
                    .await
            }
            Target::Managed => match self.runtime.get() {
                None => Err(AppError::assistant(
                    "LLM_NOT_INSTALLED",
                    "ローカル LLM が利用できません",
                )),
                Some(rt) => {
                    // 回答の生成中はアイドル停止の対象にしない(起動の待ち時間も含める)
                    rt.use_started();
                    let started = tokio::select! {
                        _ = anthropic::cancelled(cancel_rx) => None,
                        r = rt.ensure_ready() => Some(r),
                    };
                    let result = match started {
                        None => Ok(StreamEnd::Cancelled),
                        Some(Err(e)) => Err(e),
                        Some(Ok(ep)) => {
                            let req = MessageRequest {
                                model: &ep.model,
                                system,
                                messages,
                                tools,
                            };
                            self.openai
                                .stream(&ep.base_url, &ep.model, &req, cancel_rx, on_event)
                                .await
                        }
                    };
                    rt.use_finished();
                    result
                }
            },
        }
    }

    /// プロンプト翻訳の設定と、今すぐ実行できるか(docs/34 §5)。
    pub async fn translate_status(&self) -> AppResult<TranslateStatus> {
        let local_installed = self.runtime.get().is_some_and(|r| r.installed());
        self.db
            .call(move |c| {
                let (provider, model) = store::translate_settings(c)?;
                let reason = translate_target(c, provider, &model, local_installed).err();
                Ok(TranslateStatus {
                    provider,
                    model,
                    ready: reason.is_none(),
                    reason: reason.map(|r| r.0),
                })
            })
            .await
    }

    /// プロバイダ・モデルを更新する(AI サポートの設定は変えない)。
    pub async fn translate_update(&self, patch: TranslatePatch) -> AppResult<()> {
        self.db
            .call(move |c| {
                if let Some(p) = patch.provider {
                    store::set_translate_provider(c, p)?;
                }
                if let Some(m) = &patch.model {
                    store::set_translate_model(c, m)?;
                }
                Ok(())
            })
            .await
    }

    pub fn translate_cancel(&self, request_id: &str) {
        if let Ok(g) = self.translating.lock()
            && let Some(r) = g.as_ref()
            && r.request_id == request_id
        {
            let _ = r.cancel.send(true);
        }
    }

    /// 日本語の入力を、`target` のモデルに合ったプロンプトへ翻訳・校正する(docs/34)。
    /// 中断されたら `None`。会話には保存しない。ログに本文は出さない。
    pub async fn translate(
        &self,
        request_id: String,
        text: String,
        target: translate::TranslateTarget,
    ) -> AppResult<Option<String>> {
        let text = text.trim().to_string();
        let len = text.chars().count();
        if len == 0 || len > translate::MAX_INPUT_CHARS {
            return Err(AppError::validation(
                "text",
                format!(
                    "入力は 1〜{} 文字で入力してください",
                    translate::MAX_INPUT_CHARS
                ),
            ));
        }
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        {
            let mut g = self
                .translating
                .lock()
                .map_err(|_| AppError::internal("lock poisoned"))?;
            if g.is_some() {
                return Err(AppError::assistant(
                    "ASSISTANT_BUSY",
                    "別の翻訳が実行中です。完了するか停止してください",
                ));
            }
            *g = Some(Running {
                request_id,
                cancel: cancel_tx,
            });
        }
        let result = self.translate_inner(&text, &target, &mut cancel_rx).await;
        if let Ok(mut g) = self.translating.lock() {
            *g = None;
        }
        if let Err(e) = &result {
            tracing::warn!(code = e.code(), detail = %e, "prompt translate failed");
        }
        result
    }

    async fn translate_inner(
        &self,
        text: &str,
        target: &translate::TranslateTarget,
        cancel_rx: &mut watch::Receiver<bool>,
    ) -> AppResult<Option<String>> {
        let local_installed = self.runtime.get().is_some_and(|r| r.installed());
        let dest = self
            .db
            .call(move |c| {
                let (provider, model) = store::translate_settings(c)?;
                translate_target(c, provider, &model, local_installed).map_err(|r| r.1)
            })
            .await?;
        let system = translate::system_prompt();
        let user = translate::user_message(capabilities::catalog(), target, text);
        let messages = vec![("user".to_string(), user)];
        let mut out = String::new();
        let mut stop: Option<String> = None;
        let end = self
            .dispatch(
                &dest,
                &system,
                &messages,
                serde_json::json!([]),
                cancel_rx,
                |ev| match ev {
                    StreamEvent::Text(t) => out.push_str(&t),
                    StreamEvent::Stop(r) => stop = Some(r),
                    StreamEvent::ToolUse { .. } => {}
                },
            )
            .await?;
        if end == StreamEnd::Cancelled {
            return Ok(None);
        }
        let cleaned = translate::clean_output(&out);
        if cleaned.contains(translate::REFUSED_MARK) {
            return Err(AppError::assistant(
                "ASSISTANT_API_ERROR",
                "この内容は翻訳できません(未成年や実在の人物を性的に描く内容は扱えません)。入力を見直してください",
            ));
        }
        if cleaned.is_empty() {
            let reason = stop.as_deref().unwrap_or("不明");
            tracing::warn!(stop_reason = reason, "prompt translate returned no text");
            let message = if reason == "refusal" {
                "Claude が内容を理由に応答を断りました(停止理由: refusal)。表現を変えるか、設定の「プロンプト翻訳」でローカル LLM を選んでください".to_string()
            } else {
                format!(
                    "翻訳結果が空でした(停止理由: {reason})。入力を変えて、もう一度試してください"
                )
            };
            return Err(AppError::assistant("ASSISTANT_API_ERROR", message));
        }
        Ok(Some(cleaned))
    }

    fn release(&self) {
        if let Ok(mut g) = self.running.lock() {
            *g = None;
        }
    }

    /// 検証・会話の作成・質問の保存・履歴の取得を 1 回の DB 呼び出しで行う。
    async fn prepare(
        &self,
        conversation_id: Option<String>,
        text: &str,
        params: &GenerationParams,
    ) -> AppResult<(String, Target, Vec<(String, String)>, Option<String>)> {
        let params_json = serde_json::to_string(&context::snapshot(params))?;
        let text = text.to_string();
        let local_installed = self.runtime.get().is_some_and(|r| r.installed());
        self.db
            .call(move |c| {
                let status = store::status(c)?;
                // 送信先ごとの前提を、会話を保存する前に確認する(docs/11 §13.2・docs/30 §6)
                let target = match status.provider {
                    Provider::Anthropic => {
                        if !status.consent {
                            return Err(AppError::assistant(
                                "ASSISTANT_NO_CONSENT",
                                "AI サポートの利用に同意してください",
                            ));
                        }
                        let key = store::api_key(c)?.ok_or_else(|| {
                            AppError::assistant("ASSISTANT_NO_KEY", "API キーが登録されていません")
                        })?;
                        Target::Anthropic {
                            key,
                            model: status.model.clone(),
                        }
                    }
                    Provider::LocalExternal => Target::External {
                        base: openai_compat::validate_base_url(&status.local_base_url)?,
                        model: status.local_model.clone(),
                    },
                    Provider::Local => {
                        if !local_installed {
                            return Err(AppError::assistant(
                                "LLM_NOT_INSTALLED",
                                "ローカル LLM が導入されていません。設定の「AI サポート」から導入してください",
                            ));
                        }
                        Target::Managed
                    }
                };
                let tx = c.transaction()?;
                let cid = match conversation_id {
                    Some(id) => {
                        if !store::conversation_exists(&tx, &id)? {
                            return Err(AppError::assistant(
                                "ASSISTANT_CONVERSATION_NOT_FOUND",
                                "会話が見つかりません",
                            ));
                        }
                        id
                    }
                    None => {
                        let id = uuid::Uuid::now_v7().to_string();
                        store::create_conversation(&tx, &id, &context::title_from(&text))?;
                        id
                    }
                };
                let mid = uuid::Uuid::now_v7().to_string();
                store::append_message(&tx, &cid, &mid, "user", &text, Some(&params_json), &[])?;
                store::enforce_limit(&tx)?;
                let mut rows = store::recent_messages(&tx, &cid, context::HISTORY_LIMIT)?;
                tx.commit()?;
                // 最新の質問は、フォームの内容を付けて別に組み立てる
                rows.pop();
                let custom = store::system_prompt(c)?;
                Ok((cid, target, context::build_history(rows), custom))
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::params::{ModelKind, sample};
    use crate::engine::comfyui::testutil::{Route, spawn};

    #[derive(Default)]
    struct Rec(Mutex<Vec<String>>);
    impl Rec {
        fn log(&self, s: String) {
            self.0.lock().unwrap().push(s);
        }
        fn all(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }
    impl AssistantSink for Rec {
        fn delta(&self, _: &str, t: &str) {
            self.log(format!("delta:{t}"));
        }
        fn suggestion(&self, _: &str, i: usize, s: &StoredSuggestion) {
            match &s.model {
                Some(m) => self.log(format!("model-suggestion:{i}:{}", m.family_id)),
                None => self.log(format!(
                    "suggestion:{i}:{}",
                    serde_json::Value::Object(s.changes.clone())
                )),
            }
        }
        fn done(&self, _: &str, c: bool) {
            self.log(format!("done:{c}"));
        }
        fn error(&self, _: &str, e: &AppError) {
            self.log(format!("error:{}", e.code()));
        }
    }

    const SSE_OK: &str = concat!(
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"steps を増やすと良いです\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"name\":\"suggest_params\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"summary\\\":\\\"増やす\\\",\\\"changes\\\":{\\\"steps\\\":40,\\\"seed\\\":1}}\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );

    /// 設定の提案(steps)と、モデルの提案(NoobAI-XL)と、カタログ外の ID の提案(捨てられる)。
    const SSE_MODEL: &str = concat!(
        "event: content_block_start
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}

",
        "event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"アニメ調なら NoobAI-XL がおすすめです\"}}

",
        "event: content_block_stop
data: {\"type\":\"content_block_stop\",\"index\":0}

",
        "event: content_block_start
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"name\":\"suggest_model\"}}

",
        "event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"familyId\\\":\\\"checkpoint-noobai-xl\\\",\\\"reason\\\":\\\"Danbooru タグでアニメ調を指定できるため\\\"}\"}}

",
        "event: content_block_stop
data: {\"type\":\"content_block_stop\",\"index\":1}

",
        "event: content_block_start
data: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"name\":\"suggest_model\"}}

",
        "event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"familyId\\\":\\\"checkpoint-animagine-xl-4\\\",\\\"reason\\\":\\\"2 件目は捨てられる\\\"}\"}}

",
        "event: content_block_stop
data: {\"type\":\"content_block_stop\",\"index\":2}

",
        "event: message_stop
data: {\"type\":\"message_stop\"}

",
    );

    async fn setup(
        routes: Vec<Route>,
    ) -> (
        Arc<AssistantService>,
        Arc<Rec>,
        Db,
        crate::engine::comfyui::testutil::Mock,
    ) {
        let mock = spawn(routes).await;
        let db = Db::open_in_memory().unwrap();
        let rec = Arc::new(Rec::default());
        let svc = AssistantService::new(
            db.clone(),
            AnthropicClient::with_base(&mock.url).unwrap(),
            rec.clone(),
        );
        (svc, rec, db, mock)
    }

    async fn ready(svc: &Arc<AssistantService>) {
        svc.set_key("sk-ant-test".into()).await.unwrap();
        svc.update(AssistantPatch {
            consent: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    }

    async fn wait_done(rec: &Arc<Rec>) {
        for _ in 0..200 {
            if rec
                .all()
                .iter()
                .any(|l| l.starts_with("done:") || l.starts_with("error:"))
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("did not finish: {:?}", rec.all());
    }

    #[tokio::test]
    async fn system_prompt_is_editable_resettable_and_validated() {
        let (svc, _rec, db, _mock) = setup(vec![]).await;
        // 未設定は初期値
        let g = svc.prompt_get().await.unwrap();
        assert!(!g.customized);
        assert_eq!(g.text, g.default_text);
        assert!(g.model_guide.contains("# モデルガイド"));
        // 空・上限超えは拒否
        assert_eq!(
            svc.prompt_set(
                "  
"
                .into()
            )
            .await
            .unwrap_err()
            .code(),
            "VALIDATION"
        );
        let long = "あ".repeat(MAX_SYSTEM_PROMPT_CHARS + 1);
        assert_eq!(svc.prompt_set(long).await.unwrap_err().code(), "VALIDATION");
        // 保存すると編集済みになり、DB に入る
        assert!(svc.prompt_set("カスタム".into()).await.unwrap());
        let g = svc.prompt_get().await.unwrap();
        assert!(g.customized && g.text == "カスタム");
        assert_eq!(
            db.call(|c| store::system_prompt(c))
                .await
                .unwrap()
                .as_deref(),
            Some("カスタム")
        );
        // 初期値と一致なら未設定に戻る
        assert!(!svc.prompt_set(g.default_text.clone()).await.unwrap());
        assert!(!svc.prompt_get().await.unwrap().customized);
        // リセット
        svc.prompt_set("もう一度".into()).await.unwrap();
        assert_eq!(svc.prompt_reset().await.unwrap(), g.default_text);
        assert!(
            db.call(|c| store::system_prompt(c))
                .await
                .unwrap()
                .is_none()
        );
        // status に本文は含まれない
        svc.prompt_set("秘密の文面".into()).await.unwrap();
        let st = format!("{:?}", svc.status().await.unwrap());
        assert!(!st.contains("秘密の文面"));
    }

    #[tokio::test]
    async fn refuses_without_consent_or_key_and_makes_no_request() {
        let (svc, _rec, _db, mock) = setup(vec![]).await;
        let p = sample(ModelKind::Sd15);
        let e = svc.ask(None, "q".into(), p.clone()).await.unwrap_err();
        assert_eq!(e.code(), "ASSISTANT_NO_CONSENT");
        svc.update(AssistantPatch {
            consent: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        let e = svc.ask(None, "q".into(), p).await.unwrap_err();
        assert_eq!(e.code(), "ASSISTANT_NO_KEY");
        assert!(mock.requests.lock().unwrap().is_empty(), "外部通信しない");
        // 失敗しても次の相談を妨げない
        assert!(svc.running.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn set_key_verifies_and_rejects_invalid() {
        let (svc, _rec, _db, _mock) = setup(vec![Route::get("/v1/models?limit=1", "{}")]).await;
        assert!(svc.set_key("sk-ant-ok".into()).await.unwrap());
        let s = svc.status().await.unwrap();
        assert!(s.has_key && s.verified);

        let (svc, _r, _d, _m) = setup(vec![Route::seq(
            "GET /v1/models?limit=1",
            vec![(401, "{}")],
        )])
        .await;
        let e = svc.set_key("sk-ant-bad".into()).await.unwrap_err();
        assert_eq!(e.code(), "ASSISTANT_AUTH");
        assert!(
            !svc.status().await.unwrap().has_key,
            "無効なキーは保存しない"
        );

        let e = svc.set_key("has space".into()).await.unwrap_err();
        assert_eq!(e.code(), "VALIDATION");
    }

    #[tokio::test]
    async fn model_suggestion_is_validated_saved_and_the_guide_is_sent() {
        let (svc, rec, _db, mock) = setup(vec![
            Route::get("/v1/models?limit=1", "{}"),
            Route::post("/v1/messages", 200, SSE_MODEL),
        ])
        .await;
        ready(&svc).await;
        let mut p = sample(ModelKind::Sdxl);
        p.model.name = "my-private-checkpoint-v7.safetensors".into();
        let models = context::ModelContext {
            family_id: None,
            installed_families: vec![
                "checkpoint-sdxl".into(),
                "my-private-checkpoint-v7.safetensors".into(),
            ],
        };
        let started = svc
            .ask_with(None, "アニメ調にしたい".into(), p, None, Some(models))
            .await
            .unwrap();
        wait_done(&rec).await;

        // 回答 1 回につきモデルの提案は 1 件まで(2 件目は捨てる)
        let log = rec.all();
        let model_logs: Vec<_> = log
            .iter()
            .filter(|l| l.starts_with("model-suggestion:"))
            .collect();
        assert_eq!(
            model_logs,
            vec!["model-suggestion:0:checkpoint-noobai-xl"],
            "{log:?}"
        );

        let conv = svc
            .conversation_get(started.conversation_id.clone())
            .await
            .unwrap();
        let saved = &conv.messages[1].suggestions;
        assert_eq!(saved.len(), 1);
        assert_eq!(
            saved[0].model.as_ref().unwrap().family_id,
            "checkpoint-noobai-xl"
        );
        assert!(saved[0].changes.is_empty());

        // 送った内容: モデルガイドとルール(システム)、導入済みのファミリー ID。ファイル名は送らない
        let sent = {
            let reqs = mock.requests.lock().unwrap();
            reqs.iter()
                .find(|r| r.starts_with("POST /v1/messages"))
                .unwrap()
                .clone()
        };
        assert!(sent.contains("# モデルガイド") && sent.contains("[checkpoint-noobai-xl]"));
        assert!(sent.contains("suggest_model"));
        assert!(sent.contains("installedFamilies") && sent.contains("checkpoint-sdxl"));
        // 導入済みの一覧に混ぜたファイル名は、カタログに無いので捨てられる(ModelContext のテストも参照)
        assert!(!sent.contains(
            "\"installedFamilies\": [
    \"checkpoint-sdxl\",
    \"my-private"
        ));
    }

    #[tokio::test]
    async fn ask_streams_saves_conversation_and_filters_suggestion() {
        let (svc, rec, db, mock) = setup(vec![
            Route::get("/v1/models?limit=1", "{}"),
            Route::post("/v1/messages", 200, SSE_OK),
        ])
        .await;
        ready(&svc).await;
        let mut p = sample(ModelKind::Sd15);
        p.prompt = "a cat".into();
        p.steps = 20;
        p.seed = 424242;
        let started = svc.ask(None, "もっと綺麗に".into(), p).await.unwrap();
        wait_done(&rec).await;

        let log = rec.all();
        assert!(log.iter().any(|l| l.starts_with("delta:steps")));
        // seed はホワイトリスト外なので除外され、steps だけが提案される
        assert!(
            log.contains(&"suggestion:0:{\"steps\":40}".to_string()),
            "{log:?}"
        );
        assert_eq!(log.last().unwrap(), "done:false");

        let conv = svc
            .conversation_get(started.conversation_id.clone())
            .await
            .unwrap();
        assert_eq!(conv.title, "もっと綺麗に");
        assert_eq!(conv.messages.len(), 2);
        assert_eq!(conv.messages[0].role, "user");
        assert_eq!(conv.messages[1].id, started.assistant_message_id);
        assert_eq!(conv.messages[1].suggestions.len(), 1);

        // 送ったリクエストに、キー・seed・ComfyUI の URL は含まれない
        let sent = {
            let reqs = mock.requests.lock().unwrap();
            reqs.iter()
                .find(|r| r.starts_with("POST /v1/messages"))
                .unwrap()
                .clone()
        };
        assert!(sent.contains("a cat") && sent.contains("もっと綺麗に"));
        assert!(!sent.contains("424242"));
        assert!(
            !sent.contains("sk-ant-test"),
            "キーは本文に含めない(ヘッダのみ)"
        );

        // 適用済みの記録
        svc.suggestion_applied(started.assistant_message_id.clone(), 0)
            .await
            .unwrap();
        let conv = svc.conversation_get(started.conversation_id).await.unwrap();
        assert!(conv.messages[1].suggestions[0].applied);
        let _ = db;
    }

    #[tokio::test]
    async fn api_errors_map_to_codes_and_keep_user_message() {
        let (svc, rec, _db, _mock) = setup(vec![
            Route::get("/v1/models?limit=1", "{}"),
            Route::post("/v1/messages", 429, "{}"),
        ])
        .await;
        ready(&svc).await;
        let started = svc
            .ask(None, "q".into(), sample(ModelKind::Sd15))
            .await
            .unwrap();
        wait_done(&rec).await;
        assert_eq!(rec.all().last().unwrap(), "error:ASSISTANT_RATE_LIMIT");
        let conv = svc.conversation_get(started.conversation_id).await.unwrap();
        assert_eq!(
            conv.messages.len(),
            1,
            "失敗しても質問は残り、回答は保存されない"
        );
        // 完了後は再度相談できる
        for _ in 0..100 {
            if svc.running.lock().unwrap().is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(svc.running.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn busy_while_running_and_unknown_conversation_rejected() {
        let (svc, _rec, _db, _mock) = setup(vec![Route::get("/v1/models?limit=1", "{}")]).await;
        ready(&svc).await;
        let e = svc
            .ask(Some("nope".into()), "q".into(), sample(ModelKind::Sd15))
            .await
            .unwrap_err();
        assert_eq!(e.code(), "ASSISTANT_CONVERSATION_NOT_FOUND");

        let (tx, _rx) = watch::channel(false);
        *svc.running.lock().unwrap() = Some(Running {
            request_id: "x".into(),
            cancel: tx,
        });
        let e = svc
            .ask(None, "q".into(), sample(ModelKind::Sd15))
            .await
            .unwrap_err();
        assert_eq!(e.code(), "ASSISTANT_BUSY");
    }

    #[tokio::test]
    async fn cancel_signals_only_the_matching_request() {
        let (svc, _rec, _db, _mock) = setup(vec![]).await;
        let (tx, rx) = watch::channel(false);
        *svc.running.lock().unwrap() = Some(Running {
            request_id: "r1".into(),
            cancel: tx,
        });
        svc.cancel("other");
        assert!(!*rx.borrow());
        svc.cancel("r1");
        assert!(*rx.borrow());
    }

    #[tokio::test]
    async fn follow_up_includes_history() {
        let (svc, rec, _db, mock) = setup(vec![
            Route::get("/v1/models?limit=1", "{}"),
            Route::post("/v1/messages", 200, SSE_OK),
        ])
        .await;
        ready(&svc).await;
        let p = sample(ModelKind::Sd15);
        let first = svc.ask(None, "最初の質問".into(), p.clone()).await.unwrap();
        wait_done(&rec).await;
        for _ in 0..100 {
            if svc.running.lock().unwrap().is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        rec.0.lock().unwrap().clear();
        svc.ask(Some(first.conversation_id.clone()), "続きの質問".into(), p)
            .await
            .unwrap();
        wait_done(&rec).await;
        let reqs = mock.requests.lock().unwrap();
        let second = reqs
            .iter()
            .filter(|r| r.starts_with("POST /v1/messages"))
            .nth(1)
            .unwrap();
        assert!(second.contains("最初の質問") && second.contains("steps を増やすと良いです"));
        assert!(second.contains("続きの質問"));
    }

    // ---------------------------------------------------------------- ローカル LLM(docs/11 §13・docs/30)

    /// OpenAI 互換の SSE(回答 + `suggest_params` の断片化した引数 + 思考)。
    const OPENAI_SSE: &str = concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"秘密の思考\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"<think>隠す</think>steps を増やすと良いです\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c\",\"function\":{\"name\":\"suggest_params\",\"arguments\":\"{\\\"summary\\\":\\\"増やす\\\",\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"changes\\\":{\\\"steps\\\":40,\\\"seed\\\":1}}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );

    async fn use_external(svc: &Arc<AssistantService>, mock_url: &str) {
        svc.update(AssistantPatch {
            provider: Some(Provider::LocalExternal),
            local_base_url: Some(format!("{mock_url}/v1")),
            local_model: Some("qwen3.5:9b".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn external_local_server_needs_no_key_or_consent_and_never_calls_anthropic() {
        let (svc, rec, db, mock) =
            setup(vec![Route::post("/v1/chat/completions", 200, OPENAI_SSE)]).await;
        use_external(&svc, &mock.url).await;
        // キーも同意も無い
        let s = svc.status().await.unwrap();
        assert!(!s.has_key && !s.consent);

        let started = svc
            .ask(None, "もっと細かく".into(), sample(ModelKind::Sd15))
            .await
            .unwrap();
        wait_done(&rec).await;
        let log = rec.all();
        // 思考は出ない。回答と、検証済みの提案(seed は除外)が届く
        assert!(
            log.iter().any(|l| l == "delta:steps を増やすと良いです"),
            "{log:?}"
        );
        assert!(
            log.iter()
                .all(|l| !l.contains("思考") && !l.contains("隠す"))
        );
        assert!(
            log.iter().any(|l| l.starts_with("suggestion:0:")
                && l.contains("\"steps\":40")
                && !l.contains("seed")),
            "{log:?}"
        );
        assert!(log.contains(&"done:false".to_string()));

        let reqs = mock.requests.lock().unwrap().clone();
        assert_eq!(reqs.len(), 1);
        assert!(reqs[0].starts_with("POST /v1/chat/completions"));
        assert!(reqs[0].contains("enable_thinking"));
        assert!(reqs[0].contains("\"role\":\"system\""));
        assert!(
            reqs.iter().all(|r| !r.contains("/v1/messages")),
            "Anthropic へは送らない"
        );

        // 回答は会話に保存される
        let conv = db
            .call({
                let id = started.conversation_id.clone();
                move |c| store::get_conversation(c, &id)
            })
            .await
            .unwrap();
        assert_eq!(conv.messages.len(), 2);
        assert!(conv.messages[1].text.contains("steps を増やすと良いです"));
        assert_eq!(conv.messages[1].suggestions.len(), 1);
    }

    #[tokio::test]
    async fn update_rejects_non_loopback_urls_and_bad_models() {
        let (svc, _rec, _db, _mock) = setup(vec![]).await;
        for bad in [
            "http://192.168.1.10:11434/v1",
            "http://example.com/v1",
            "http://127.0.0.1.evil.test/v1",
            "javascript:alert(1)",
        ] {
            let e = svc
                .update(AssistantPatch {
                    local_base_url: Some(bad.into()),
                    ..Default::default()
                })
                .await
                .unwrap_err();
            assert_eq!(e.code(), "VALIDATION", "{bad}");
        }
        let e = svc
            .update(AssistantPatch {
                local_model: Some("   ".into()),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert_eq!(e.code(), "VALIDATION");
        // 保存値は変わらない
        let s = svc.status().await.unwrap();
        assert_eq!(
            s.local_base_url,
            crate::db::assistant::DEFAULT_LOCAL_BASE_URL
        );
        assert_eq!(s.provider, Provider::Anthropic);
    }

    #[tokio::test]
    async fn local_check_reports_reachability_and_model() {
        let (svc, _rec, _db, mock) = setup(vec![Route::get(
            "/v1/models",
            r#"{"data":[{"id":"qwen3.5:9b"},{"id":"other"}]}"#,
        )])
        .await;
        use_external(&svc, &mock.url).await;
        let c = svc.local_check(None, None).await.unwrap();
        assert!(c.reachable && c.model_found);
        assert_eq!(c.models, ["qwen3.5:9b", "other"]);
        let c = svc.local_check(None, Some("missing".into())).await.unwrap();
        assert!(c.reachable && !c.model_found);
        // 閉じたポートは「届かない」(エラーにしない)
        let c = svc
            .local_check(Some("http://127.0.0.1:9/v1".into()), None)
            .await
            .unwrap();
        assert!(!c.reachable && !c.model_found);
        // ループバック以外はそもそも確認しない
        assert_eq!(
            svc.local_check(Some("http://example.com/v1".into()), None)
                .await
                .unwrap_err()
                .code(),
            "VALIDATION"
        );
    }

    #[tokio::test]
    async fn unreachable_external_server_reports_a_local_message() {
        let (svc, rec, _db, _mock) = setup(vec![]).await;
        svc.update(AssistantPatch {
            provider: Some(Provider::LocalExternal),
            local_base_url: Some("http://127.0.0.1:9/v1".into()),
            ..Default::default()
        })
        .await
        .unwrap();
        svc.ask(None, "q".into(), sample(ModelKind::Sd15))
            .await
            .unwrap();
        wait_done(&rec).await;
        assert!(
            rec.all()
                .contains(&"error:ASSISTANT_UNREACHABLE".to_string())
        );
    }

    struct FakeRuntime {
        installed: bool,
        endpoint: Option<LocalEndpoint>,
        started: std::sync::atomic::AtomicU32,
        finished: std::sync::atomic::AtomicU32,
    }

    impl FakeRuntime {
        fn new(installed: bool, endpoint: Option<LocalEndpoint>) -> Arc<Self> {
            Arc::new(Self {
                installed,
                endpoint,
                started: Default::default(),
                finished: Default::default(),
            })
        }
        fn counts(&self) -> (u32, u32) {
            use std::sync::atomic::Ordering::SeqCst;
            (self.started.load(SeqCst), self.finished.load(SeqCst))
        }
    }

    #[async_trait::async_trait]
    impl LocalRuntime for FakeRuntime {
        fn installed(&self) -> bool {
            self.installed
        }
        async fn ensure_ready(&self) -> AppResult<LocalEndpoint> {
            self.endpoint
                .clone()
                .ok_or_else(|| AppError::assistant("LLM_START_FAILED", "起動できませんでした"))
        }
        fn use_started(&self) {
            self.started
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        fn use_finished(&self) {
            self.finished
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    async fn use_managed(svc: &Arc<AssistantService>) {
        svc.update(AssistantPatch {
            provider: Some(Provider::Local),
            ..Default::default()
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn managed_local_requires_installation_before_saving_anything() {
        let (svc, _rec, db, mock) = setup(vec![]).await;
        use_managed(&svc).await;
        // ランタイム未接続・未導入のどちらも、会話を作らずに断る
        let e = svc
            .ask(None, "q".into(), sample(ModelKind::Sd15))
            .await
            .unwrap_err();
        assert_eq!(e.code(), "LLM_NOT_INSTALLED");
        svc.set_runtime(FakeRuntime::new(false, None));
        let e = svc
            .ask(None, "q".into(), sample(ModelKind::Sd15))
            .await
            .unwrap_err();
        assert_eq!(e.code(), "LLM_NOT_INSTALLED");
        assert!(svc.conversation_list().await.unwrap().is_empty());
        assert!(mock.requests.lock().unwrap().is_empty());
        let _ = db;
    }

    #[tokio::test]
    async fn managed_local_streams_from_the_started_server_and_tracks_use() {
        let (svc, rec, _db, mock) =
            setup(vec![Route::post("/v1/chat/completions", 200, OPENAI_SSE)]).await;
        let rt = FakeRuntime::new(
            true,
            Some(LocalEndpoint {
                base_url: format!("{}/v1", mock.url),
                model: "qwen3.5-9b".into(),
            }),
        );
        svc.set_runtime(rt.clone());
        use_managed(&svc).await;
        svc.ask(None, "q".into(), sample(ModelKind::Sd15))
            .await
            .unwrap();
        wait_done(&rec).await;
        assert!(
            rec.all().contains(&"done:false".to_string()),
            "{:?}",
            rec.all()
        );
        assert!(rec.all().iter().any(|l| l.starts_with("suggestion:0:")));
        let reqs = mock.requests.lock().unwrap().clone();
        assert!(reqs[0].contains("\"model\":\"qwen3.5-9b\""));
        // 使用中の記録は対になる(アイドル停止を妨げ続けない)
        for _ in 0..100 {
            if rt.counts() == (1, 1) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(rt.counts(), (1, 1));
    }

    #[tokio::test]
    async fn managed_local_start_failure_is_reported_and_use_is_released() {
        let (svc, rec, _db, _mock) = setup(vec![]).await;
        let rt = FakeRuntime::new(true, None);
        svc.set_runtime(rt.clone());
        use_managed(&svc).await;
        svc.ask(None, "q".into(), sample(ModelKind::Sd15))
            .await
            .unwrap();
        wait_done(&rec).await;
        assert!(rec.all().contains(&"error:LLM_START_FAILED".to_string()));
        assert_eq!(rt.counts(), (1, 1));
        // 失敗しても次の相談を妨げない
        assert!(svc.running.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn anthropic_still_requires_consent_and_key_after_switching_back() {
        let (svc, _rec, _db, mock) = setup(vec![]).await;
        use_external(&svc, &mock.url).await;
        svc.update(AssistantPatch {
            provider: Some(Provider::Anthropic),
            ..Default::default()
        })
        .await
        .unwrap();
        let e = svc
            .ask(None, "q".into(), sample(ModelKind::Sd15))
            .await
            .unwrap_err();
        assert_eq!(e.code(), "ASSISTANT_NO_CONSENT");
        assert!(mock.requests.lock().unwrap().is_empty());
    }
}
