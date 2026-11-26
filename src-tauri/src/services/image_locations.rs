//! 各ユーザーの画像の場所と、指定できる場所の制限(docs/52 §4.1)。

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::AppPaths;
use crate::db::Db;
use crate::db::jobs::ImageRow;
use crate::error::{AppError, AppResult};
use crate::services::migration::files::{self, Relation};
use crate::services::settings;
use crate::services::users::migrate::LEGACY_DB as USER_DB;

/// 画像の場所: 個別の指定 → 全体の保存先の下のユーザー ID → 以前の版の場所 → アプリの既定。
pub fn location(
    data_dir: &Path,
    user_id: Option<&str>,
    individual: Option<&str>,
    root: Option<&str>,
    legacy_user: Option<&str>,
) -> PathBuf {
    if let Some(dir) = individual {
        return PathBuf::from(dir);
    }
    match (root, user_id) {
        (Some(r), Some(id)) => Path::new(r).join(id),
        (Some(r), None) => PathBuf::from(r),
        (None, Some(id)) if legacy_user != Some(id) => {
            data_dir.join("users").join(id).join("images")
        }
        (None, _) => data_dir.join("images"),
    }
}

#[derive(Debug, Clone)]
pub struct UserImages {
    pub id: String,
    pub name: String,
    /// 個別設定の場所。
    pub individual: Option<String>,
    /// 実際の場所。
    pub dir: PathBuf,
    pub db_path: PathBuf,
    /// DB を読めた(読めないユーザーの場所は分からない)。
    pub readable: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Locations {
    /// 全体の画像の保存先。
    pub root: Option<String>,
    pub users: Vec<UserImages>,
}

fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
}

/// 個別の画像の場所(DB が無い新しいユーザーは「指定なし」)。読めなければ `Err`。
fn read_individual(db_path: &Path) -> rusqlite::Result<Option<String>> {
    if !db_path.exists() {
        return Ok(None);
    }
    let raw: Option<String> = open_read_only(db_path)?
        .query_row(
            "SELECT value_json FROM settings WHERE key = 'storage.images_dir'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
}

/// ユーザーの DB から、画像の相対パスの記録を読む(読み取り専用)。
pub fn read_image_rows(db_path: &Path) -> AppResult<Vec<ImageRow>> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let c = open_read_only(db_path)?;
    let mut stmt = c.prepare("SELECT file_path, thumb_path FROM images")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ImageRow {
                id: String::new(),
                job_id: String::new(),
                file_path: r.get(0)?,
                thumb_path: r.get(1)?,
                width: 0,
                height: 0,
                seed: 0,
                file_size: 0,
                is_favorite: false,
                created_at: 0,
                project_id: None,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 全ユーザーの画像の場所。ログインしていないユーザーの DB は読み取り専用で開く。
pub async fn all(db: &Db, paths: &AppPaths) -> AppResult<Locations> {
    let (root, legacy_user, users) = db
        .call_device(|c| {
            let global = settings::global_images(c)?;
            let mut stmt = c.prepare("SELECT id, name FROM app.users ORDER BY created_at")?;
            let users = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((global.0, global.1, users))
        })
        .await?;
    let data_dir = paths.data_dir.clone();
    let workspaces: Vec<_> = users
        .into_iter()
        .map(|(id, name)| {
            let db_path = paths.user_workspace(&id).join(USER_DB);
            (id, name, db_path)
        })
        .collect();
    let (r, l) = (root.clone(), legacy_user.clone());
    let users = tokio::task::spawn_blocking(move || {
        workspaces
            .into_iter()
            .map(|(id, name, db_path)| {
                let read = read_individual(&db_path);
                let readable = read.is_ok();
                let individual = read.ok().flatten();
                let dir = location(
                    &data_dir,
                    Some(&id),
                    individual.as_deref(),
                    r.as_deref(),
                    l.as_deref(),
                );
                UserImages {
                    id,
                    name,
                    individual,
                    dir,
                    db_path,
                    readable,
                }
            })
            .collect()
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Locations { root, users })
}

pub fn overlaps(a: &Path, b: &Path) -> bool {
    files::relation(a, b) != Relation::Distinct
}

/// 指定した場所が使えない理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// アプリのデータ領域の中(自分の既定の場所を除く)。
    InAppData,
    /// ほかのユーザーの画像の場所、または全体の保存先と重なる。
    InUse,
}

impl Rejected {
    pub fn error(self) -> AppError {
        AppError::validation(
            "newDir",
            match self {
                Self::InAppData => "アプリのデータ領域内は指定できません",
                Self::InUse => "ほかで使われている場所と重なるため、指定できません",
            },
        )
    }
}

fn in_app_data(data_dir: &Path, dest: &Path) -> bool {
    files::real_path(dest).starts_with(files::real_path(data_dir))
}

/// ユーザー `me` が個別の保存先として `dest` を指定できるか(ファイルシステムを調べる)。
pub fn check_individual(
    data_dir: &Path,
    all: &Locations,
    me: Option<&str>,
    dest: &Path,
) -> Option<Rejected> {
    let own_default = me.map(|id| data_dir.join("users").join(id).join("images"));
    let is_own_default = own_default.is_some_and(|d| files::relation(&d, dest) == Relation::Same);
    if !is_own_default && in_app_data(data_dir, dest) {
        return Some(Rejected::InAppData);
    }
    let others = all
        .users
        .iter()
        .filter(|u| Some(u.id.as_str()) != me)
        .any(|u| overlaps(&u.dir, dest));
    let root = all
        .root
        .as_deref()
        .is_some_and(|r| overlaps(Path::new(r), dest));
    (others || root).then_some(Rejected::InUse)
}

/// 全体の保存先として `root` を指定できるか。
pub fn check_root(data_dir: &Path, all: &Locations, root: &Path) -> Option<Rejected> {
    if in_app_data(data_dir, root) {
        return Some(Rejected::InAppData);
    }
    all.users
        .iter()
        .filter_map(|u| u.individual.as_deref())
        .any(|d| overlaps(Path::new(d), root))
        .then_some(Rejected::InUse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_follows_individual_then_root_then_standard() {
        let d = Path::new("/data");
        assert_eq!(
            location(d, Some("a"), Some("/mine"), Some("/r"), None),
            PathBuf::from("/mine")
        );
        assert_eq!(
            location(d, Some("a"), None, Some("/r"), Some("a")),
            PathBuf::from("/r").join("a")
        );
        assert_eq!(
            location(d, Some("a"), None, None, Some("a")),
            d.join("images")
        );
        assert_eq!(
            location(d, Some("b"), None, None, Some("a")),
            d.join("users").join("b").join("images")
        );
    }
}
