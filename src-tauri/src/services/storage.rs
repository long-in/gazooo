//! 画像保存先の切り替え(docs/04-ipc-api.md「保存先の変更」)。
//!
//! ファイルは移さない。データを移すのは `services::migration`(docs/51)。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::AppPaths;
use crate::db::Db;
use crate::db::scopes::{self, Scope};
use crate::error::{AppError, AppResult};
use crate::services::image_locations as locations;
use crate::services::settings::{self, SettingsPatch};

pub struct StorageService {
    db: Db,
    paths: AppPaths,
}

fn storage_err(path: &Path, e: impl std::fmt::Display) -> AppError {
    AppError::Storage(format!("{}: {e}", path.display()))
}

impl StorageService {
    pub fn new(db: Db, paths: AppPaths) -> Arc<Self> {
        Arc::new(Self { db, paths })
    }

    /// 設定だけを切り替え、ログイン中のユーザーが使う場所を返す。
    /// `new_dir = None` は、個別なら全体設定に戻し、全体なら標準値に戻す。
    pub async fn change_images_dir(
        &self,
        scope: Scope,
        new_dir: Option<String>,
    ) -> AppResult<PathBuf> {
        if let Some(dir) = &new_dir
            && !Path::new(dir).is_absolute()
        {
            return Err(AppError::validation(
                "newDir",
                "保存先は絶対パスで指定してください",
            ));
        }

        let running: i64 = self
            .db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM jobs WHERE status = 'running'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await?;
        if running > 0 {
            return Err(AppError::JobInvalidState(
                "生成中は保存先を変更できません。完了またはキャンセルしてから変更してください"
                    .into(),
            ));
        }

        if let Some(dir) = &new_dir {
            let all = locations::all(&self.db, &self.paths).await?;
            let dest = PathBuf::from(dir);
            let data_dir = self.paths.data_dir.clone();
            let me = self.paths.current_user_id();
            tokio::task::spawn_blocking(move || -> AppResult<()> {
                let rejected = match scope {
                    Scope::Global => locations::check_root(&data_dir, &all, &dest),
                    Scope::User => {
                        locations::check_individual(&data_dir, &all, me.as_deref(), &dest)
                    }
                };
                if let Some(r) = rejected {
                    return Err(r.error());
                }
                // 作成できて書き込めることを先に確認する
                std::fs::create_dir_all(&dest).map_err(|e| storage_err(&dest, e))?;
                let probe = dest.join(format!(".gazooo-write-test-{}", uuid::Uuid::now_v7()));
                std::fs::write(&probe, b"").map_err(|_| {
                    AppError::validation(
                        "newDir",
                        "この場所には書き込めません。別の場所を選んでください",
                    )
                })?;
                let _ = std::fs::remove_file(&probe);
                Ok(())
            })
            .await
            .map_err(|e| AppError::internal(e.to_string()))??;
        }

        match scope {
            Scope::Global => {
                self.db
                    .call_device(move |c| {
                        let value = new_dir.map(serde_json::Value::String);
                        scopes::write(c, Scope::Global, scopes::KEY_IMAGES_ROOT, value.as_ref())
                    })
                    .await?;
            }
            Scope::User => {
                let patch = SettingsPatch {
                    images_dir: Some(new_dir),
                    ..SettingsPatch::default()
                };
                self.db
                    .call(move |c| settings::update(c, patch).map(|_| ()))
                    .await?;
            }
        }

        let current = self.db.call(|c| settings::load(c)).await?;
        let root = self.paths.resolve_images_dir(&current);
        let dir = root.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&dir).map_err(|e| storage_err(&dir, e))
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;
        Ok(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::jobs::insert;

    fn harness() -> (Arc<StorageService>, Db, PathBuf) {
        let dir = std::env::temp_dir().join(format!("gazooo-storage-{}", uuid::Uuid::now_v7()));
        let db = Db::open_in_memory().unwrap();
        let svc = StorageService::new(db.clone(), AppPaths::new(dir.join("appdata")));
        (svc, db, dir)
    }

    async fn setting(db: &Db) -> Option<String> {
        db.call(|c| settings::load(c)).await.unwrap().images_dir
    }

    #[tokio::test]
    async fn only_the_setting_changes_and_none_restores_default() {
        let (svc, db, dir) = harness();
        let target = dir.join("elsewhere").to_string_lossy().into_owned();
        svc.change_images_dir(Scope::User, Some(target.clone()))
            .await
            .unwrap();
        assert!(Path::new(&target).is_dir(), "新しい保存先は作成される");
        assert_eq!(setting(&db).await, Some(target));
        svc.change_images_dir(Scope::User, None).await.unwrap();
        assert_eq!(setting(&db).await, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn rejects_relative_paths_and_running_jobs() {
        let (svc, db, dir) = harness();
        let err = svc
            .change_images_dir(Scope::User, Some("relative/dir".into()))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
        db.call(|c| {
            insert(c, "j", "{}", "p", "m", 0)?;
            crate::db::jobs::mark_running(c, "j", 1).map(|_| ())
        })
        .await
        .unwrap();
        let target = dir.join("x").to_string_lossy().into_owned();
        let err = svc
            .change_images_dir(Scope::User, Some(target))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "JOB_INVALID_STATE");
        assert_eq!(setting(&db).await, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
