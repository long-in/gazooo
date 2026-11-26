//! アプリ共通エラー。IPC には `{ code, message, details? }` としてシリアライズされる
//! (docs/02-architecture.md §3.7, docs/04-ipc-api.md)。

use serde::{Serialize, Serializer};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{message}")]
    Validation { field: String, message: String },
    #[error("画像生成エンジンに接続できません: {0}")]
    EngineUnreachable(String),
    #[error("画像生成エンジンの応答が想定と異なります: {0}")]
    EngineBadResponse(String),
    #[error("指定したモデルが見つかりません: {0}")]
    EngineModelMissing(String),
    #[error("必要なカスタムノードが見つかりません: {0}")]
    EngineNodeMissing(String),
    #[error("画像生成に失敗しました: {0}")]
    EngineExecutionFailed(String),
    /// 顔の入れ替えの後段合成で、対象画像から顔が検出できなかった(docs/46 §6 E)。
    #[error("対象画像から顔を検出できませんでした")]
    FaceNotDetected,
    #[error("ジョブが見つかりません: {0}")]
    JobNotFound(String),
    #[error("このジョブには実行できない操作です: {0}")]
    JobInvalidState(String),
    #[error("画像が見つかりません: {0}")]
    ImageNotFound(String),
    #[error("プリセットが見つかりません: {0}")]
    PresetNotFound(String),
    #[error("プロジェクトが見つかりません: {0}")]
    ProjectNotFound(String),
    /// 確認時より削除対象が増えている(docs/49 §4)。何も削除しない。
    #[error("確認後に削除対象の画像が増えました。もう一度確認してください")]
    StaleCount,
    /// AI サポート(docs/11)。`code` は `ASSISTANT_*`。
    #[error("{message}")]
    Assistant { code: &'static str, message: String },
    /// モデル取得(docs/12)。`code` は `MODELS_*`。`details` に HTTP ステータスなどを入れる。
    #[error("{message}")]
    Models {
        code: &'static str,
        message: String,
        details: Option<serde_json::Value>,
    },
    /// ComfyUI の起動管理(docs/15)。`code` は `ENGINE_*`(起動・停止の失敗)。
    #[error("{message}")]
    EngineProc {
        code: &'static str,
        message: String,
        details: Option<serde_json::Value>,
    },
    /// バックアップ(docs/35)。`code` は `BACKUP_*`。
    #[error("{message}")]
    Backup { code: &'static str, message: String },
    /// 保存先のデータ移行(docs/51)。`code` は `STORAGE_*`。
    #[error("{message}")]
    Migration { code: &'static str, message: String },
    /// ユーザー機能(docs/36)。`code` は `AUTH_*` / `USER_*`。`details` に `retryAfterMs` などを入れる。
    #[error("{message}")]
    User {
        code: &'static str,
        message: String,
        details: Option<serde_json::Value>,
    },
    #[error("ログインしていません")]
    NotLoggedIn,
    #[error("database error: {0}")]
    Db(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn validation(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Validation {
            field: field.into(),
            message: message.into(),
        }
    }

    pub fn assistant(code: &'static str, message: impl Into<String>) -> Self {
        Self::Assistant {
            code,
            message: message.into(),
        }
    }

    pub fn models(code: &'static str, message: impl Into<String>) -> Self {
        Self::Models {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn models_with(
        code: &'static str,
        message: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self::Models {
            code,
            message: message.into(),
            details: Some(details),
        }
    }

    pub fn backup(code: &'static str, message: impl Into<String>) -> Self {
        Self::Backup {
            code,
            message: message.into(),
        }
    }

    pub fn migration(code: &'static str, message: impl Into<String>) -> Self {
        Self::Migration {
            code,
            message: message.into(),
        }
    }

    pub fn engine_proc(code: &'static str, message: impl Into<String>) -> Self {
        Self::EngineProc {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn user(code: &'static str, message: impl Into<String>) -> Self {
        Self::User {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn user_with(
        code: &'static str,
        message: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self::User {
            code,
            message: message.into(),
            details: Some(details),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    /// フロントエンドが i18n キーに使う安定したコード。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation { .. } => "VALIDATION",
            Self::EngineUnreachable(_) => "ENGINE_UNREACHABLE",
            Self::EngineBadResponse(_) => "ENGINE_BAD_RESPONSE",
            Self::EngineModelMissing(_) => "ENGINE_MODEL_MISSING",
            Self::EngineNodeMissing(_) => "ENGINE_NODE_MISSING",
            Self::EngineExecutionFailed(_) => "ENGINE_EXECUTION_FAILED",
            Self::FaceNotDetected => "FACE_NOT_DETECTED",
            Self::JobNotFound(_) => "JOB_NOT_FOUND",
            Self::JobInvalidState(_) => "JOB_INVALID_STATE",
            Self::ImageNotFound(_) => "IMAGE_NOT_FOUND",
            Self::PresetNotFound(_) => "PRESET_NOT_FOUND",
            Self::ProjectNotFound(_) => "PROJECT_NOT_FOUND",
            Self::StaleCount => "STALE_COUNT",
            Self::Assistant { code, .. } => code,
            Self::Models { code, .. } => code,
            Self::EngineProc { code, .. } => code,
            Self::Backup { code, .. } => code,
            Self::Migration { code, .. } => code,
            Self::User { code, .. } => code,
            Self::NotLoggedIn => "NOT_LOGGED_IN",
            Self::Db(_) => "DB_ERROR",
            Self::Storage(_) => "STORAGE_ERROR",
            Self::Internal(_) => "INTERNAL",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let details = match self {
            Self::Validation { field, .. } => Some(json!({ "field": field })),
            Self::Models { details, .. }
            | Self::EngineProc { details, .. }
            | Self::User { details, .. } => details.clone(),
            _ => None,
        };
        json!({
            "code": self.code(),
            "message": self.to_string(),
            "details": details,
        })
        .serialize(serializer)
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e.to_string())
    }
}

impl From<rusqlite_migration::Error> for AppError {
    fn from(e: rusqlite_migration::Error) -> Self {
        Self::Db(e.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Storage(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::Internal(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_code_message_details() {
        let v = serde_json::to_value(AppError::validation("comfyuiUrl", "invalid")).unwrap();
        assert_eq!(v["code"], "VALIDATION");
        assert_eq!(v["message"], "invalid");
        assert_eq!(v["details"]["field"], "comfyuiUrl");
    }

    #[test]
    fn non_validation_has_null_details() {
        let v = serde_json::to_value(AppError::Db("x".into())).unwrap();
        assert_eq!(v["code"], "DB_ERROR");
        assert!(v["details"].is_null());
    }
}
