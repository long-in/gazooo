//! `presets` テーブル(docs/03-data-model.md)。

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::AppResult;

#[derive(Debug, Clone)]
pub struct PresetRow {
    pub id: String,
    pub name: String,
    pub params_json: String,
    pub created_at: i64,
    pub updated_at: i64,
}

const COLS: &str = "id, name, params_json, created_at, updated_at";

fn map(r: &Row) -> rusqlite::Result<PresetRow> {
    Ok(PresetRow {
        id: r.get(0)?,
        name: r.get(1)?,
        params_json: r.get(2)?,
        created_at: r.get(3)?,
        updated_at: r.get(4)?,
    })
}

pub fn list(conn: &Connection) -> AppResult<Vec<PresetRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM presets ORDER BY name COLLATE NOCASE ASC"
    ))?;
    let rows = stmt.query_map([], map)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Option<PresetRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM presets WHERE id = ?1"),
            [id],
            map,
        )
        .optional()?)
}

pub fn find_by_name(conn: &Connection, name: &str) -> AppResult<Option<PresetRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM presets WHERE name = ?1"),
            [name],
            map,
        )
        .optional()?)
}

pub fn insert(conn: &Connection, p: &PresetRow) -> AppResult<()> {
    conn.execute(
        "INSERT INTO presets (id, name, params_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![p.id, p.name, p.params_json, p.created_at, p.updated_at],
    )?;
    Ok(())
}

/// 見つからなければ `false`。
pub fn update(
    conn: &Connection,
    id: &str,
    name: &str,
    params_json: &str,
    now: i64,
) -> AppResult<bool> {
    let n = conn.execute(
        "UPDATE presets SET name = ?2, params_json = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, name, params_json, now],
    )?;
    Ok(n > 0)
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<bool> {
    Ok(conn.execute("DELETE FROM presets WHERE id = ?1", [id])? > 0)
}
