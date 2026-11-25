//! フロントエンドへのイベント通知(docs/04-ipc-api.md「イベント」)。
//! テストで差し替えられるようトレイトにしている。

use tauri::{AppHandle, Emitter};

use crate::domain::job::{ImageItem, Job, ProgressEvent};

pub trait EventSink: Send + Sync {
    fn job_updated(&self, job: &Job);
    fn job_progress(&self, progress: &ProgressEvent);
    fn image_created(&self, image: &ImageItem);
    fn image_deleted(&self, ids: &[String]);
    /// キューの一時停止・再開(docs/20 §3.3)。通知先が無いテスト用の実装は何もしなくてよい。
    fn queue_state(&self, _paused: bool) {}
    /// プロジェクトの作成・更新・削除・並べ替え(docs/40 §7)。
    fn project_changed(&self) {}
    /// 画像の所属が変わった(`project_id` = 移動先。`None` = 未分類)。
    fn image_moved(&self, _ids: &[String], _project_id: Option<&str>) {}
}

pub struct TauriSink(pub AppHandle);

impl EventSink for TauriSink {
    fn job_updated(&self, job: &Job) {
        let _ = self.0.emit("job://updated", job);
    }
    fn job_progress(&self, progress: &ProgressEvent) {
        let _ = self.0.emit("job://progress", progress);
    }
    fn image_created(&self, image: &ImageItem) {
        let _ = self.0.emit("image://created", image);
    }
    fn image_deleted(&self, ids: &[String]) {
        let _ = self
            .0
            .emit("image://deleted", serde_json::json!({ "ids": ids }));
    }
    fn queue_state(&self, paused: bool) {
        let _ = self
            .0
            .emit("queue://state", serde_json::json!({ "paused": paused }));
    }
    fn project_changed(&self) {
        let _ = self.0.emit("project://changed", ());
    }
    fn image_moved(&self, ids: &[String], project_id: Option<&str>) {
        let _ = self.0.emit(
            "image://moved",
            serde_json::json!({ "ids": ids, "projectId": project_id }),
        );
    }
}

impl crate::services::assistant::AssistantSink for TauriSink {
    fn delta(&self, request_id: &str, text: &str) {
        let _ = self.0.emit(
            "assistant://delta",
            serde_json::json!({ "requestId": request_id, "text": text }),
        );
    }
    fn suggestion(
        &self,
        request_id: &str,
        index: usize,
        s: &crate::db::assistant::StoredSuggestion,
    ) {
        let _ = self.0.emit(
            "assistant://suggestion",
            serde_json::json!({
                "requestId": request_id,
                "index": index,
                "summary": s.summary,
                "changes": s.changes,
                "model": s.model,
            }),
        );
    }
    fn done(&self, request_id: &str, cancelled: bool) {
        let _ = self.0.emit(
            "assistant://done",
            serde_json::json!({ "requestId": request_id, "cancelled": cancelled }),
        );
    }
    fn error(&self, request_id: &str, error: &crate::error::AppError) {
        let _ = self.0.emit(
            "assistant://error",
            serde_json::json!({
                "requestId": request_id,
                "code": error.code(),
                "message": error.to_string(),
            }),
        );
    }
}

impl crate::services::models::ModelSink for TauriSink {
    fn progress(&self, state: &crate::services::models::DownloadState) {
        let _ = self.0.emit("models://progress", state);
    }
    fn done(&self, state: &crate::services::models::DownloadState) {
        let _ = self.0.emit("models://done", state);
    }
}

impl crate::services::engine_install::InstallSink for TauriSink {
    fn state(&self, state: &crate::services::engine_install::InstallState) {
        let _ = self.0.emit("engine://install-state", state);
    }
    fn log(&self, line: &str) {
        let _ = self
            .0
            .emit("engine://install-log", serde_json::json!({ "line": line }));
    }
}

impl crate::services::llm::LlmSink for TauriSink {
    fn state(&self, state: crate::services::llm::LlmState, message: Option<&str>) {
        let _ = self.0.emit(
            "llm://state",
            serde_json::json!({ "state": state, "message": message }),
        );
    }
    fn install(&self, state: &crate::services::llm::InstallState) {
        let _ = self.0.emit("llm://install-progress", state);
    }
}

/// 翻訳専用モデルの状態・導入の通知(本体の `llm://` とは別のイベント名。docs/34 §9.13)。
pub struct TauriMtSink(pub AppHandle);

impl crate::services::llm::LlmSink for TauriMtSink {
    fn state(&self, state: crate::services::llm::LlmState, message: Option<&str>) {
        let _ = self.0.emit(
            "translate-mt://state",
            serde_json::json!({ "state": state, "message": message }),
        );
    }
    fn install(&self, state: &crate::services::llm::InstallState) {
        let _ = self.0.emit("translate-mt://install-progress", state);
    }
}

impl crate::services::engine_process::ProcSink for TauriSink {
    fn state(&self, state: &crate::services::engine_process::EngineProcState) {
        let _ = self.0.emit("engine://proc-state", state);
    }
    fn log(&self, line: &str) {
        let _ = self
            .0
            .emit("engine://log", serde_json::json!({ "line": line }));
    }
}
