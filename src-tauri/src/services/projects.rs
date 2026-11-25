//! プロジェクト(作業フォルダ。docs/40)。画像と「最後に使った生成設定」の入れ物。
//! 画像の所属の変更(移動)は `ImageService::move_to_project`。

use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::db::Db;
use crate::db::projects::{self as repo, ProjectSummary};
use crate::error::{AppError, AppResult};
use crate::services::events::EventSink;
use crate::util::now_ms;

pub const MAX_NAME_CHARS: usize = 60;
/// 色のパレット(ユーザーカラーと同じ 8 色。docs/36 §7)。
pub const PALETTE: [&str; 8] = [
    "red", "orange", "amber", "green", "teal", "blue", "violet", "pink",
];

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub sort_order: i64,
    pub image_count: i64,
    pub cover_image_id: Option<String>,
    pub has_last_params: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<ProjectSummary> for Project {
    fn from(s: ProjectSummary) -> Self {
        Self {
            id: s.row.id,
            name: s.row.name,
            color: s.row.color,
            sort_order: s.row.sort_order,
            image_count: s.image_count,
            cover_image_id: s.cover_image_id,
            has_last_params: s.row.last_params_json.is_some(),
            created_at: s.row.created_at,
            updated_at: s.row.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectList {
    pub projects: Vec<Project>,
    pub total_images: i64,
    pub favorite_images: i64,
    pub unassigned_images: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetail {
    pub project: Project,
    /// 最後にこのプロジェクトで投入した生成設定(無ければ `null`)。
    pub last_params: Option<Value>,
}

pub struct ProjectService {
    db: Db,
    sink: Arc<dyn EventSink>,
}

fn check_name(name: &str) -> AppResult<String> {
    let name = name.trim().to_owned();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::validation(
            "name",
            format!("名前は 1〜{MAX_NAME_CHARS} 文字で入力してください"),
        ));
    }
    Ok(name)
}

fn check_color(color: Option<String>) -> AppResult<Option<String>> {
    match color {
        Some(c) if !PALETTE.contains(&c.as_str()) => {
            Err(AppError::validation("color", "色の指定が正しくありません"))
        }
        other => Ok(other),
    }
}

fn name_conflict() -> AppError {
    AppError::validation("name", "同じ名前のプロジェクトが既にあります")
}

impl ProjectService {
    pub fn new(db: Db, sink: Arc<dyn EventSink>) -> Arc<Self> {
        Arc::new(Self { db, sink })
    }

    pub async fn list(&self) -> AppResult<ProjectList> {
        self.db
            .call(|c| {
                let projects = repo::list(c)?.into_iter().map(Project::from).collect();
                let t = repo::totals(c)?;
                Ok(ProjectList {
                    projects,
                    total_images: t.total,
                    favorite_images: t.favorites,
                    unassigned_images: t.unassigned,
                })
            })
            .await
    }

    pub async fn get(&self, id: &str) -> AppResult<ProjectDetail> {
        let owned = id.to_owned();
        self.db
            .call(move |c| {
                let row = repo::get(c, &owned)?.ok_or(AppError::ProjectNotFound(owned.clone()))?;
                let last_params = row
                    .last_params_json
                    .as_deref()
                    // スキーマ変更で読めなくなっても、プロジェクトの表示は壊さない
                    .and_then(|j| serde_json::from_str::<Value>(j).ok());
                let summary = repo::list(c)?
                    .into_iter()
                    .find(|p| p.row.id == owned)
                    .ok_or(AppError::ProjectNotFound(owned))?;
                Ok(ProjectDetail {
                    project: summary.into(),
                    last_params,
                })
            })
            .await
    }

    pub async fn create(&self, name: String, color: Option<String>) -> AppResult<Project> {
        let name = check_name(&name)?;
        let color = check_color(color)?;
        let project = self
            .db
            .call(move |c| {
                if repo::find_by_name(c, &name)?.is_some() {
                    return Err(name_conflict());
                }
                let id = uuid::Uuid::now_v7().to_string();
                repo::insert(c, &id, &name, color.as_deref(), now_ms())?;
                let summary = repo::list(c)?
                    .into_iter()
                    .find(|p| p.row.id == id)
                    .ok_or_else(|| AppError::internal("project vanished"))?;
                Ok(Project::from(summary))
            })
            .await?;
        self.sink.project_changed();
        Ok(project)
    }

    /// 名前と色を置き換える(色 `None` = なし)。
    pub async fn update(
        &self,
        id: String,
        name: String,
        color: Option<String>,
    ) -> AppResult<Project> {
        let name = check_name(&name)?;
        let color = check_color(color)?;
        let project = self
            .db
            .call(move |c| {
                if repo::find_by_name(c, &name)?.is_some_and(|e| e.id != id) {
                    return Err(name_conflict());
                }
                if !repo::update(c, &id, &name, color.as_deref(), now_ms())? {
                    return Err(AppError::ProjectNotFound(id));
                }
                let summary = repo::list(c)?
                    .into_iter()
                    .find(|p| p.row.id == id)
                    .ok_or(AppError::ProjectNotFound(id))?;
                Ok(Project::from(summary))
            })
            .await?;
        self.sink.project_changed();
        Ok(project)
    }

    pub async fn reorder(&self, ids: Vec<String>) -> AppResult<()> {
        self.db.call(move |c| repo::reorder(c, &ids)).await?;
        self.sink.project_changed();
        Ok(())
    }

    /// 削除する。画像・ジョブは未分類へ戻る(消えない)。未分類へ戻った画像数を返す。
    pub async fn delete(&self, id: &str) -> AppResult<i64> {
        let owned = id.to_owned();
        let (found, images) = self.db.call(move |c| repo::delete(c, &owned)).await?;
        if !found {
            return Err(AppError::ProjectNotFound(id.to_owned()));
        }
        self.sink.project_changed();
        Ok(images)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::job::{ImageItem, Job, ProgressEvent};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder {
        changed: Mutex<u32>,
    }
    impl EventSink for Recorder {
        fn job_updated(&self, _: &Job) {}
        fn job_progress(&self, _: &ProgressEvent) {}
        fn image_created(&self, _: &ImageItem) {}
        fn image_deleted(&self, _: &[String]) {}
        fn project_changed(&self) {
            *self.changed.lock().unwrap() += 1;
        }
    }

    fn svc() -> (Arc<ProjectService>, Arc<Recorder>) {
        let rec = Arc::new(Recorder::default());
        let svc = ProjectService::new(Db::open_in_memory().unwrap(), rec.clone());
        (svc, rec)
    }

    #[tokio::test]
    async fn create_validates_name_and_color_and_rejects_duplicates() {
        let (svc, rec) = svc();
        let a = svc
            .create("  キャラA ".into(), Some("blue".into()))
            .await
            .unwrap();
        assert_eq!(a.name, "キャラA");
        assert_eq!(a.color.as_deref(), Some("blue"));
        assert!(!a.has_last_params);

        for bad in ["", "   ", &"あ".repeat(61)] {
            assert_eq!(
                svc.create(bad.into(), None).await.unwrap_err().code(),
                "VALIDATION"
            );
        }
        assert_eq!(
            svc.create("x".into(), Some("#ff0000".into()))
                .await
                .unwrap_err()
                .code(),
            "VALIDATION"
        );
        assert_eq!(
            svc.create("キャラA".into(), None).await.unwrap_err().code(),
            "VALIDATION"
        );
        assert_eq!(*rec.changed.lock().unwrap(), 1, "失敗では通知しない");
    }

    #[tokio::test]
    async fn update_renames_and_clears_color() {
        let (svc, _) = svc();
        let a = svc.create("A".into(), Some("red".into())).await.unwrap();
        let b = svc.create("B".into(), None).await.unwrap();
        let a2 = svc.update(a.id.clone(), "A2".into(), None).await.unwrap();
        assert_eq!((a2.name.as_str(), a2.color), ("A2", None));
        // 自分自身と同じ名前は可。他と同じ名前は不可
        svc.update(a.id.clone(), "A2".into(), None).await.unwrap();
        assert_eq!(
            svc.update(a.id, "B".into(), None).await.unwrap_err().code(),
            "VALIDATION"
        );
        assert_eq!(
            svc.update("nope".into(), "Z".into(), None)
                .await
                .unwrap_err()
                .code(),
            "PROJECT_NOT_FOUND"
        );
        let _ = b;
    }

    #[tokio::test]
    async fn list_get_reorder_delete() {
        let (svc, rec) = svc();
        let a = svc.create("A".into(), None).await.unwrap();
        let b = svc.create("B".into(), None).await.unwrap();
        svc.reorder(vec![b.id.clone(), a.id.clone()]).await.unwrap();
        let l = svc.list().await.unwrap();
        assert_eq!(
            l.projects
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["B", "A"]
        );
        assert_eq!(
            (l.total_images, l.favorite_images, l.unassigned_images),
            (0, 0, 0)
        );

        let d = svc.get(&a.id).await.unwrap();
        assert!(d.last_params.is_none());
        assert_eq!(
            svc.get("nope").await.unwrap_err().code(),
            "PROJECT_NOT_FOUND"
        );

        assert_eq!(svc.delete(&a.id).await.unwrap(), 0);
        assert_eq!(
            svc.delete(&a.id).await.unwrap_err().code(),
            "PROJECT_NOT_FOUND"
        );
        assert_eq!(svc.list().await.unwrap().projects.len(), 1);
        assert_eq!(*rec.changed.lock().unwrap(), 4);
    }
}
