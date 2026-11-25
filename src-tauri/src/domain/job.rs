//! ジョブ・画像の IPC 用データ型(docs/04-ipc-api.md「共通型」)。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JobError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub status: JobStatus,
    pub position: i64,
    /// 保存された `GenerationParams`(スキーマ変更に強いよう JSON のまま返す)。
    pub params: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
    pub image_ids: Vec<String>,
    /// 出力先のプロジェクト(`null` = 未分類。docs/40)。
    pub project_id: Option<String>,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageItem {
    pub id: String,
    pub job_id: String,
    pub width: u32,
    pub height: u32,
    pub seed: u64,
    pub is_favorite: bool,
    pub created_at: i64,
    /// 所属プロジェクト(`null` = 未分類。docs/40)。
    pub project_id: Option<String>,
    /// 絶対パス(フロントが `convertFileSrc` で URL に変換する)。
    pub file_path: String,
    pub thumb_path: String,
}

/// 進捗がどの段階のものか(顔補正を有効にしたジョブで「生成中」→「顔補正中」と表示するため)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressPhase {
    Generate,
    FaceDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub job_id: String,
    pub step: u32,
    pub total_steps: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<ProgressPhase>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
