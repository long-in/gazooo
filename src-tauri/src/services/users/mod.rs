//! ユーザー機能(docs/36)。ユーザー一覧・ロック・ログイン状態を管理する。
//!
//! ログイン中のユーザーは 1 人だけ。`Db`・`AppPaths` を共有する全サービスは、
//! ログイン・ログアウトに追従する。

pub mod auth;
pub mod migrate;

use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::AppPaths;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::util::now_ms;

pub const MAX_USERS: i64 = 20;
const NAME_MAX: usize = 24;
pub const PALETTE: [&str; 8] = [
    "red", "orange", "amber", "green", "teal", "blue", "violet", "pink",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthKind {
    None,
    Pin,
    Passphrase,
}

impl AuthKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pin => "pin",
            Self::Passphrase => "passphrase",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "pin" => Self::Pin,
            "passphrase" => Self::Passphrase,
            _ => Self::None,
        }
    }
}

/// フロントに返してよいユーザー情報(ハッシュは含めない)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSummary {
    pub id: String,
    pub name: String,
    pub color: String,
    pub auth_kind: AuthKind,
    pub pin_length: Option<u32>,
    /// 待機中のとき、この時刻(UTC ミリ秒)まで試行できない。
    pub locked_until: Option<i64>,
    pub last_login_at: Option<i64>,
}

struct UserRow {
    id: String,
    name: String,
    color: String,
    kind: AuthKind,
    hash: Option<String>,
    pin_length: Option<u32>,
    failed: u32,
    locked_until: Option<i64>,
    last_login_at: Option<i64>,
}

const COLS: &str =
    "id, name, color, auth_kind, auth_hash, pin_length, failed_count, locked_until, last_login_at";

fn read_row(r: &Row) -> rusqlite::Result<UserRow> {
    Ok(UserRow {
        id: r.get(0)?,
        name: r.get(1)?,
        color: r.get(2)?,
        kind: AuthKind::parse(&r.get::<_, String>(3)?),
        hash: r.get(4)?,
        pin_length: r.get(5)?,
        failed: r.get(6)?,
        locked_until: r.get(7)?,
        last_login_at: r.get(8)?,
    })
}

impl UserRow {
    fn summary(&self, now: i64) -> UserSummary {
        UserSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            color: self.color.clone(),
            auth_kind: self.kind,
            pin_length: self.pin_length,
            locked_until: self.locked_until.filter(|t| *t > now),
            last_login_at: self.last_login_at,
        }
    }
}

/// ユーザー削除の結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    /// 作業スペースの外に画像保存先を設定していた場合のそのパス(自動では削除しない)。
    pub external_images_dir: Option<String>,
}

pub struct UserService {
    store: Mutex<Connection>,
    db: Db,
    paths: AppPaths,
    current: RwLock<Option<String>>,
}

impl UserService {
    /// `app.db` を開いて、必要なら従来のデータを最初のユーザーに引き継ぐ(docs/36 §8)。
    pub fn new(app_db: &Path, db: Db, paths: AppPaths) -> AppResult<Arc<Self>> {
        let store = Connection::open(app_db)?;
        store.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        if migrate::needs_migration(&paths, &store)? {
            migrate::migrate_legacy(&paths, &store)?;
            tracing::info!("legacy data migrated to the first user");
        }
        Ok(Arc::new(Self {
            store: Mutex::new(store),
            db,
            paths,
            current: RwLock::new(None),
        }))
    }

    async fn blocking<T, F>(self: &Arc<Self>, f: F) -> AppResult<T>
    where
        F: FnOnce(&Self) -> AppResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let this = Arc::clone(self);
        tokio::task::spawn_blocking(move || f(&this))
            .await
            .map_err(|e| AppError::internal(e.to_string()))?
    }

    fn conn(&self) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
        self.store
            .lock()
            .map_err(|_| AppError::internal("user store lock poisoned"))
    }

    fn current_id(&self) -> Option<String> {
        self.current.read().ok().and_then(|g| g.clone())
    }

    fn row(&self, id: &str) -> AppResult<UserRow> {
        let c = self.conn()?;
        Self::get_row(&c, id)
    }

    fn get_row(c: &Connection, id: &str) -> AppResult<UserRow> {
        c.query_row(
            &format!("SELECT {COLS} FROM users WHERE id = ?1"),
            [id],
            read_row,
        )
        .optional()?
        .ok_or_else(|| AppError::validation("id", "ユーザーが見つかりません"))
    }

    // ---- 一覧・作成 ------------------------------------------------------------------

    pub async fn list(self: &Arc<Self>) -> AppResult<Vec<UserSummary>> {
        self.blocking(|s| {
            let c = s.conn()?;
            let now = now_ms();
            let mut stmt = c.prepare(&format!(
                "SELECT {COLS} FROM users
                 ORDER BY last_login_at IS NULL, last_login_at DESC, created_at ASC"
            ))?;
            let rows = stmt
                .query_map([], read_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows.iter().map(|r| r.summary(now)).collect())
        })
        .await
    }

    pub async fn current(self: &Arc<Self>) -> AppResult<Option<UserSummary>> {
        self.blocking(|s| {
            let Some(id) = s.current_id() else {
                return Ok(None);
            };
            let c = s.conn()?;
            Ok(Some(Self::get_row(&c, &id)?.summary(now_ms())))
        })
        .await
    }

    pub async fn create(self: &Arc<Self>, name: String, color: String) -> AppResult<UserSummary> {
        self.blocking(move |s| {
            let name = validate_name(&name)?;
            if !PALETTE.contains(&color.as_str()) {
                return Err(AppError::validation("color", "色の指定が正しくありません"));
            }
            let c = s.conn()?;
            let count: i64 = c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
            if count >= MAX_USERS {
                return Err(AppError::user(
                    "USER_LIMIT",
                    format!("ユーザーは {MAX_USERS} 人までです"),
                ));
            }
            ensure_name_free(&c, &name, None)?;
            let id = uuid::Uuid::now_v7().to_string();
            c.execute(
                "INSERT INTO users (id, name, color, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![id, name, color, now_ms()],
            )?;
            Ok(Self::get_row(&c, &id)?.summary(now_ms()))
        })
        .await
    }

    // ---- ロックの検証 ----------------------------------------------------------------

    /// ロックの入力を検証する。失敗は回数を数え、待機を設定する(`app.db` に保存。再起動でも残る)。
    fn check_secret(&self, row: &UserRow, secret: Option<&str>) -> AppResult<()> {
        if row.kind == AuthKind::None {
            return Ok(());
        }
        let now = now_ms();
        if let Some(until) = row.locked_until.filter(|t| *t > now) {
            return Err(locked_out(until - now));
        }
        let ok = match (secret, row.hash.as_deref()) {
            (Some(s), Some(h)) => auth::verify_secret(s, h),
            _ => false,
        };
        let c = self.conn()?;
        if ok {
            c.execute(
                "UPDATE users SET failed_count = 0, locked_until = NULL WHERE id = ?1",
                [&row.id],
            )?;
            return Ok(());
        }
        let failed = row.failed + 1;
        let wait = auth::lockout_ms(failed);
        c.execute(
            "UPDATE users SET failed_count = ?2, locked_until = ?3 WHERE id = ?1",
            params![row.id, failed, wait.map(|w| now + w)],
        )?;
        match wait {
            Some(w) => Err(locked_out(w)),
            None => Err(AppError::user_with(
                "AUTH_FAILED",
                "入力が違います",
                serde_json::json!({ "remaining": auth::FREE_ATTEMPTS - failed }),
            )),
        }
    }

    // ---- ログイン・ログアウト ---------------------------------------------------------

    pub async fn login(
        self: &Arc<Self>,
        id: String,
        secret: Option<String>,
    ) -> AppResult<UserSummary> {
        self.blocking(move |s| {
            if s.current_id().is_some() {
                return Err(AppError::validation("id", "すでにログインしています"));
            }
            let row = s.row(&id)?;
            s.check_secret(&row, secret.as_deref())?;

            let workspace = s.paths.user_workspace(&row.id);
            std::fs::create_dir_all(&workspace)?;
            s.db.attach_user(&workspace.join(migrate::LEGACY_DB))?;
            s.paths.set_user_dir(Some(workspace));
            if let Ok(mut g) = s.current.write() {
                *g = Some(row.id.clone());
            }
            let c = s.conn()?;
            c.execute(
                "UPDATE users SET last_login_at = ?2 WHERE id = ?1",
                params![row.id, now_ms()],
            )?;
            tracing::info!(user = %row.id, "user logged in");
            Ok(Self::get_row(&c, &row.id)?.summary(now_ms()))
        })
        .await
    }

    /// ユーザーの DB を手放す。呼ぶ前に、実行中のジョブなどを止めておくこと(コマンド層)。
    pub fn logout(&self) -> AppResult<()> {
        self.db.detach_user()?;
        self.paths.set_user_dir(None);
        if let Ok(mut g) = self.current.write() {
            *g = None;
        }
        tracing::info!("user logged out");
        Ok(())
    }

    pub fn is_logged_in(&self) -> bool {
        self.current_id().is_some()
    }

    // ---- 名前の変更・削除 ------------------------------------------------------------

    pub async fn rename(
        self: &Arc<Self>,
        id: String,
        name: String,
        secret: Option<String>,
    ) -> AppResult<UserSummary> {
        self.blocking(move |s| {
            let name = validate_name(&name)?;
            let row = s.row(&id)?;
            // ログイン中の自分自身は、ロックの入力なしで変更できる
            if s.current_id().as_deref() != Some(id.as_str()) {
                s.check_secret(&row, secret.as_deref())?;
            }
            let c = s.conn()?;
            ensure_name_free(&c, &name, Some(&id))?;
            c.execute(
                "UPDATE users SET name = ?2 WHERE id = ?1",
                params![id, name],
            )?;
            Ok(Self::get_row(&c, &id)?.summary(now_ms()))
        })
        .await
    }

    /// ログイン中の本人のアバター色を変更する(docs/38 §3)。他人の色は変えられない。
    pub async fn set_color(self: &Arc<Self>, id: String, color: String) -> AppResult<UserSummary> {
        self.blocking(move |s| {
            if !PALETTE.contains(&color.as_str()) {
                return Err(AppError::validation("color", "色の指定が正しくありません"));
            }
            if s.current_id().as_deref() != Some(id.as_str()) {
                return Err(AppError::user(
                    "FORBIDDEN",
                    "色を変更できるのはログイン中のユーザー本人だけです",
                ));
            }
            let c = s.conn()?;
            c.execute(
                "UPDATE users SET color = ?2 WHERE id = ?1",
                params![id, color],
            )?;
            Ok(Self::get_row(&c, &id)?.summary(now_ms()))
        })
        .await
    }

    /// 作業スペースを完全に削除する(ゴミ箱へは移さない。docs/36 §5.5)。
    pub async fn delete(
        self: &Arc<Self>,
        id: String,
        secret: Option<String>,
        confirm_name: Option<String>,
    ) -> AppResult<DeleteResult> {
        self.blocking(move |s| {
            if s.current_id().as_deref() == Some(id.as_str()) {
                return Err(AppError::validation(
                    "id",
                    "ログイン中のユーザーは削除できません。切り替えてから削除してください",
                ));
            }
            let row = s.row(&id)?;
            let count: i64 = s
                .conn()?
                .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
            if count <= 1 {
                return Err(AppError::user("USER_LAST", "最後の 1 人は削除できません"));
            }
            if row.kind != AuthKind::None {
                match secret.as_deref() {
                    Some(_) => s.check_secret(&row, secret.as_deref())?,
                    // 「ロックを忘れた」: ユーザー名の入力で確認する(復旧手段はない。docs/36 §5.5)
                    None if confirm_name.as_deref() == Some(row.name.as_str()) => {}
                    None => {
                        return Err(AppError::validation(
                            "confirmName",
                            "ユーザー名を正確に入力してください",
                        ));
                    }
                }
            }
            let workspace = s.paths.user_workspace(&id);
            let external = external_images_dir(&workspace);
            match std::fs::remove_dir_all(&workspace) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            s.conn()?
                .execute("DELETE FROM users WHERE id = ?1", [&id])?;
            tracing::info!(user = %id, "user deleted");
            Ok(DeleteResult {
                external_images_dir: external,
            })
        })
        .await
    }

    // ---- ロックの設定 ----------------------------------------------------------------

    /// ログイン中のユーザーのロックを設定・変更する(既存のロックがあれば現在の値が必要)。
    pub async fn lock_set(
        self: &Arc<Self>,
        kind: AuthKind,
        secret: String,
        pin_length: Option<u32>,
        current: Option<String>,
    ) -> AppResult<UserSummary> {
        self.blocking(move |s| {
            let id = s.current_id().ok_or(AppError::NotLoggedIn)?;
            let row = s.row(&id)?;
            // 入力の形式を先に確かめる(誤った入力で失敗回数を増やさない)
            let length = match kind {
                AuthKind::Pin => {
                    let n = pin_length.unwrap_or(4) as usize;
                    auth::validate_pin(&secret, n)?;
                    Some(n as u32)
                }
                AuthKind::Passphrase => {
                    auth::validate_passphrase(&secret)?;
                    None
                }
                AuthKind::None => {
                    return Err(AppError::validation("kind", "方式を指定してください"));
                }
            };
            s.check_secret(&row, current.as_deref())?;
            let hash = auth::hash_secret(&secret)?;
            let c = s.conn()?;
            c.execute(
                "UPDATE users SET auth_kind = ?2, auth_hash = ?3, pin_length = ?4,
                 failed_count = 0, locked_until = NULL WHERE id = ?1",
                params![id, kind.as_str(), hash, length],
            )?;
            Ok(Self::get_row(&c, &id)?.summary(now_ms()))
        })
        .await
    }

    pub async fn lock_clear(self: &Arc<Self>, current: Option<String>) -> AppResult<UserSummary> {
        self.blocking(move |s| {
            let id = s.current_id().ok_or(AppError::NotLoggedIn)?;
            let row = s.row(&id)?;
            s.check_secret(&row, current.as_deref())?;
            let c = s.conn()?;
            c.execute(
                "UPDATE users SET auth_kind = 'none', auth_hash = NULL, pin_length = NULL,
                 failed_count = 0, locked_until = NULL WHERE id = ?1",
                [&id],
            )?;
            Ok(Self::get_row(&c, &id)?.summary(now_ms()))
        })
        .await
    }
}

fn locked_out(retry_after_ms: i64) -> AppError {
    AppError::user_with(
        "AUTH_LOCKED_OUT",
        "試行が続いたため、しばらく入力できません",
        serde_json::json!({ "retryAfterMs": retry_after_ms.max(0) }),
    )
}

fn validate_name(name: &str) -> AppResult<String> {
    let name = name.trim().to_string();
    let n = name.chars().count();
    if n == 0 || n > NAME_MAX || name.chars().any(char::is_control) {
        return Err(AppError::validation(
            "name",
            "名前は 1〜24 文字で入力してください",
        ));
    }
    Ok(name)
}

fn ensure_name_free(c: &Connection, name: &str, except: Option<&str>) -> AppResult<()> {
    let taken: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE lower(name) = lower(?1) AND id != ?2)",
        params![name, except.unwrap_or("")],
        |r| r.get(0),
    )?;
    if taken {
        return Err(AppError::user(
            "USER_NAME_TAKEN",
            "同じ名前のユーザーがいます",
        ));
    }
    Ok(())
}

/// 削除するユーザーが作業スペースの外に画像保存先を持っていれば、そのパス(読み取り専用で調べる)。
fn external_images_dir(workspace: &Path) -> Option<String> {
    let db = workspace.join(migrate::LEGACY_DB);
    let c = Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let raw: String = c
        .query_row(
            "SELECT value_json FROM settings WHERE key = 'storage.images_dir'",
            [],
            |r| r.get(0),
        )
        .ok()?;
    let dir: String = serde_json::from_str(&raw).ok()?;
    (!Path::new(&dir).starts_with(workspace)).then_some(dir)
}

#[cfg(test)]
mod tests;
