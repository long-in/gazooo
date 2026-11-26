//! 全体設定と個別設定の解決(docs/52)。使う値は「個別設定(あれば)→ 全体設定 → 標準値」。
//!
//! 上書きできる項目は組(`Group`)にまとめる。ユーザーの DB に組の印があれば、その組のキーは
//! ユーザーの値だけを読む(無いキーは標準値)。印がなければ全体設定(`app.device_settings`)を読む。
//! 組の中で送信先とモデルが食い違わないようにするため、キー単位では混ぜない。

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{assistant, is_device_key};
use crate::error::AppResult;
use crate::util::now_ms;

const GLOBAL: &str = "app.device_settings";
const USER: &str = "main.settings";
/// 以前の版の値を引き継ぎ済みである印(ユーザーの DB)。
const KEY_UPGRADED: &str = "scope.upgraded";
const KEY_IMAGES_DIR: &str = "storage.images_dir";
/// 全体の画像の保存先(`app.db`)。
pub const KEY_IMAGES_ROOT: &str = "storage.images_root";
/// 以前の版の画像の場所(`{data}/images`)を使い続けるユーザー(`app.db`。docs/52 §5)。
pub const KEY_LEGACY_IMAGES_USER: &str = "storage.legacy_images_user";

/// 書き込み先。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Global,
    #[default]
    User,
}

/// 読み方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// 実際に使う値。
    Effective,
    /// 全体設定の値(個別設定を見ない)。
    Global,
}

impl From<Option<Scope>> for Layer {
    fn from(scope: Option<Scope>) -> Self {
        match scope {
            Some(Scope::Global) => Self::Global,
            _ => Self::Effective,
        }
    }
}

/// 上書きできる項目の組(docs/52 §2 B。画像の保存先は専用の扱い)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Group {
    Theme,
    Language,
    AssistantConnection,
    AssistantPrompt,
    AssistantAutoDelete,
    Translate,
    Convert,
}

impl Group {
    pub const ALL: [Group; 7] = [
        Self::Theme,
        Self::Language,
        Self::AssistantConnection,
        Self::AssistantPrompt,
        Self::AssistantAutoDelete,
        Self::Translate,
        Self::Convert,
    ];

    pub fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Theme => &["ui.theme"],
            Self::Language => &["ui.language"],
            Self::AssistantConnection => &[
                "assistant.provider",
                "assistant.model",
                "assistant.local_base_url",
                "assistant.local_model",
            ],
            Self::AssistantPrompt => &["assistant.system_prompt"],
            Self::AssistantAutoDelete => &["assistant.auto_delete"],
            Self::Translate => &[
                "translate.provider",
                "translate.model",
                "translate.mt_model",
            ],
            Self::Convert => &["convert.settings"],
        }
    }

    fn marker(self) -> &'static str {
        match self {
            Self::Theme => "override.theme",
            Self::Language => "override.language",
            Self::AssistantConnection => "override.assistant_connection",
            Self::AssistantPrompt => "override.assistant_prompt",
            Self::AssistantAutoDelete => "override.assistant_auto_delete",
            Self::Translate => "override.translate",
            Self::Convert => "override.convert",
        }
    }

    pub fn of(key: &str) -> Option<Group> {
        Self::ALL.into_iter().find(|g| g.keys().contains(&key))
    }
}

fn read(conn: &Connection, table: &str, key: &str) -> AppResult<Option<(String, i64)>> {
    Ok(conn
        .query_row(
            &format!("SELECT value_json, updated_at FROM {table} WHERE key = ?1"),
            [key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

fn put(conn: &Connection, table: &str, key: &str, json: &str) -> AppResult<()> {
    conn.execute(
        &format!(
            "INSERT INTO {table} (key, value_json, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at"
        ),
        params![key, json, now_ms()],
    )?;
    Ok(())
}

fn remove(conn: &Connection, table: &str, key: &str) -> AppResult<()> {
    conn.execute(&format!("DELETE FROM {table} WHERE key = ?1"), [key])?;
    Ok(())
}

pub fn is_overridden(conn: &Connection, group: Group) -> AppResult<bool> {
    Ok(read(conn, USER, group.marker())?.is_some())
}

/// 値を読むテーブル。`Layer::Global` は、上書きできる項目についてユーザーの DB に触れない。
pub fn read_table(conn: &Connection, layer: Layer, key: &str) -> AppResult<&'static str> {
    if is_device_key(key) {
        return Ok(GLOBAL);
    }
    Ok(match Group::of(key) {
        None => USER,
        Some(_) if layer == Layer::Global => GLOBAL,
        Some(g) if is_overridden(conn, g)? => USER,
        Some(_) => GLOBAL,
    })
}

pub fn get_json(conn: &Connection, layer: Layer, key: &str) -> AppResult<Option<Value>> {
    let table = read_table(conn, layer, key)?;
    // 壊れた値は無いものとして扱う(起動不能にしない)
    Ok(read(conn, table, key)?.and_then(|(s, _)| serde_json::from_str(&s).ok()))
}

/// 値を書く(`None` は削除 = そのスコープでは標準値)。上書きできる項目を個別に書くと、
/// 組全体が個別設定になる(ほかのキーは、その時点の全体の値を引き継ぐ)。
pub fn write(conn: &Connection, scope: Scope, key: &str, value: Option<&Value>) -> AppResult<()> {
    let table = if is_device_key(key) {
        GLOBAL
    } else {
        match Group::of(key) {
            None => USER,
            Some(_) if scope == Scope::Global => GLOBAL,
            Some(g) => {
                set_override(conn, g, true)?;
                USER
            }
        }
    };
    match value {
        Some(v) => put(conn, table, key, &v.to_string()),
        None => remove(conn, table, key),
    }
}

/// 組を個別設定にする(全体の値を写してから)/ 全体設定に戻す。すでにその状態なら何もしない。
pub fn set_override(conn: &Connection, group: Group, enabled: bool) -> AppResult<()> {
    if is_overridden(conn, group)? == enabled {
        return Ok(());
    }
    for key in group.keys() {
        remove(conn, USER, key)?;
        if enabled && let Some((json, _)) = read(conn, GLOBAL, key)? {
            put(conn, USER, key, &json)?;
        }
    }
    if enabled {
        put(conn, USER, group.marker(), "true")
    } else {
        remove(conn, USER, group.marker())
    }
}

/// 全体設定の組を標準値に戻す。
pub fn reset_global(conn: &Connection, group: Group) -> AppResult<()> {
    for key in group.keys() {
        remove(conn, GLOBAL, key)?;
    }
    Ok(())
}

/// 標準値と同じ値か(引き継ぎで「全体設定を使う」にするかの判定)。
fn is_standard(key: &str, value: &Value) -> bool {
    let standard: Value = match key {
        "ui.theme" => "dark".into(),
        "assistant.provider" | "translate.provider" => "anthropic".into(),
        "assistant.model" => assistant::DEFAULT_MODEL.into(),
        "assistant.local_base_url" => assistant::DEFAULT_LOCAL_BASE_URL.into(),
        "assistant.local_model" => assistant::DEFAULT_LOCAL_MODEL.into(),
        "assistant.auto_delete" => true.into(),
        "assistant.system_prompt" => {
            crate::services::assistant::capabilities::default_prompt().into()
        }
        "translate.model" => assistant::DEFAULT_TRANSLATE_MODEL.into(),
        "translate.mt_model" => "general".into(),
        "convert.settings" => {
            let default = crate::services::convert::ConvertSettings::default();
            return serde_json::from_value::<crate::services::convert::ConvertSettings>(
                value.clone(),
            )
            .is_ok_and(|v| v == default);
        }
        // 言語の標準は「OS に合わせる」(値なし)。保存されている値は、標準と異なる
        _ => return false,
    };
    *value == standard
}

/// 以前の版で保存した値を、全体 / 個別の区分へ引き継ぐ(docs/52 §5)。ユーザーごとに一度だけ。
/// 画像のファイルは移さず、そのユーザーの見え方も変えない。
/// `conn` はユーザーの DB で、`app.db` が `app` として ATTACH されていること。
pub fn upgrade_user(conn: &Connection, data_dir: &Path, user_id: &str) -> AppResult<()> {
    if read(conn, USER, KEY_UPGRADED)?.is_some() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    for group in Group::ALL {
        if is_overridden(&tx, group)? {
            continue;
        }
        let mut stored = Vec::new();
        for key in group.keys() {
            if let Some((json, _)) = read(&tx, USER, key)? {
                stored.push((*key, serde_json::from_str::<Value>(&json).ok()));
            }
        }
        if stored.is_empty() {
            continue;
        }
        let all_standard = stored
            .iter()
            .all(|(k, v)| v.as_ref().is_none_or(|v| v.is_null() || is_standard(k, v)));
        if all_standard {
            for (key, _) in stored {
                remove(&tx, USER, key)?;
            }
        } else {
            put(&tx, USER, group.marker(), "true")?;
        }
    }
    // 以前の版から固定された画像の場所は、利用者が選んだものではないので個別設定にしない
    let pinned = read(&tx, USER, KEY_IMAGES_DIR)?
        .and_then(|(s, _)| serde_json::from_str::<String>(&s).ok())
        .is_some_and(|d| {
            crate::services::models::engine_paths::same_dir(Path::new(&d), &data_dir.join("images"))
        });
    if pinned {
        remove(&tx, USER, KEY_IMAGES_DIR)?;
        // 全体の保存先が設定済みなら、そちらに従う(特例はもう要らない)
        if read(&tx, GLOBAL, KEY_IMAGES_ROOT)?.is_none() {
            let json = Value::String(user_id.to_owned()).to_string();
            put(&tx, GLOBAL, KEY_LEGACY_IMAGES_USER, &json)?;
        }
    }
    put(&tx, USER, KEY_UPGRADED, "true")?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    async fn with<T: Send + 'static>(
        db: &Db,
        f: impl FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    ) -> T {
        db.call(f).await.unwrap()
    }

    fn effective(c: &Connection, key: &str) -> AppResult<Option<Value>> {
        get_json(c, Layer::Effective, key)
    }

    #[tokio::test]
    async fn user_value_wins_and_global_changes_reach_only_followers() {
        let db = Db::open_in_memory().unwrap();
        with(&db, |c| {
            write(c, Scope::Global, "ui.theme", Some(&"light".into()))?;
            assert_eq!(effective(c, "ui.theme")?, Some("light".into()));
            write(c, Scope::User, "ui.theme", Some(&"system".into()))?;
            write(c, Scope::Global, "ui.theme", Some(&"dark".into()))?;
            assert_eq!(effective(c, "ui.theme")?, Some("system".into()));
            assert_eq!(get_json(c, Layer::Global, "ui.theme")?, Some("dark".into()));
            set_override(c, Group::Theme, false)?;
            assert_eq!(effective(c, "ui.theme")?, Some("dark".into()));
            reset_global(c, Group::Theme)?;
            assert_eq!(effective(c, "ui.theme")?, None);
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn a_group_is_overridden_as_a_whole() {
        let db = Db::open_in_memory().unwrap();
        with(&db, |c| {
            write(
                c,
                Scope::Global,
                "assistant.provider",
                Some(&"local".into()),
            )?;
            write(c, Scope::Global, "assistant.local_model", Some(&"g".into()))?;
            // 1 つ書くと、ほかは全体の値を引き継いで組ごと個別になる
            write(c, Scope::User, "assistant.model", Some(&"m".into()))?;
            write(
                c,
                Scope::Global,
                "assistant.provider",
                Some(&"anthropic".into()),
            )?;
            assert_eq!(effective(c, "assistant.provider")?, Some("local".into()));
            assert_eq!(effective(c, "assistant.local_model")?, Some("g".into()));
            // 個別で値を消すと、全体ではなく標準値になる(言語の「OS に合わせる」)
            write(c, Scope::Global, "ui.language", Some(&"en".into()))?;
            write(c, Scope::User, "ui.language", None)?;
            assert_eq!(effective(c, "ui.language")?, None);
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn personal_keys_ignore_the_scope() {
        let db = Db::open_in_memory().unwrap();
        with(&db, |c| {
            write(c, Scope::Global, "assistant.api_key", Some(&"k".into()))?;
            let global: i64 = c.query_row(
                "SELECT COUNT(*) FROM app.device_settings WHERE key = 'assistant.api_key'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(global, 0, "秘密は全体設定に入れない");
            assert_eq!(effective(c, "assistant.api_key")?, Some("k".into()));
            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn upgrade_drops_standard_values_and_runs_once() {
        let db = Db::open_in_memory().unwrap();
        let data = std::env::temp_dir().join("gazooo-scope-upgrade");
        let pinned = Value::String(data.join("images").to_string_lossy().into_owned());
        with(&db, move |c| {
            put(c, USER, "ui.theme", "\"dark\"")?;
            put(c, USER, "ui.language", "\"en\"")?;
            put(c, USER, "assistant.provider", "\"anthropic\"")?;
            put(c, USER, "assistant.model", "\"claude-haiku-5-5\"")?;
            put(c, USER, KEY_IMAGES_DIR, &pinned.to_string())?;
            upgrade_user(c, &data, "u1")?;
            assert!(!is_overridden(c, Group::Theme)?);
            assert!(is_overridden(c, Group::Language)?);
            assert!(is_overridden(c, Group::AssistantConnection)?);
            assert!(read(c, USER, KEY_IMAGES_DIR)?.is_none());
            assert_eq!(
                get_json(c, Layer::Effective, KEY_LEGACY_IMAGES_USER)?,
                Some("u1".into())
            );
            // 2 回目は何もしない
            put(c, USER, KEY_IMAGES_DIR, &pinned.to_string())?;
            upgrade_user(c, &data, "u1")?;
            assert!(read(c, USER, KEY_IMAGES_DIR)?.is_some());
            Ok(())
        })
        .await;
    }
}
