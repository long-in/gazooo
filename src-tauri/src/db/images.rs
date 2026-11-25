//! `images` テーブルの検索・更新(docs/03-data-model.md)。挿入は `jobs.rs` の `insert_image`。

use rusqlite::{Connection, OptionalExtension, Row, params_from_iter, types::Value};

use super::jobs::ImageRow;
use crate::error::AppResult;

const COLS: &str = "images.id, images.job_id, images.file_path, images.thumb_path, images.width, images.height, \
                    images.seed, images.file_size, images.is_favorite, images.created_at, images.project_id";

fn map_image(r: &Row) -> rusqlite::Result<ImageRow> {
    Ok(ImageRow {
        id: r.get(0)?,
        job_id: r.get(1)?,
        file_path: r.get(2)?,
        thumb_path: r.get(3)?,
        width: r.get(4)?,
        height: r.get(5)?,
        seed: r.get::<_, i64>(6)? as u64,
        file_size: r.get::<_, i64>(7)? as u64,
        is_favorite: r.get(8)?,
        created_at: r.get(9)?,
        project_id: r.get(10)?,
    })
}

/// 一覧の範囲(docs/40 §5.1)。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum ImageScope {
    /// プロジェクトに関係なくすべて。
    #[default]
    All,
    /// どのプロジェクトにも属さない画像。
    Unassigned,
    Project(String),
}

#[derive(Debug, Default, Clone)]
pub struct ImageFilter {
    pub scope: ImageScope,
    /// プロンプトの部分一致。
    pub query: Option<String>,
    pub favorites_only: bool,
}

/// `LIKE` のワイルドカードをエスケープする(エスケープ文字は `\`)。
fn like_pattern(q: &str) -> String {
    let mut out = String::with_capacity(q.len() + 2);
    out.push('%');
    for c in q.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// 新しい順(`created_at`、同値は `id` の降順)。`after` を渡すとその次から返す(キーセット方式)。
pub fn list(
    conn: &Connection,
    filter: &ImageFilter,
    after: Option<(i64, &str)>,
    limit: i64,
) -> AppResult<Vec<ImageRow>> {
    let mut sql =
        format!("SELECT {COLS} FROM images JOIN jobs ON jobs.id = images.job_id WHERE 1 = 1");
    let mut args: Vec<Value> = Vec::new();
    match &filter.scope {
        ImageScope::All => {}
        ImageScope::Unassigned => sql.push_str(" AND images.project_id IS NULL"),
        ImageScope::Project(id) => {
            sql.push_str(" AND images.project_id = ?");
            args.push(Value::Text(id.clone()));
        }
    }
    if filter.favorites_only {
        sql.push_str(" AND images.is_favorite = 1");
    }
    if let Some(q) = filter
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
    {
        sql.push_str(" AND jobs.prompt_text LIKE ? ESCAPE '\\'");
        args.push(Value::Text(like_pattern(q)));
    }
    if let Some((created_at, id)) = after {
        sql.push_str(" AND (images.created_at, images.id) < (?, ?)");
        args.push(Value::Integer(created_at));
        args.push(Value::Text(id.to_owned()));
    }
    sql.push_str(" ORDER BY images.created_at DESC, images.id DESC LIMIT ?");
    args.push(Value::Integer(limit));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(args), map_image)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Option<ImageRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM images WHERE id = ?1"),
            [id],
            map_image,
        )
        .optional()?)
}

pub fn get_many(conn: &Connection, ids: &[String]) -> AppResult<Vec<ImageRow>> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(row) = get(conn, id)? {
            out.push(row);
        }
    }
    Ok(out)
}

/// 見つからなければ `false`。
pub fn set_favorite(conn: &Connection, id: &str, value: bool) -> AppResult<bool> {
    let n = conn.execute(
        "UPDATE images SET is_favorite = ?2 WHERE id = ?1",
        rusqlite::params![id, value],
    )?;
    Ok(n > 0)
}

/// 画像の所属を変える(`None` = 未分類)。実際に更新した画像の ID を返す。ファイルは動かさない。
pub fn move_to_project(
    conn: &mut Connection,
    ids: &[String],
    project_id: Option<&str>,
) -> AppResult<Vec<String>> {
    let tx = conn.transaction()?;
    let mut moved = Vec::with_capacity(ids.len());
    for id in ids {
        let n = tx.execute(
            "UPDATE images SET project_id = ?2 WHERE id = ?1",
            rusqlite::params![id, project_id],
        )?;
        if n > 0 {
            moved.push(id.clone());
        }
    }
    tx.commit()?;
    Ok(moved)
}

pub fn delete(conn: &mut Connection, ids: &[String]) -> AppResult<usize> {
    let tx = conn.transaction()?;
    let mut n = 0;
    for id in ids {
        n += tx.execute("DELETE FROM images WHERE id = ?1", [id])?;
    }
    tx.commit()?;
    Ok(n)
}

/// すべての画像(保存先の移動で、全ファイルを列挙するために使う)。
pub fn all(conn: &Connection) -> AppResult<Vec<ImageRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM images ORDER BY created_at ASC"
    ))?;
    let rows = stmt
        .query_map([], map_image)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// サムネイル未生成(`thumb_path` が原寸と同じ)の画像。
pub fn needing_thumbs(conn: &Connection) -> AppResult<Vec<ImageRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM images WHERE thumb_path = file_path ORDER BY created_at ASC"
    ))?;
    let rows = stmt
        .query_map([], map_image)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn set_thumb_path(conn: &Connection, id: &str, path: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE images SET thumb_path = ?2 WHERE id = ?1",
        rusqlite::params![id, path],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::db::jobs::{insert, insert_image};

    fn add_image(conn: &Connection, job: &str, id: &str, created_at: i64) {
        insert_image(
            conn,
            &ImageRow {
                id: id.into(),
                job_id: job.into(),
                file_path: format!("a/{id}.png"),
                thumb_path: format!("a/{id}.png"),
                width: 8,
                height: 8,
                seed: 1,
                file_size: 1,
                is_favorite: false,
                created_at,
                project_id: None,
            },
        )
        .unwrap();
    }

    fn seed(conn: &Connection) {
        insert(conn, "j1", "{}", "a red cat", "m", 0).unwrap();
        insert(conn, "j2", "{}", "100% sure_thing", "m", 0).unwrap();
        add_image(conn, "j1", "i1", 10);
        add_image(conn, "j1", "i2", 20);
        add_image(conn, "j2", "i3", 20); // i2 と同時刻 → id 降順
        add_image(conn, "j2", "i4", 30);
    }

    fn ids(rows: Vec<ImageRow>) -> Vec<String> {
        rows.into_iter().map(|r| r.id).collect()
    }

    #[tokio::test]
    async fn orders_newest_first_with_id_tiebreak() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            seed(c);
            assert_eq!(
                ids(list(c, &ImageFilter::default(), None, 10)?),
                ["i4", "i3", "i2", "i1"]
            );
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn keyset_paging_has_no_duplicates_or_gaps_even_with_inserts() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            seed(c);
            let f = ImageFilter::default();
            let p1 = list(c, &f, None, 2)?;
            assert_eq!(ids(p1.clone()), ["i4", "i3"]);
            // ページ間に新しい画像が追加されても、続きは重複・欠落しない
            add_image(c, "j1", "i5", 99);
            let last = p1.last().unwrap();
            let p2 = list(c, &f, Some((last.created_at, &last.id)), 2)?;
            assert_eq!(ids(p2), ["i2", "i1"]);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn search_matches_prompt_and_escapes_wildcards() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            seed(c);
            let q = |s: &str| ImageFilter {
                query: Some(s.into()),
                ..Default::default()
            };
            assert_eq!(ids(list(c, &q("RED"), None, 10)?), ["i2", "i1"]); // 大文字小文字を区別しない
            assert_eq!(ids(list(c, &q("100%"), None, 10)?), ["i4", "i3"]);
            assert!(
                list(c, &q("%"), None, 10)?.len() == 2,
                "% はワイルドカードではなく文字として扱う"
            );
            assert_eq!(list(c, &q("_"), None, 10)?.len(), 2, "_ も文字として扱う");
            assert!(list(c, &q("zzz"), None, 10)?.is_empty());
            assert_eq!(list(c, &q("   "), None, 10)?.len(), 4, "空白のみは検索なし");
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn favorites_filter_and_toggle() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            seed(c);
            assert!(set_favorite(c, "i2", true)?);
            assert!(!set_favorite(c, "nope", true)?);
            let fav = ImageFilter {
                favorites_only: true,
                ..Default::default()
            };
            assert_eq!(ids(list(c, &fav, None, 10)?), ["i2"]);
            set_favorite(c, "i2", false)?;
            assert!(list(c, &fav, None, 10)?.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn delete_and_thumb_backfill_queries() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            seed(c);
            assert_eq!(needing_thumbs(c)?.len(), 4);
            set_thumb_path(c, "i1", "thumbs/i1.jpg")?;
            assert_eq!(needing_thumbs(c)?.len(), 3);

            assert_eq!(delete(c, &["i1".into(), "i2".into(), "ghost".into()])?, 2);
            assert_eq!(get_many(c, &["i1".into(), "i3".into()])?.len(), 1);
            assert!(get(c, "i2")?.is_none());
            Ok(())
        })
        .await
        .unwrap();
    }
}
