//! `projects` テーブルのリポジトリ(docs/40 §3)。

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub sort_order: i64,
    pub last_params_json: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 一覧用(画像数・表紙つき)。
#[derive(Debug, Clone)]
pub struct ProjectSummary {
    pub row: ProjectRow,
    pub image_count: i64,
    pub cover_image_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub total: i64,
    pub favorites: i64,
    pub unassigned: i64,
}

const COLS: &str = "id, name, color, sort_order, last_params_json, created_at, updated_at";

fn map_row(r: &Row) -> rusqlite::Result<ProjectRow> {
    Ok(ProjectRow {
        id: r.get(0)?,
        name: r.get(1)?,
        color: r.get(2)?,
        sort_order: r.get(3)?,
        last_params_json: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Option<ProjectRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM projects WHERE id = ?1"),
            [id],
            map_row,
        )
        .optional()?)
}

pub fn exists(conn: &Connection, id: &str) -> AppResult<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM projects WHERE id = ?1", [id], |_| Ok(()))
        .optional()?
        .is_some())
}

pub fn find_by_name(conn: &Connection, name: &str) -> AppResult<Option<ProjectRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM projects WHERE name = ?1"),
            [name],
            map_row,
        )
        .optional()?)
}

/// 並び順(`sort_order`、同値は作成順)。画像数と最新の画像(表紙)つき。
pub fn list(conn: &Connection) -> AppResult<Vec<ProjectSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS},
                (SELECT COUNT(*) FROM images i WHERE i.project_id = projects.id),
                (SELECT i.id FROM images i WHERE i.project_id = projects.id
                   ORDER BY i.created_at DESC, i.id DESC LIMIT 1)
           FROM projects ORDER BY sort_order ASC, created_at ASC, id ASC"
    ))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ProjectSummary {
                row: map_row(r)?,
                image_count: r.get(7)?,
                cover_image_id: r.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn totals(conn: &Connection) -> AppResult<Totals> {
    Ok(conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(is_favorite = 1), 0),
                COALESCE(SUM(project_id IS NULL), 0)
           FROM images",
        [],
        |r| {
            Ok(Totals {
                total: r.get(0)?,
                favorites: r.get(1)?,
                unassigned: r.get(2)?,
            })
        },
    )?)
}

/// 末尾へ追加する。
pub fn insert(
    conn: &Connection,
    id: &str,
    name: &str,
    color: Option<&str>,
    now: i64,
) -> AppResult<()> {
    let order: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM projects",
        [],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO projects (id, name, color, sort_order, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![id, name, color, order, now],
    )?;
    Ok(())
}

/// 名前と色を更新する。見つからなければ `false`。
pub fn update(
    conn: &Connection,
    id: &str,
    name: &str,
    color: Option<&str>,
    now: i64,
) -> AppResult<bool> {
    let n = conn.execute(
        "UPDATE projects SET name = ?2, color = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, name, color, now],
    )?;
    Ok(n > 0)
}

/// 最後に使った生成設定を保存する(存在しなければ何もしない)。
pub fn set_last_params(conn: &Connection, id: &str, params_json: &str, now: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE projects SET last_params_json = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, params_json, now],
    )?;
    Ok(())
}

/// `ids` の順に `sort_order` を振り直す。現在のプロジェクト全件と過不足なく一致しなければ `VALIDATION`。
pub fn reorder(conn: &mut Connection, ids: &[String]) -> AppResult<()> {
    let tx = conn.transaction()?;
    let mut current: Vec<String> = {
        let mut stmt = tx.prepare("SELECT id FROM projects")?;
        stmt.query_map([], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut given = ids.to_vec();
    current.sort();
    given.sort();
    if current != given {
        return Err(AppError::validation(
            "ids",
            "並び替えにはすべてのプロジェクトを重複なく指定してください",
        ));
    }
    for (i, id) in ids.iter().enumerate() {
        tx.execute(
            "UPDATE projects SET sort_order = ?2 WHERE id = ?1",
            params![id, i as i64],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// 削除する。所属していた画像・ジョブは外部キーの `ON DELETE SET NULL` で未分類へ戻る。
/// 戻り値は `(削除したか, 未分類へ戻った画像数)`。
pub fn delete(conn: &mut Connection, id: &str) -> AppResult<(bool, i64)> {
    let tx = conn.transaction()?;
    let images: i64 = tx.query_row(
        "SELECT COUNT(*) FROM images WHERE project_id = ?1",
        [id],
        |r| r.get(0),
    )?;
    let n = tx.execute("DELETE FROM projects WHERE id = ?1", [id])?;
    tx.commit()?;
    Ok((n > 0, if n > 0 { images } else { 0 }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::db::images::{self as image_repo, ImageFilter, ImageScope};
    use crate::db::jobs::{ImageRow, insert_image, insert_in_project};

    fn job(c: &Connection, id: &str, project: Option<&str>) {
        insert_in_project(c, id, "{}", "a cat", "m", 0, project).unwrap();
    }

    fn image(c: &Connection, job_id: &str, id: &str, at: i64) {
        insert_image(
            c,
            &ImageRow {
                id: id.into(),
                job_id: job_id.into(),
                file_path: format!("a/{id}.png"),
                thumb_path: format!("a/{id}.png"),
                width: 8,
                height: 8,
                seed: 1,
                file_size: 1,
                is_favorite: false,
                created_at: at,
                project_id: None,
            },
        )
        .unwrap();
    }

    fn ids(rows: Vec<ImageRow>) -> Vec<String> {
        rows.into_iter().map(|r| r.id).collect()
    }

    fn scoped(scope: ImageScope, favorites_only: bool) -> ImageFilter {
        ImageFilter {
            scope,
            favorites_only,
            ..Default::default()
        }
    }

    /// p1: i1(10) i2(20★)、p2: i3(30★)、未分類: i4(40)
    fn seed(c: &mut Connection) {
        insert(c, "p1", "キャラA", None, 1).unwrap();
        insert(c, "p2", "背景", Some("#ff0000"), 2).unwrap();
        job(c, "j1", Some("p1"));
        job(c, "j2", Some("p2"));
        job(c, "j3", None);
        image(c, "j1", "i1", 10);
        image(c, "j1", "i2", 20);
        image(c, "j2", "i3", 30);
        image(c, "j3", "i4", 40);
        image_repo::set_favorite(c, "i2", true).unwrap();
        image_repo::set_favorite(c, "i3", true).unwrap();
    }

    #[tokio::test]
    async fn images_inherit_the_job_project_and_scopes_filter() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            seed(c);
            let all = |f: &ImageFilter| image_repo::list(c, f, None, 10);
            assert_eq!(
                ids(all(&scoped(ImageScope::All, false))?),
                ["i4", "i3", "i2", "i1"]
            );
            assert_eq!(
                ids(all(&scoped(ImageScope::Project("p1".into()), false))?),
                ["i2", "i1"]
            );
            assert_eq!(ids(all(&scoped(ImageScope::Unassigned, false))?), ["i4"]);
            // お気に入りは階層なし(全プロジェクト横断)で、プロジェクト内ではそのプロジェクトのみ
            assert_eq!(ids(all(&scoped(ImageScope::All, true))?), ["i3", "i2"]);
            assert_eq!(
                ids(all(&scoped(ImageScope::Project("p2".into()), true))?),
                ["i3"]
            );
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn keyset_paging_within_a_project_has_no_gaps() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            seed(c);
            let f = scoped(ImageScope::Project("p1".into()), false);
            let p1 = image_repo::list(c, &f, None, 1)?;
            assert_eq!(ids(p1.clone()), ["i2"]);
            let last = p1.last().unwrap();
            let p2 = image_repo::list(c, &f, Some((last.created_at, &last.id)), 1)?;
            assert_eq!(ids(p2), ["i1"]);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn list_has_counts_cover_and_totals() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            seed(c);
            let l = list(c)?;
            assert_eq!(
                l.iter().map(|p| p.row.name.as_str()).collect::<Vec<_>>(),
                ["キャラA", "背景"]
            );
            assert_eq!(
                (l[0].image_count, l[0].cover_image_id.as_deref()),
                (2, Some("i2"))
            );
            assert_eq!(
                (l[1].image_count, l[1].cover_image_id.as_deref()),
                (1, Some("i3"))
            );
            assert_eq!(
                totals(c)?,
                Totals {
                    total: 4,
                    favorites: 2,
                    unassigned: 1
                }
            );
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn delete_returns_images_and_jobs_to_unassigned_without_deleting_them() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            seed(c);
            assert_eq!(delete(c, "p1")?, (true, 2));
            assert_eq!(delete(c, "p1")?, (false, 0));
            assert!(get(c, "p1")?.is_none());
            let f = scoped(ImageScope::Unassigned, false);
            assert_eq!(ids(image_repo::list(c, &f, None, 10)?), ["i4", "i2", "i1"]);
            let job_project: Option<String> =
                c.query_row("SELECT project_id FROM jobs WHERE id='j1'", [], |r| {
                    r.get(0)
                })?;
            assert_eq!(job_project, None);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn move_updates_only_images_and_reports_moved_ids() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            seed(c);
            let moved = image_repo::move_to_project(
                c,
                &["i4".into(), "ghost".into(), "i1".into()],
                Some("p2"),
            )?;
            assert_eq!(moved, ["i4", "i1"]);
            let f = scoped(ImageScope::Project("p2".into()), false);
            assert_eq!(ids(image_repo::list(c, &f, None, 10)?), ["i4", "i3", "i1"]);
            // ジョブの出力先は変わらない
            let job_project: Option<String> =
                c.query_row("SELECT project_id FROM jobs WHERE id='j1'", [], |r| {
                    r.get(0)
                })?;
            assert_eq!(job_project.as_deref(), Some("p1"));
            // 未分類へ戻す
            image_repo::move_to_project(c, &["i4".into()], None)?;
            assert_eq!(totals(c)?.unassigned, 1);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn name_is_unique_and_reorder_requires_all_ids() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            seed(c);
            assert!(insert(c, "p3", "キャラA", None, 3).is_err());
            insert(c, "p3", "小物", None, 3)?;
            reorder(c, &["p3".into(), "p1".into(), "p2".into()])?;
            let order: Vec<_> = list(c)?.into_iter().map(|p| p.row.id).collect();
            assert_eq!(order, ["p3", "p1", "p2"]);
            assert!(reorder(c, &["p1".into(), "p2".into()]).is_err());
            assert!(reorder(c, &["p1".into(), "p1".into(), "p2".into()]).is_err());
            assert!(reorder(c, &["p1".into(), "p2".into(), "zzz".into()]).is_err());
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn last_params_roundtrip_and_update() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            seed(c);
            assert!(get(c, "p1")?.unwrap().last_params_json.is_none());
            set_last_params(c, "p1", r#"{"prompt":"x"}"#, 5)?;
            assert_eq!(
                get(c, "p1")?.unwrap().last_params_json.as_deref(),
                Some(r#"{"prompt":"x"}"#)
            );
            assert!(update(c, "p1", "改名", None, 6)?);
            assert!(!update(c, "nope", "x", None, 6)?);
            assert_eq!(find_by_name(c, "改名")?.unwrap().id, "p1");
            Ok(())
        })
        .await
        .unwrap();
    }
}
