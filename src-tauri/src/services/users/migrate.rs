//! ユーザー機能の導入前のデータ(単一の `gazooo.db`)を、最初のユーザーに引き継ぐ(docs/36 §8)。
//!
//! ファイルは移動しない(画像は数 GB になりうる)。DB だけをコピーし、画像保存先は旧位置に固定する。

use std::path::Path;

use rusqlite::{Connection, params};

use crate::AppPaths;
use crate::db::is_device_key;
use crate::error::{AppError, AppResult};
use crate::util::now_ms;

pub const LEGACY_DB: &str = "gazooo.db";
pub const LEGACY_RENAMED: &str = "gazooo.db.pre-users";

/// 最初のユーザーの既定名(あとから変更できる)。
pub const FIRST_USER_NAME: &str = "ユーザー 1";
pub const FIRST_USER_COLOR: &str = "blue";

/// 移行が必要か(ユーザーが 0 人で、従来の DB がある)。
pub fn needs_migration(paths: &AppPaths, store: &Connection) -> AppResult<bool> {
    let users: i64 = store.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
    Ok(users == 0 && paths.data_dir.join(LEGACY_DB).is_file())
}

/// 移行を実行して、作成した最初のユーザーの ID を返す。失敗したら作りかけを消して元に戻す。
pub fn migrate_legacy(paths: &AppPaths, store: &Connection) -> AppResult<String> {
    let legacy = paths.data_dir.join(LEGACY_DB);
    let id = uuid::Uuid::now_v7().to_string();
    let workspace = paths.user_workspace(&id);
    match run(paths, store, &legacy, &id, &workspace) {
        Ok(()) => Ok(id),
        Err(e) => {
            // 作りかけを片付ける(旧 DB は変更していない)
            let _ = std::fs::remove_dir_all(&workspace);
            let _ = store.execute("DELETE FROM users WHERE id = ?1", [&id]);
            Err(e)
        }
    }
}

fn run(
    paths: &AppPaths,
    store: &Connection,
    legacy: &Path,
    id: &str,
    workspace: &Path,
) -> AppResult<()> {
    std::fs::create_dir_all(workspace)?;
    let target = workspace.join(LEGACY_DB);

    // 1. 旧 DB を一貫した状態でコピーする(WAL を直接コピーしない)
    {
        let src = Connection::open(legacy)?;
        src.execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])?;
    }

    // 2. コピーを検査し、端末共通のキーを `device_settings` へ移す
    let copy = Connection::open(&target)?;
    let check: String = copy.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(AppError::internal(format!(
            "migration integrity check: {check}"
        )));
    }
    move_device_settings(&copy, store)?;

    // 3. 画像は移動しない。保存先を従来の実効値に固定する(未設定なら `{data}/images`)
    let has_dir: bool = copy.query_row(
        "SELECT EXISTS(SELECT 1 FROM settings WHERE key = 'storage.images_dir')",
        [],
        |r| r.get(0),
    )?;
    if !has_dir {
        let old = paths.data_dir.join("images");
        copy.execute(
            "INSERT INTO settings (key, value_json, updated_at) VALUES ('storage.images_dir', ?1, ?2)",
            params![
                serde_json::json!(old.to_string_lossy()).to_string(),
                now_ms()
            ],
        )?;
    }
    drop(copy);

    // 4. 最初のユーザーを登録する
    store.execute(
        "INSERT INTO users (id, name, color, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id, FIRST_USER_NAME, FIRST_USER_COLOR, now_ms()],
    )?;

    // 5. 旧 DB を残したまま名前を変える(自動では削除しない)。失敗しても移行自体は完了している
    if let Err(e) = std::fs::rename(legacy, legacy.with_file_name(LEGACY_RENAMED)) {
        tracing::warn!(error = %e, "could not rename legacy database");
    }
    Ok(())
}

fn move_device_settings(copy: &Connection, store: &Connection) -> AppResult<()> {
    let rows: Vec<(String, String, i64)> = {
        let mut stmt = copy.prepare("SELECT key, value_json, updated_at FROM settings")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?
    };
    for (key, value, at) in rows.into_iter().filter(|(k, _, _)| is_device_key(k)) {
        store.execute(
            "INSERT INTO device_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![key, value, at],
        )?;
        copy.execute("DELETE FROM settings WHERE key = ?1", [&key])?;
    }
    Ok(())
}
