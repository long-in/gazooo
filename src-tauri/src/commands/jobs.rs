use std::sync::Arc;

use tauri::State;

use crate::domain::job::{Job, JobStatus, Page};
use crate::domain::params::GenerationParams;
use crate::error::AppResult;
use crate::services::jobs::JobService;

#[tauri::command]
pub async fn job_enqueue(
    jobs: State<'_, Arc<JobService>>,
    params: GenerationParams,
    count: Option<u32>,
    project_id: Option<String>,
) -> AppResult<Vec<Job>> {
    jobs.enqueue_in(params, count.unwrap_or(1), project_id)
        .await
}

#[tauri::command]
pub async fn job_list(
    jobs: State<'_, Arc<JobService>>,
    statuses: Option<Vec<JobStatus>>,
    cursor: Option<String>,
    limit: Option<u32>,
) -> AppResult<Page<Job>> {
    jobs.list(statuses.unwrap_or_default(), cursor, limit).await
}

#[tauri::command]
pub async fn job_cancel(jobs: State<'_, Arc<JobService>>, id: String) -> AppResult<()> {
    jobs.cancel(&id).await
}

#[tauri::command]
pub async fn job_reorder(
    jobs: State<'_, Arc<JobService>>,
    id: String,
    new_position: usize,
) -> AppResult<()> {
    jobs.reorder(&id, new_position).await
}

#[tauri::command]
pub async fn job_retry(jobs: State<'_, Arc<JobService>>, id: String) -> AppResult<Job> {
    jobs.retry(&id).await
}

#[tauri::command]
pub async fn job_remove(jobs: State<'_, Arc<JobService>>, ids: Vec<String>) -> AppResult<usize> {
    jobs.remove(ids).await
}

#[tauri::command]
pub fn queue_pause(jobs: State<'_, Arc<JobService>>) {
    jobs.pause();
}

#[tauri::command]
pub fn queue_resume(jobs: State<'_, Arc<JobService>>) {
    jobs.resume();
}

#[tauri::command]
pub fn queue_state(jobs: State<'_, Arc<JobService>>) -> QueueState {
    QueueState {
        paused: jobs.is_paused(),
    }
}

/// キューの状態(docs/20 §3.4)。
#[derive(serde::Serialize)]
pub struct QueueState {
    pub paused: bool,
}

#[tauri::command]
pub async fn job_clear_finished(jobs: State<'_, Arc<JobService>>) -> AppResult<usize> {
    jobs.clear_finished().await
}
