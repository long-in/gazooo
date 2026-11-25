//! 翻訳専用モデル(PLaMo 3 Translate)の導入・削除・ライセンス表示(docs/34 §9.13)。
//! 起動・停止・メモリ解放は、翻訳の実行時に自動で行う(本体のローカル LLM と同じ仕組み)。

use serde::Serialize;
use tauri::State;

use crate::commands::llm::InstallStarted;
use crate::error::{AppError, AppResult};
use crate::services::llm::{LlmStatus, MtLlm, plamo_license};

type Svc<'a> = State<'a, MtLlm>;

#[derive(Serialize)]
pub struct LicenseText {
    pub lang: String,
    pub text: String,
}

#[tauri::command]
pub async fn translate_mt_status(svc: Svc<'_>) -> AppResult<LlmStatus> {
    svc.0.status().await
}

#[tauri::command]
pub async fn translate_mt_install(svc: Svc<'_>) -> AppResult<InstallStarted> {
    Ok(InstallStarted {
        install_id: svc.0.install_start().await?,
    })
}

#[tauri::command]
pub fn translate_mt_install_cancel(svc: Svc<'_>, install_id: String) {
    svc.0.install_cancel(&install_id);
}

#[tauri::command]
pub async fn translate_mt_remove(svc: Svc<'_>) -> AppResult<LlmStatus> {
    svc.0.remove().await
}

/// PLaMo Community License の全文(`ja` / `en`)。アプリに同梱しているため、導入前の同意画面でも読める。
#[tauri::command]
pub fn translate_mt_license(lang: String) -> AppResult<LicenseText> {
    let text = plamo_license(&lang)
        .ok_or_else(|| AppError::validation("lang", "ja か en を指定してください"))?;
    Ok(LicenseText {
        lang,
        text: text.to_string(),
    })
}
