//! バックアップの検査と復元(docs/35 §4.2・§4.3)。
//!
//! - 検査(`stage`): ZIP のパスを検証し、DB を `restore-staging/` に展開して `integrity_check` する。
//! - 画像の展開(`extract_images`): 追加のみ。画像保存先へ直接書く。
//! - DB の入れ替え(`apply`): 稼働中の接続に `ATTACH` して、**1 つのトランザクション** で区分ごとに置き換える。

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{Connection, OpenFlags};
use zip::ZipArchive;

use super::scope::{API_KEY_KEYS, Include, tables};
use super::{
    CANCELLED, DB_ENTRY, FORMAT, IMAGES_PREFIX, MANIFEST_NAME, Manifest, join_rel, map_io, safe_rel,
};
use crate::error::{AppError, AppResult};

/// DB の展開サイズの上限(ZIP 爆弾の対策)。
const MAX_DB_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

fn corrupt(msg: impl std::fmt::Display) -> AppError {
    AppError::backup(
        "BACKUP_CORRUPT",
        format!("バックアップが壊れています: {msg}"),
    )
}

fn unsupported(msg: impl std::fmt::Display) -> AppError {
    AppError::backup("BACKUP_UNSUPPORTED", msg.to_string())
}

fn open_zip(src: &Path) -> AppResult<ZipArchive<File>> {
    let f = File::open(src).map_err(map_io)?;
    ZipArchive::new(f).map_err(corrupt)
}

/// ZIP の全エントリ名を検証する。`images/` 配下は安全な相対パスのみ。シンボリックリンクは拒否。
fn validate_entries(zip: &mut ZipArchive<File>) -> AppResult<()> {
    for i in 0..zip.len() {
        let e = zip.by_index_raw(i).map_err(corrupt)?;
        let name = e.name().map_err(corrupt)?.into_owned();
        if e.is_symlink() {
            return Err(corrupt(format!("シンボリックリンクは使えません: {name}")));
        }
        if let Some(rel) = name.strip_prefix(IMAGES_PREFIX) {
            if !e.is_dir() && !safe_rel(rel) {
                return Err(corrupt(format!("不正なパスが含まれています: {name}")));
            }
        } else if name != MANIFEST_NAME && name != DB_ENTRY && !e.is_dir() {
            return Err(corrupt(format!("想定外のファイルが含まれています: {name}")));
        }
    }
    Ok(())
}

fn read_manifest(zip: &mut ZipArchive<File>, current_schema: u32) -> AppResult<Manifest> {
    let mut entry = zip
        .by_name(MANIFEST_NAME)
        .map_err(|_| unsupported("gazooo のバックアップファイルではありません"))?;
    let mut buf = Vec::new();
    (&mut entry)
        .take(MAX_MANIFEST_BYTES)
        .read_to_end(&mut buf)
        .map_err(corrupt)?;
    let m: Manifest = serde_json::from_slice(&buf).map_err(corrupt)?;
    if m.app != "gazooo" || m.format != FORMAT {
        return Err(unsupported("対応していないバックアップの形式です"));
    }
    if m.schema_version > current_schema {
        return Err(unsupported(
            "新しいバージョンのアプリで作成されたバックアップです。アプリを更新してから復元してください",
        ));
    }
    Ok(m)
}

/// バックアップを検査して、DB を `staging_dir/gazooo.db` に展開する。現在のデータは変更しない。
pub fn stage(
    src: &Path,
    staging_dir: &Path,
    current_schema: u32,
) -> AppResult<(Manifest, PathBuf)> {
    let mut zip = open_zip(src)?;
    validate_entries(&mut zip)?;
    let manifest = read_manifest(&mut zip, current_schema)?;

    let _ = fs::remove_dir_all(staging_dir);
    fs::create_dir_all(staging_dir).map_err(map_io)?;
    let staged = staging_dir.join("gazooo.db");
    {
        let mut entry = zip
            .by_name(DB_ENTRY)
            .map_err(|_| corrupt("DB が含まれていません"))?;
        let mut out = File::create(&staged).map_err(map_io)?;
        let n = io::copy(&mut (&mut entry).take(MAX_DB_BYTES + 1), &mut out).map_err(map_io)?;
        if n > MAX_DB_BYTES {
            return Err(corrupt("DB が大きすぎます"));
        }
    }
    let check = || -> Result<(String, u32), rusqlite::Error> {
        let c = Connection::open_with_flags(&staged, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let ok: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        let v: u32 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        Ok((ok, v))
    };
    let (ok, version) = check().map_err(corrupt)?;
    if ok != "ok" {
        return Err(corrupt("DB の整合性チェックに失敗しました"));
    }
    if version > current_schema {
        return Err(unsupported(
            "新しいバージョンのアプリで作成されたバックアップです。アプリを更新してから復元してください",
        ));
    }
    Ok((manifest, staged))
}

/// `images/…` を `target` へ **追加のみ** で展開する(同名は上書き)。展開したファイル数を返す。
pub fn extract_images(
    src: &Path,
    target: &Path,
    cancel: &AtomicBool,
    progress: &(dyn Fn(&str, u64, u64) + Sync),
) -> AppResult<u64> {
    let mut zip = open_zip(src)?;
    validate_entries(&mut zip)?;
    fs::create_dir_all(target).map_err(map_io)?;
    let total = zip.len() as u64;
    let mut done = 0u64;
    for i in 0..zip.len() {
        if cancel.load(Ordering::SeqCst) {
            return Err(AppError::backup(CANCELLED, "復元を中止しました"));
        }
        let mut e = zip.by_index(i).map_err(corrupt)?;
        let name = e.name().map_err(corrupt)?.into_owned();
        let Some(rel) = name.strip_prefix(IMAGES_PREFIX) else {
            continue;
        };
        if e.is_dir() || rel.is_empty() {
            continue;
        }
        let dest = join_rel(target, rel).ok_or_else(|| corrupt(format!("不正なパス: {name}")))?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(map_io)?;
        }
        let size = e.size();
        let mut out = File::create(&dest).map_err(map_io)?;
        io::copy(&mut (&mut e).take(size), &mut out).map_err(map_io)?;
        done += 1;
        progress("files", i as u64 + 1, total);
    }
    Ok(done)
}

fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn columns(conn: &Connection, schema: &str, table: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA {schema}.table_info({})", quote(table)))?;
    let cols = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(cols)
}

/// 両方に存在する列(古いバックアップに無い列は、現在のスキーマの既定値になる)。
fn common_columns(conn: &Connection, table: &str) -> AppResult<Vec<String>> {
    let bk: HashSet<String> = columns(conn, "bk", table)?.into_iter().collect();
    Ok(columns(conn, "main", table)?
        .into_iter()
        .filter(|c| bk.contains(c))
        .collect())
}

/// ステージングした DB の `settings` から、バックアップ側の画像保存先を読む(未設定・null は `None`)。
pub fn staged_images_dir(staged: &Path) -> AppResult<Option<String>> {
    let c = Connection::open_with_flags(staged, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let raw: Option<String> = c
        .query_row(
            "SELECT value_json FROM settings WHERE key = 'storage.images_dir'",
            [],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(raw
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_str().map(str::to_owned)))
}

/// 稼働中の DB に、選んだ区分だけをステージングの DB の内容で置き換える。失敗すれば何も変わらない。
pub fn apply(
    conn: &mut Connection,
    staged: &Path,
    include: &Include,
    restore_api_key: bool,
) -> AppResult<()> {
    conn.execute(
        "ATTACH DATABASE ?1 AS bk",
        [staged.to_string_lossy().as_ref()],
    )?;
    let res = apply_inner(conn, include, restore_api_key);
    let _ = conn.execute_batch("DETACH DATABASE bk");
    res
}

fn bk_has_table(conn: &Connection, table: &str) -> AppResult<bool> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM bk.sqlite_master WHERE type='table' AND name=?1",
        [table],
        |r| r.get::<_, i64>(0),
    )? > 0)
}

fn apply_inner(conn: &mut Connection, include: &Include, restore_api_key: bool) -> AppResult<()> {
    let tx = conn.transaction()?;
    let [k1, k2] = API_KEY_KEYS;
    let restore_keys = include.settings && restore_api_key;

    // 先に全部消す(子 → 親)
    for section in include.sections() {
        for t in tables(section).iter().rev() {
            if *t == "settings" {
                // 現在の API キーは、復元するときに置き換えるまで残す
                tx.execute(
                    "DELETE FROM main.settings WHERE key NOT IN (?1, ?2)",
                    [k1, k2],
                )?;
            } else {
                tx.execute(&format!("DELETE FROM main.{}", quote(t)), [])?;
            }
        }
    }
    // 挿入(親 → 子)
    for section in include.sections() {
        for t in tables(section) {
            if !bk_has_table(&tx, t)? {
                continue; // 古いバックアップに無いテーブル(空のまま)
            }
            let cols = common_columns(&tx, t)?
                .iter()
                .map(|c| quote(c))
                .collect::<Vec<_>>()
                .join(", ");
            if cols.is_empty() {
                continue;
            }
            let q = quote(t);
            if *t == "settings" {
                tx.execute(
                    &format!(
                        "INSERT OR REPLACE INTO main.{q} ({cols}) SELECT {cols} FROM bk.{q} WHERE key NOT IN (?1, ?2)"
                    ),
                    [k1, k2],
                )?;
                if restore_keys {
                    tx.execute(
                        &format!(
                            "INSERT OR REPLACE INTO main.{q} ({cols}) SELECT {cols} FROM bk.{q} WHERE key IN (?1, ?2)"
                        ),
                        [k1, k2],
                    )?;
                }
            } else {
                tx.execute(
                    &format!("INSERT OR REPLACE INTO main.{q} ({cols}) SELECT {cols} FROM bk.{q}"),
                    [],
                )?;
            }
        }
    }
    if include.gallery {
        // 取得時点の待機中・実行中のジョブが、再起動後に勝手に生成を始めないよう取り消し済みにする
        tx.execute(
            "UPDATE main.jobs SET status='cancelled', finished_at=COALESCE(finished_at, created_at)
             WHERE status IN ('queued','running')",
            [],
        )?;
    }
    tx.commit()?;
    Ok(())
}

// ---------------------------------------------------------------- 孤立ファイル

/// 文字列中の `inputs/<ファイル名>` の参照を集める(JSON のどこにあっても拾う。保守的に残す側へ倒す)。
fn scan_input_refs(text: &str, out: &mut HashSet<String>) {
    let mut rest = text;
    while let Some(pos) = rest.find("inputs/") {
        let tail = &rest[pos..];
        let end = tail["inputs/".len()..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
            .map_or(tail.len(), |n| n + "inputs/".len());
        out.insert(tail[..end].to_string());
        rest = &tail[end.max(1)..];
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphanReport {
    pub count: u64,
    pub bytes: u64,
}

/// 画像保存先のうち、`images_schema` の `images` とその `jobs`・`presets_schema` の `presets` が参照しないファイル。
/// 対象は想定のレイアウト(`YYYY/MM/`・`thumbs/`・`inputs/`)の画像ファイルのみ。
pub fn scan_orphans(
    conn: &Connection,
    images_schema: &str,
    presets_schema: &str,
    images_dir: &Path,
) -> AppResult<Vec<(PathBuf, u64)>> {
    let mut referenced: HashSet<String> = HashSet::new();
    {
        let mut stmt = conn.prepare(&format!(
            "SELECT file_path, thumb_path FROM {images_schema}.images"
        ))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (a, b) = row?;
            referenced.insert(a);
            referenced.insert(b);
        }
    }
    for (schema, table) in [(images_schema, "jobs"), (presets_schema, "presets")] {
        let mut stmt = conn.prepare(&format!("SELECT params_json FROM {schema}.{table}"))?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for row in rows {
            scan_input_refs(&row?, &mut referenced);
        }
    }

    let mut found = Vec::new();
    walk(images_dir, "", 0, &referenced, &mut found);
    Ok(found)
}

fn is_year_dir(name: &str) -> bool {
    name.len() == 4 && name.bytes().all(|b| b.is_ascii_digit())
}

fn walk(
    dir: &Path,
    rel: &str,
    depth: usize,
    referenced: &HashSet<String>,
    out: &mut Vec<(PathBuf, u64)>,
) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let child = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            let ok = match depth {
                0 => name == "thumbs" || name == "inputs" || is_year_dir(&name),
                1 => {
                    is_year_dir(rel) && name.len() == 2 && name.bytes().all(|b| b.is_ascii_digit())
                }
                _ => false,
            };
            if ok {
                walk(&e.path(), &child, depth + 1, referenced, out);
            }
        } else if meta.is_file()
            && ((depth == 1 && (rel == "thumbs" || rel == "inputs")) || depth == 2)
        {
            let ext = Path::new(&name)
                .extension()
                .and_then(|x| x.to_str())
                .map(str::to_ascii_lowercase);
            if matches!(ext.as_deref(), Some("png" | "jpg" | "jpeg" | "webp"))
                && !referenced.contains(&child)
            {
                out.push((e.path(), meta.len()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_refs_are_found_anywhere_in_json() {
        let mut out = HashSet::new();
        scan_input_refs(
            r#"{"controlnet":{"units":[{"image":"inputs/abc-1.png"}]},"inpaint":{"mask":"inputs/m_2.png"}}"#,
            &mut out,
        );
        assert!(out.contains("inputs/abc-1.png"));
        assert!(out.contains("inputs/m_2.png"));
        assert_eq!(out.len(), 2);
    }
}
