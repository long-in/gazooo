//! バックアップの取得(docs/35 §3・§4.1)。
//!
//! 1. 稼働中の DB から `VACUUM INTO` で一貫したスナップショットを作る(`snapshot`)。
//! 2. スナップショット上で OFF の区分・API キーを削除し、`VACUUM` で空きページからも消す(`prune`)。
//! 3. スナップショットと画像を ZIP に書く(`write_backup`)。画像は圧縮済みなので無圧縮で入れる。

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;
use zip::write::SimpleFileOptions;

use super::scope::{API_KEY_KEYS, Include, excluded_sections, tables};
use super::{
    Bytes, CANCELLED, Counts, DB_ENTRY, FORMAT, IMAGES_PREFIX, MANIFEST_NAME, Manifest,
    ManifestInclude, join_rel, map_io, map_zip, safe_rel,
};
use crate::error::{AppError, AppResult};
use crate::util::now_ms;

/// 稼働中の DB の一貫したスナップショットを `dest`(未作成のパス)に作る。WAL のままコピーしない。
pub fn snapshot(conn: &Connection, dest: &Path) -> AppResult<()> {
    let _ = fs::remove_file(dest);
    conn.execute("VACUUM INTO ?1", [dest.to_string_lossy().as_ref()])?;
    Ok(())
}

pub struct Pruned {
    pub counts: Counts,
    /// 画像保存先からの相対パス(`/` 区切り)。原寸・サムネイル。ギャラリーが OFF なら空。
    pub image_files: BTreeSet<String>,
}

/// スナップショットから、含めない区分と API キーを取り除く。取り除いた内容が空きページに残らないよう `VACUUM` し直す。
pub fn prune(snap: &Path, include: &Include, api_key: bool) -> AppResult<Pruned> {
    let conn = Connection::open(snap)?;
    conn.execute_batch("PRAGMA foreign_keys=OFF;")?;
    for section in excluded_sections(include) {
        for t in tables(section).iter().rev() {
            conn.execute(&format!("DELETE FROM {t}"), [])?;
        }
    }
    if !(include.settings && api_key) {
        for k in API_KEY_KEYS {
            conn.execute("DELETE FROM settings WHERE key = ?1", [k])?;
        }
    }
    let count = |sql: &str| -> AppResult<u64> {
        Ok(conn.query_row(sql, [], |r| r.get::<_, i64>(0))?.max(0) as u64)
    };
    let counts = Counts {
        settings: count("SELECT COUNT(*) FROM settings")?,
        conversations: count("SELECT COUNT(*) FROM assistant_conversations")?,
        jobs: count("SELECT COUNT(*) FROM jobs")?,
        images: count("SELECT COUNT(*) FROM images")?,
        presets: count("SELECT COUNT(*) FROM presets")?,
        projects: count("SELECT COUNT(*) FROM projects")?,
        inputs: 0,
    };
    let mut image_files = BTreeSet::new();
    if include.gallery {
        let mut stmt = conn.prepare("SELECT file_path, thumb_path FROM images")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (file, thumb) = row?;
            image_files.insert(file);
            image_files.insert(thumb);
        }
    }
    conn.execute_batch("VACUUM;")?;
    Ok(Pruned {
        counts,
        image_files,
    })
}

/// `inputs/` 直下のファイルの相対パス(参照画像・マスク)。
pub fn list_inputs(images_dir: &Path) -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir(images_dir.join("inputs")) {
        for e in rd.flatten() {
            if e.metadata().is_ok_and(|m| m.is_file()) {
                v.push(format!("inputs/{}", e.file_name().to_string_lossy()));
            }
        }
    }
    v.sort();
    v
}

pub struct WriteParams<'a> {
    pub snapshot: &'a Path,
    pub schema_version: u32,
    pub app_version: &'a str,
    pub include: Include,
    pub api_key: bool,
    pub images_dir: &'a Path,
    pub dest: &'a Path,
    pub cancel: &'a AtomicBool,
    pub progress: &'a (dyn Fn(&str, u64, u64) + Sync),
}

#[derive(Debug)]
pub struct Written {
    pub images: u64,
    pub skipped: u64,
    pub bytes: u64,
}

pub fn write_backup(p: &WriteParams) -> AppResult<Written> {
    let res = write_inner(p);
    if res.is_err() {
        let _ = fs::remove_file(p.dest);
    }
    res
}

fn write_inner(p: &WriteParams) -> AppResult<Written> {
    (p.progress)("snapshot", 0, 1);
    let api_key = p.include.settings && p.api_key;
    let mut pruned = prune(p.snapshot, &p.include, api_key)?;

    let mut files = pruned.image_files.clone();
    if p.include.gallery {
        let inputs = list_inputs(p.images_dir);
        pruned.counts.inputs = inputs.len() as u64;
        files.extend(inputs);
    }
    let total = files.len() as u64 + 1;

    let out = File::create(p.dest).map_err(map_io)?;
    let mut zip = zip::ZipWriter::new(io::BufWriter::new(out));
    let deflated = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let stored = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);

    zip.start_file(DB_ENTRY, deflated).map_err(map_zip)?;
    let mut db = File::open(p.snapshot).map_err(map_io)?;
    io::copy(&mut db, &mut zip).map_err(map_io)?;
    (p.progress)("files", 1, total);

    let (mut written, mut skipped, mut bytes) = (0u64, 0u64, 0u64);
    for (i, rel) in files.iter().enumerate() {
        if p.cancel.load(Ordering::SeqCst) {
            return Err(AppError::backup(CANCELLED, "バックアップを中止しました"));
        }
        let Some(path) = join_rel(p.images_dir, rel).filter(|_| safe_rel(rel)) else {
            skipped += 1;
            continue;
        };
        let Ok(mut f) = File::open(&path) else {
            skipped += 1; // DB にあるがディスクに無い画像は、飛ばして件数だけ記録する
            continue;
        };
        zip.start_file(format!("{IMAGES_PREFIX}{rel}"), stored)
            .map_err(map_zip)?;
        bytes += io::copy(&mut f, &mut zip).map_err(map_io)?;
        written += 1;
        (p.progress)("files", i as u64 + 2, total);
    }

    let manifest = Manifest {
        format: FORMAT,
        app: "gazooo".into(),
        app_version: p.app_version.into(),
        schema_version: p.schema_version,
        created_at: now_ms(),
        include: ManifestInclude {
            settings: p.include.settings,
            gallery: p.include.gallery,
            presets: p.include.presets,
            api_key,
        },
        counts: pruned.counts,
        bytes: Bytes { images: bytes },
    };
    zip.start_file(MANIFEST_NAME, deflated).map_err(map_zip)?;
    zip.write_all(&serde_json::to_vec_pretty(&manifest)?)
        .map_err(map_io)?;
    zip.finish().map_err(map_zip)?.flush().map_err(map_io)?;
    Ok(Written {
        images: written,
        skipped,
        bytes,
    })
}
