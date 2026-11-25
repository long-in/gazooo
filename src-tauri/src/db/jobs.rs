//! `jobs` / `images` テーブルのリポジトリ(docs/03-data-model.md)。

use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};

use crate::domain::job::JobStatus;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct JobRow {
    pub id: String,
    pub status: JobStatus,
    pub position: i64,
    pub params_json: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    /// 出力先のプロジェクト(`None` = 未分類。docs/40)。
    pub project_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImageRow {
    pub id: String,
    pub job_id: String,
    /// 画像ルートからの相対パス(`/` 区切り)。
    pub file_path: String,
    pub thumb_path: String,
    pub width: u32,
    pub height: u32,
    pub seed: u64,
    pub file_size: u64,
    pub is_favorite: bool,
    pub created_at: i64,
    /// 所属プロジェクト(`None` = 未分類。docs/40)。
    pub project_id: Option<String>,
}

const JOB_COLS: &str = "id, status, position, params_json, error_code, error_message, created_at, started_at, finished_at, project_id";

fn map_job(r: &Row) -> rusqlite::Result<JobRow> {
    let status: String = r.get(1)?;
    Ok(JobRow {
        id: r.get(0)?,
        status: JobStatus::parse(&status).unwrap_or(JobStatus::Failed),
        position: r.get(2)?,
        params_json: r.get(3)?,
        error_code: r.get(4)?,
        error_message: r.get(5)?,
        created_at: r.get(6)?,
        started_at: r.get(7)?,
        finished_at: r.get(8)?,
        project_id: r.get(9)?,
    })
}

/// ジョブを待機状態で追加し、割り当てた並び順を返す(未分類。テスト用の短縮形)。
#[cfg(test)]
pub fn insert(
    conn: &Connection,
    id: &str,
    params_json: &str,
    prompt: &str,
    model_name: &str,
    now: i64,
) -> AppResult<i64> {
    insert_in_project(conn, id, params_json, prompt, model_name, now, None)
}

/// プロジェクトを指定してジョブを追加する(`None` = 未分類)。
pub fn insert_in_project(
    conn: &Connection,
    id: &str,
    params_json: &str,
    prompt: &str,
    model_name: &str,
    now: i64,
    project_id: Option<&str>,
) -> AppResult<i64> {
    let position: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM jobs",
        [],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO jobs (id, status, position, engine, params_json, prompt_text, model_name, created_at, project_id)
         VALUES (?1, 'queued', ?2, 'comfyui', ?3, ?4, ?5, ?6, ?7)",
        params![id, position, params_json, prompt, model_name, now, project_id],
    )?;
    Ok(position)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Option<JobRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {JOB_COLS} FROM jobs WHERE id = ?1"),
            [id],
            map_job,
        )
        .optional()?)
}

/// 実行中 → 待機(並び順)→ 終了(新しい順)の順で返す。非表示のジョブは除く。
pub fn list(
    conn: &Connection,
    statuses: &[JobStatus],
    limit: i64,
    offset: i64,
) -> AppResult<Vec<JobRow>> {
    let mut sql = format!("SELECT {JOB_COLS} FROM jobs WHERE queue_hidden = 0");
    if !statuses.is_empty() {
        let marks = vec!["?"; statuses.len()].join(",");
        sql.push_str(&format!(" AND status IN ({marks})"));
    }
    sql.push_str(
        " ORDER BY CASE status WHEN 'running' THEN 0 WHEN 'queued' THEN 1 ELSE 2 END,
                   CASE WHEN status = 'queued' THEN position ELSE 0 END ASC,
                   created_at DESC, rowid DESC
          LIMIT ? OFFSET ?",
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut args: Vec<rusqlite::types::Value> = statuses
        .iter()
        .map(|s| rusqlite::types::Value::Text(s.as_str().to_owned()))
        .collect();
    args.push(limit.into());
    args.push(offset.into());
    let rows = stmt
        .query_map(params_from_iter(args), map_job)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn next_queued(conn: &Connection) -> AppResult<Option<JobRow>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {JOB_COLS} FROM jobs WHERE status = 'queued' ORDER BY position ASC LIMIT 1"
            ),
            [],
            map_job,
        )
        .optional()?)
}

/// 待機中のジョブを実行中にする。取消などで既に待機中でなければ `false`(実行してはならない)。
pub fn mark_running(conn: &Connection, id: &str, now: i64) -> AppResult<bool> {
    let n = conn.execute(
        "UPDATE jobs SET status = 'running', started_at = ?2 WHERE id = ?1 AND status = 'queued'",
        params![id, now],
    )?;
    Ok(n > 0)
}

/// 実行中のジョブを、待機の先頭へ戻す(接続断からの自動復旧。docs/47 §4.3)。実行中でなければ `false`。
pub fn requeue_front(conn: &Connection, id: &str) -> AppResult<bool> {
    let n = conn.execute(
        "UPDATE jobs SET status = 'queued', started_at = NULL, remote_id = NULL,
                position = (SELECT COALESCE(MIN(position), 0) - 1 FROM jobs WHERE status = 'queued')
         WHERE id = ?1 AND status = 'running'",
        [id],
    )?;
    Ok(n > 0)
}

/// 待機中のジョブを取り消す。ワーカーが先に実行開始していた場合は `false`。
pub fn cancel_queued(conn: &Connection, id: &str, now: i64) -> AppResult<bool> {
    let n = conn.execute(
        "UPDATE jobs SET status = 'cancelled', finished_at = ?2 WHERE id = ?1 AND status = 'queued'",
        params![id, now],
    )?;
    Ok(n > 0)
}

pub fn set_remote_id(conn: &Connection, id: &str, remote_id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE jobs SET remote_id = ?2 WHERE id = ?1",
        params![id, remote_id],
    )?;
    Ok(())
}

pub fn finish(
    conn: &Connection,
    id: &str,
    status: JobStatus,
    error: Option<(&str, &str)>,
    now: i64,
) -> AppResult<()> {
    conn.execute(
        "UPDATE jobs SET status = ?2, error_code = ?3, error_message = ?4, finished_at = ?5 WHERE id = ?1",
        params![id, status.as_str(), error.map(|e| e.0), error.map(|e| e.1), now],
    )?;
    Ok(())
}

/// 起動時: 前回の実行中ジョブ(アプリ終了で中断されたもの)を失敗扱いにする。
pub fn fail_interrupted(conn: &Connection, now: i64) -> AppResult<usize> {
    Ok(conn.execute(
        "UPDATE jobs SET status = 'failed', error_code = 'INTERRUPTED',
                error_message = 'アプリの終了により中断されました', finished_at = ?1
         WHERE status = 'running'",
        [now],
    )?)
}

/// 待機中ジョブを `new_index`(0 始まり)の位置へ移動する。
pub fn reorder(conn: &mut Connection, id: &str, new_index: usize) -> AppResult<()> {
    let tx = conn.transaction()?;
    let status: Option<String> = tx
        .query_row("SELECT status FROM jobs WHERE id = ?1", [id], |r| r.get(0))
        .optional()?;
    match status.as_deref() {
        None => return Err(AppError::JobNotFound(id.to_owned())),
        Some("queued") => {}
        Some(_) => {
            return Err(AppError::JobInvalidState(
                "待機中のジョブのみ並べ替えできます".into(),
            ));
        }
    }

    let (mut ids, positions): (Vec<String>, Vec<i64>) = {
        let mut stmt = tx.prepare(
            "SELECT id, position FROM jobs WHERE status = 'queued' ORDER BY position ASC",
        )?;
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .unzip()
    };
    let from = ids.iter().position(|x| x == id).unwrap_or(0);
    let moved = ids.remove(from);
    ids.insert(new_index.min(ids.len()), moved);
    // 既存の position の値の集合をそのまま再利用して、新しい順序に割り当てる
    for (id, pos) in ids.iter().zip(positions) {
        tx.execute(
            "UPDATE jobs SET position = ?2 WHERE id = ?1",
            params![id, pos],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// 終了済みジョブの片付け。画像を持たないものは削除、持つものはキュー画面から隠す。
pub fn clear_finished(conn: &mut Connection) -> AppResult<usize> {
    let tx = conn.transaction()?;
    let deleted = tx.execute(
        "DELETE FROM jobs WHERE status IN ('completed','failed','cancelled')
           AND NOT EXISTS (SELECT 1 FROM images WHERE images.job_id = jobs.id)",
        [],
    )?;
    let hidden = tx.execute(
        "UPDATE jobs SET queue_hidden = 1
         WHERE status IN ('completed','failed','cancelled') AND queue_hidden = 0",
        [],
    )?;
    tx.commit()?;
    Ok(deleted + hidden)
}

/// 指定した終了済みジョブを履歴から外す。画像を持たないものは削除、持つものはキュー画面から隠す。
/// 実行中・待機中のジョブ、存在しない ID が含まれていたら何もせず `VALIDATION`。
pub fn remove_finished(conn: &mut Connection, ids: &[String]) -> AppResult<usize> {
    let tx = conn.transaction()?;
    for id in ids {
        let status: Option<String> = tx
            .query_row("SELECT status FROM jobs WHERE id = ?1", [id], |r| r.get(0))
            .optional()?;
        match status.as_deref() {
            Some("completed" | "failed" | "cancelled") => {}
            _ => {
                return Err(AppError::validation(
                    "ids",
                    "実行中・待機中のジョブや、存在しないジョブは履歴から外せません",
                ));
            }
        }
    }
    let mut n = 0;
    for id in ids {
        let deleted = tx.execute(
            "DELETE FROM jobs WHERE id = ?1
               AND NOT EXISTS (SELECT 1 FROM images WHERE images.job_id = jobs.id)",
            [id],
        )?;
        n += if deleted > 0 {
            deleted
        } else {
            tx.execute("UPDATE jobs SET queue_hidden = 1 WHERE id = ?1", [id])?
        };
    }
    tx.commit()?;
    Ok(n)
}

pub fn image_ids(conn: &Connection, job_id: &str) -> AppResult<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT id FROM images WHERE job_id = ?1 ORDER BY created_at ASC, rowid ASC")?;
    let ids = stmt
        .query_map([job_id], |r| r.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(ids)
}

pub fn insert_image(conn: &Connection, img: &ImageRow) -> AppResult<()> {
    conn.execute(
        // 所属はジョブの出力先を写す(保存の直前にプロジェクトが消えても外部キー違反にならないよう、DB から引く)
        "INSERT INTO images (id, job_id, file_path, thumb_path, width, height, seed, file_size, is_favorite, created_at, project_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, (SELECT project_id FROM jobs WHERE id = ?2))",
        params![
            img.id,
            img.job_id,
            img.file_path,
            img.thumb_path,
            img.width,
            img.height,
            img.seed as i64,
            img.file_size as i64,
            img.is_favorite,
            img.created_at
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn add(conn: &Connection, id: &str, now: i64) -> i64 {
        insert(conn, id, "{}", "p", "m", now).unwrap()
    }

    fn status(conn: &Connection, id: &str) -> JobStatus {
        get(conn, id).unwrap().unwrap().status
    }

    fn image(job: &str, id: &str) -> ImageRow {
        ImageRow {
            id: id.into(),
            job_id: job.into(),
            file_path: "2026/11/a.png".into(),
            thumb_path: "2026/11/a.png".into(),
            width: 512,
            height: 512,
            seed: 1,
            file_size: 3,
            is_favorite: false,
            created_at: 0,
            project_id: None,
        }
    }

    #[tokio::test]
    async fn positions_increase_and_next_queued_is_oldest_position() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            assert_eq!(add(c, "a", 1), 0);
            assert_eq!(add(c, "b", 2), 1);
            assert_eq!(next_queued(c)?.unwrap().id, "a");
            mark_running(c, "a", 5)?;
            assert_eq!(next_queued(c)?.unwrap().id, "b");
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn reorder_moves_within_queued_only() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            for (i, id) in ["a", "b", "c", "d"].iter().enumerate() {
                add(c, id, i as i64);
            }
            mark_running(c, "a", 9)?;
            reorder(c, "d", 0)?; // 待機中は b, c, d → d, b, c
            let order: Vec<_> = list(c, &[JobStatus::Queued], 10, 0)?
                .into_iter()
                .map(|j| j.id)
                .collect();
            assert_eq!(order, ["d", "b", "c"]);
            assert_eq!(next_queued(c)?.unwrap().id, "d");

            reorder(c, "d", 99)?; // 範囲外は末尾
            let order: Vec<_> = list(c, &[JobStatus::Queued], 10, 0)?
                .into_iter()
                .map(|j| j.id)
                .collect();
            assert_eq!(order, ["b", "c", "d"]);

            assert_eq!(reorder(c, "a", 0).unwrap_err().code(), "JOB_INVALID_STATE");
            assert_eq!(reorder(c, "zzz", 0).unwrap_err().code(), "JOB_NOT_FOUND");
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn list_orders_running_then_queued_then_finished() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            for (i, id) in ["old", "run", "q1", "q2"].iter().enumerate() {
                add(c, id, i as i64);
            }
            finish(c, "old", JobStatus::Completed, None, 10)?;
            mark_running(c, "run", 11)?;
            let ids: Vec<_> = list(c, &[], 10, 0)?.into_iter().map(|j| j.id).collect();
            assert_eq!(ids, ["run", "q1", "q2", "old"]);
            // フィルタとページング
            assert_eq!(list(c, &[JobStatus::Completed], 10, 0)?.len(), 1);
            assert_eq!(list(c, &[], 2, 2)?.len(), 2);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn fail_interrupted_only_touches_running() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            add(c, "r", 0);
            add(c, "q", 1);
            mark_running(c, "r", 2)?;
            assert_eq!(fail_interrupted(c, 9)?, 1);
            let r = get(c, "r")?.unwrap();
            assert_eq!(r.status, JobStatus::Failed);
            assert_eq!(r.error_code.as_deref(), Some("INTERRUPTED"));
            assert_eq!(status(c, "q"), JobStatus::Queued);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn remove_finished_deletes_or_hides_and_rejects_active_jobs() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            for id in ["with_img", "failed", "queued"] {
                add(c, id, 0);
            }
            finish(c, "with_img", JobStatus::Completed, None, 1)?;
            insert_image(c, &image("with_img", "img1"))?;
            finish(c, "failed", JobStatus::Failed, Some(("X", "y")), 1)?;

            // 待機中や存在しない ID が混ざっていたら、何も変えずに断る
            let ids = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
            assert!(remove_finished(c, &ids(&["failed", "queued"])).is_err());
            assert!(remove_finished(c, &ids(&["failed", "nope"])).is_err());
            assert!(get(c, "failed")?.is_some());

            assert_eq!(remove_finished(c, &ids(&["with_img", "failed"]))?, 2);
            assert!(get(c, "failed")?.is_none());
            assert_eq!(image_ids(c, "with_img")?, ["img1"]);
            let visible: Vec<_> = list(c, &[], 10, 0)?.into_iter().map(|j| j.id).collect();
            assert_eq!(visible, ["queued"]);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn clear_finished_keeps_images_and_hides_job() {
        let db = Db::open_in_memory().unwrap();
        db.call(|c| {
            for id in ["with_img", "failed", "queued"] {
                add(c, id, 0);
            }
            finish(c, "with_img", JobStatus::Completed, None, 1)?;
            insert_image(c, &image("with_img", "img1"))?;
            finish(c, "failed", JobStatus::Failed, Some(("X", "y")), 1)?;

            assert_eq!(clear_finished(c)?, 2);
            // 画像なしは削除、画像ありは残って非表示
            assert!(get(c, "failed")?.is_none());
            assert!(get(c, "with_img")?.is_some());
            assert_eq!(image_ids(c, "with_img")?, ["img1"]);
            let visible: Vec<_> = list(c, &[], 10, 0)?.into_iter().map(|j| j.id).collect();
            assert_eq!(visible, ["queued"]);
            // 2 回目は対象なし
            assert_eq!(clear_finished(c)?, 0);
            Ok(())
        })
        .await
        .unwrap();
    }
}
