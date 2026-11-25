//! プリセット(生成パラメータの保存・呼び出し)。docs/04-ipc-api.md「プリセット」。

use serde::Serialize;
use serde_json::Value;

use crate::db::Db;
use crate::db::presets::{self as repo, PresetRow};
use crate::domain::params::GenerationParams;
use crate::error::{AppError, AppResult};
use crate::util::now_ms;

pub const MAX_NAME_CHARS: usize = 60;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub params: Value,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<PresetRow> for Preset {
    fn from(r: PresetRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            // スキーマ変更で読めなくなっても一覧は壊さない
            params: serde_json::from_str(&r.params_json).unwrap_or(Value::Null),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

pub struct PresetService {
    db: Db,
}

fn name_conflict() -> AppError {
    AppError::validation("name", "同じ名前のプリセットが既にあります")
}

impl PresetService {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn list(&self) -> AppResult<Vec<Preset>> {
        let rows = self.db.call(|c| repo::list(c)).await?;
        Ok(rows.into_iter().map(Preset::from).collect())
    }

    /// `id` なしは新規作成、ありは更新(上書き・名前変更)。名前は重複不可。
    pub async fn save(&self, id: Option<String>, name: String, params: Value) -> AppResult<Preset> {
        let name = name.trim().to_owned();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(AppError::validation(
                "name",
                format!("名前は 1〜{MAX_NAME_CHARS} 文字で入力してください"),
            ));
        }
        // 形式の検証のみ(プロンプトが空でもスタイル用のプリセットとして保存できる)
        let typed: GenerationParams = serde_json::from_value(params).map_err(|e| {
            AppError::validation("params", format!("生成設定の形式が正しくありません: {e}"))
        })?;
        let params_json = serde_json::to_string(&typed)?;

        self.db
            .call(move |c| {
                let now = now_ms();
                let existing = repo::find_by_name(c, &name)?;
                let row = match id {
                    Some(id) => {
                        if existing.as_ref().is_some_and(|e| e.id != id) {
                            return Err(name_conflict());
                        }
                        if !repo::update(c, &id, &name, &params_json, now)? {
                            return Err(AppError::PresetNotFound(id));
                        }
                        repo::get(c, &id)?.ok_or(AppError::PresetNotFound(id))?
                    }
                    None => {
                        if existing.is_some() {
                            return Err(name_conflict());
                        }
                        let row = PresetRow {
                            id: uuid::Uuid::now_v7().to_string(),
                            name,
                            params_json,
                            created_at: now,
                            updated_at: now,
                        };
                        repo::insert(c, &row)?;
                        row
                    }
                };
                Ok(Preset::from(row))
            })
            .await
    }

    pub async fn delete(&self, id: &str) -> AppResult<()> {
        let owned = id.to_owned();
        if self.db.call(move |c| repo::delete(c, &owned)).await? {
            Ok(())
        } else {
            Err(AppError::PresetNotFound(id.to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::params::{ModelKind, sample};

    fn svc() -> PresetService {
        PresetService::new(Db::open_in_memory().unwrap())
    }

    fn params(prompt: &str) -> Value {
        let mut p = sample(ModelKind::Sdxl);
        p.prompt = prompt.into();
        serde_json::to_value(p).unwrap()
    }

    fn field(e: AppError) -> String {
        match e {
            AppError::Validation { field, .. } => field,
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_list_update_delete() {
        let s = svc();
        let a = s
            .save(None, "  Portrait ".into(), params("face"))
            .await
            .unwrap();
        assert_eq!(a.name, "Portrait", "前後の空白は除く");
        assert_eq!(a.params["prompt"], "face");
        s.save(None, "anime".into(), params("x")).await.unwrap();

        // 名前の昇順(大文字小文字を区別しない)
        let names: Vec<_> = s
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["anime", "Portrait"]);

        // 上書き・名前変更
        let updated = s
            .save(Some(a.id.clone()), "Portrait v2".into(), params("new"))
            .await
            .unwrap();
        assert_eq!(
            (updated.id.as_str(), updated.name.as_str()),
            (a.id.as_str(), "Portrait v2")
        );
        assert_eq!(updated.params["prompt"], "new");
        assert_eq!(updated.created_at, a.created_at);
        assert!(updated.updated_at >= a.updated_at);

        s.delete(&a.id).await.unwrap();
        assert_eq!(s.list().await.unwrap().len(), 1);
        assert_eq!(
            s.delete(&a.id).await.unwrap_err().code(),
            "PRESET_NOT_FOUND"
        );
    }

    #[tokio::test]
    async fn duplicate_names_are_rejected_unless_same_preset() {
        let s = svc();
        let a = s.save(None, "A".into(), params("1")).await.unwrap();
        let b = s.save(None, "B".into(), params("2")).await.unwrap();
        assert_eq!(
            field(s.save(None, "A".into(), params("x")).await.unwrap_err()),
            "name"
        );
        assert_eq!(
            field(
                s.save(Some(b.id.clone()), "A".into(), params("x"))
                    .await
                    .unwrap_err()
            ),
            "name"
        );
        // 自分自身の名前のままの上書きは可
        s.save(Some(a.id.clone()), "A".into(), params("updated"))
            .await
            .unwrap();
        assert_eq!(
            s.list()
                .await
                .unwrap()
                .iter()
                .find(|p| p.id == a.id)
                .unwrap()
                .params["prompt"],
            "updated"
        );
    }

    #[tokio::test]
    async fn validates_name_and_params() {
        let s = svc();
        for bad in ["", "   ", &"あ".repeat(61)] {
            assert_eq!(
                field(s.save(None, bad.into(), params("x")).await.unwrap_err()),
                "name"
            );
        }
        // 60 文字(マルチバイト)ちょうどは可
        s.save(None, "あ".repeat(60), params("x")).await.unwrap();
        assert_eq!(
            field(
                s.save(None, "ok".into(), serde_json::json!({"nope": 1}))
                    .await
                    .unwrap_err()
            ),
            "params"
        );
        assert_eq!(
            s.save(Some("ghost".into()), "n".into(), params("x"))
                .await
                .unwrap_err()
                .code(),
            "PRESET_NOT_FOUND"
        );
        // プロンプトが空でも保存できる
        s.save(None, "style only".into(), params("")).await.unwrap();
    }
}
