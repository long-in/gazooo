use std::sync::Arc;

use tauri::State;

use crate::error::AppResult;
use crate::services::projects::{Project, ProjectDetail, ProjectList, ProjectService};

#[tauri::command]
pub async fn project_list(projects: State<'_, Arc<ProjectService>>) -> AppResult<ProjectList> {
    projects.list().await
}

#[tauri::command]
pub async fn project_get(
    projects: State<'_, Arc<ProjectService>>,
    id: String,
) -> AppResult<ProjectDetail> {
    projects.get(&id).await
}

#[tauri::command]
pub async fn project_create(
    projects: State<'_, Arc<ProjectService>>,
    name: String,
    color: Option<String>,
) -> AppResult<Project> {
    projects.create(name, color).await
}

#[tauri::command]
pub async fn project_update(
    projects: State<'_, Arc<ProjectService>>,
    id: String,
    name: String,
    color: Option<String>,
) -> AppResult<Project> {
    projects.update(id, name, color).await
}

#[tauri::command]
pub async fn project_reorder(
    projects: State<'_, Arc<ProjectService>>,
    ids: Vec<String>,
) -> AppResult<()> {
    projects.reorder(ids).await
}

#[tauri::command]
pub async fn project_delete(
    projects: State<'_, Arc<ProjectService>>,
    id: String,
) -> AppResult<i64> {
    projects.delete(&id).await
}
