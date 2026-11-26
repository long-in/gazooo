//! SQLite 接続とマイグレーション(docs/03-data-model.md)。
//!
//! `rusqlite` は同期 API のため、`Db::call` で `spawn_blocking` 上に逃がして使う。

use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

pub mod assistant;
pub mod images;
pub mod jobs;
pub mod presets;
pub mod projects;
pub mod scopes;

use crate::error::{AppError, AppResult};

const MIGRATION_SLICE: &[M<'static>] = &[
    M::up(include_str!("migrations/0001_init.sql")),
    M::up(include_str!("migrations/0002_job_queue_hidden.sql")),
    M::up(include_str!("migrations/0003_assistant.sql")),
    M::up(include_str!("migrations/0004_projects.sql")),
];
const MIGRATIONS: Migrations<'static> = Migrations::from_slice(MIGRATION_SLICE);

/// `app.db`(端末共通。ユーザー一覧と端末の設定)のマイグレーション(docs/36 §3)。
const APP_MIGRATION_SLICE: &[M<'static>] = &[M::up(include_str!("migrations/app_0001_init.sql"))];
const APP_MIGRATIONS: Migrations<'static> = Migrations::from_slice(APP_MIGRATION_SLICE);

/// 端末共通の設定キーか(docs/36 §4)。`true` のキーは `app.db` の `device_settings` に、
/// それ以外はログイン中のユーザーの `settings` に保存する。分類にないキーはユーザーごと。
pub fn is_device_key(key: &str) -> bool {
    matches!(
        key,
        "engine.comfyui.url"
            | "engine.comfyui.managed"
            | "engine.comfyui.models_dir"
            | "debug.enabled"
            | "storage.images_root"
            | "storage.legacy_images_user"
    ) || key.starts_with("llm.")
}

/// 全サービスが共有する DB のハンドル。ログイン中のユーザーの DB に差し替わる(docs/36 §6)。
///
/// - `call`: ログイン中のユーザーの DB。未ログインでは `NOT_LOGGED_IN`。
/// - `call_device`: 端末共通の設定だけを読み書きできる接続(未ログインでも使える)。
///
/// `Clone` で共有されるため、ログイン・ログアウトの切り替えは全サービスに反映される。
#[derive(Clone)]
pub struct Db {
    inner: Arc<Inner>,
}

struct Inner {
    user: RwLock<Option<Arc<Mutex<Connection>>>>,
    device: Arc<Mutex<Connection>>,
    /// `ATTACH` するときの `app.db` の場所(テストではメモリ上の共有 DB の URI)。
    app_uri: String,
}

impl Db {
    /// 端末共通の DB(`app.db`)を開いて(必要ならマイグレーション)、未ログインのハンドルを返す。
    pub fn new(app_db: &Path) -> AppResult<Self> {
        if let Some(parent) = app_db.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut app = Connection::open(app_db)?;
        app.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        APP_MIGRATIONS.to_latest(&mut app)?;
        drop(app);
        Self::with_app_uri(app_db.to_string_lossy().into_owned())
    }

    fn with_app_uri(app_uri: String) -> AppResult<Self> {
        let device = Connection::open_in_memory()?;
        attach_app(&device, &app_uri)?;
        Ok(Self {
            inner: Arc::new(Inner {
                user: RwLock::new(None),
                device: Arc::new(Mutex::new(device)),
                app_uri,
            }),
        })
    }

    /// ユーザーの DB を開き(必要ならバックアップ・マイグレーション)、ログイン状態にする。
    pub fn attach_user(&self, path: &Path) -> AppResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        configure(&conn)?;
        backup_before_migration(&conn, path)?;
        MIGRATIONS.to_latest(&mut conn)?;
        attach_app(&conn, &self.inner.app_uri)?;
        self.set_user(Some(conn))
    }

    /// ログアウト。ユーザーの DB 接続を手放す。
    pub fn detach_user(&self) -> AppResult<()> {
        self.set_user(None)
    }

    pub fn is_attached(&self) -> bool {
        self.inner.user.read().map(|g| g.is_some()).unwrap_or(false)
    }

    fn set_user(&self, conn: Option<Connection>) -> AppResult<()> {
        let mut slot = self
            .inner
            .user
            .write()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        *slot = conn.map(|c| Arc::new(Mutex::new(c)));
        Ok(())
    }

    /// テスト・検査用: ユーザーの DB を直接開いてログイン状態にした(`app.db` はメモリ上)ハンドル。
    #[cfg(test)]
    pub fn open(path: &Path) -> AppResult<Self> {
        let db = Self::with_app_uri(memory_app_uri())?;
        db.inner.attach_app_schema()?;
        db.attach_user(path)?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> AppResult<Self> {
        let db = Self::with_app_uri(memory_app_uri())?;
        db.inner.attach_app_schema()?;
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        MIGRATIONS.to_latest(&mut conn)?;
        attach_app(&conn, &db.inner.app_uri)?;
        db.set_user(Some(conn))?;
        Ok(db)
    }

    /// ブロッキング用スレッドで、ログイン中のユーザーの DB 処理を実行する。
    pub async fn call<T, F>(&self, f: F) -> AppResult<T>
    where
        F: FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let conn = self
            .inner
            .user
            .read()
            .map_err(|_| AppError::internal("database lock poisoned"))?
            .clone()
            .ok_or(AppError::NotLoggedIn)?;
        run_blocking(conn, f).await
    }

    /// 端末共通の設定だけを扱う処理(未ログインでも使える。ユーザーのテーブルは無い)。
    pub async fn call_device<T, F>(&self, f: F) -> AppResult<T>
    where
        F: FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
        T: Send + 'static,
    {
        run_blocking(Arc::clone(&self.inner.device), f).await
    }
}

impl Inner {
    /// テスト用: メモリ上の `app.db` にスキーマを作る(共有キャッシュの URI を直接開く)。
    #[cfg(test)]
    fn attach_app_schema(&self) -> AppResult<()> {
        let c = Connection::open(&self.app_uri)?;
        c.execute_batch(include_str!("migrations/app_0001_init.sql"))?;
        Ok(())
    }
}

#[cfg(test)]
fn memory_app_uri() -> String {
    format!(
        "file:gazooo-app-{}?mode=memory&cache=shared",
        uuid::Uuid::now_v7()
    )
}

fn attach_app(conn: &Connection, location: &str) -> AppResult<()> {
    conn.execute("ATTACH DATABASE ?1 AS app", [location])?;
    Ok(())
}

async fn run_blocking<T, F>(conn: Arc<Mutex<Connection>>, f: F) -> AppResult<T>
where
    F: FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let mut guard = conn
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        f(&mut guard)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

fn configure(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;",
    )?;
    Ok(())
}

/// 既存 DB のスキーマが古い場合、マイグレーション前に `gazooo.db.bak-<version>` を作る。
fn backup_before_migration(conn: &Connection, path: &Path) -> AppResult<()> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 0 || current as usize >= MIGRATION_SLICE.len() {
        return Ok(());
    }
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    let mut backup = path.as_os_str().to_owned();
    backup.push(format!(".bak-{current}"));
    std::fs::copy(path, std::path::PathBuf::from(backup))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_valid() {
        MIGRATIONS.validate().unwrap();
    }

    #[tokio::test]
    async fn creates_all_tables() {
        let db = Db::open_in_memory().unwrap();
        let tables: Vec<String> = db
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
                )?;
                let rows = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        assert_eq!(
            tables,
            [
                "assistant_conversations",
                "assistant_messages",
                "images",
                "jobs",
                "presets",
                "projects",
                "settings"
            ]
        );
    }

    #[tokio::test]
    async fn foreign_keys_cascade_on_job_delete() {
        let db = Db::open_in_memory().unwrap();
        let count: i64 = db
            .call(|c| {
                c.execute(
                    "INSERT INTO jobs (id,status,position,engine,params_json,prompt_text,model_name,created_at)
                     VALUES ('j','queued',0,'comfyui','{}','p','m',0)",
                    [],
                )?;
                c.execute(
                    "INSERT INTO images (id,job_id,file_path,thumb_path,width,height,seed,file_size,created_at)
                     VALUES ('i','j','a.png','a.webp',1,1,1,1,0)",
                    [],
                )?;
                c.execute("DELETE FROM jobs WHERE id='j'", [])?;
                Ok(c.query_row("SELECT COUNT(*) FROM images", [], |r| r.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn file_db_uses_wal_and_persists() {
        let dir = std::env::temp_dir().join(format!("gazooo-test-{}", uuid::Uuid::now_v7()));
        let path = dir.join("gazooo.db");
        {
            let db = Db::open(&path).unwrap();
            let mode: String = db
                .inner
                .user
                .read()
                .unwrap()
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
        }
        // 再オープン(マイグレーション済みでもエラーにならない)
        Db::open(&path).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
