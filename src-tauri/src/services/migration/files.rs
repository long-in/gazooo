//! データ移行のファイル操作(docs/51 §4)。DB・エンジンに依存しない。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use crate::error::{AppError, AppResult};
use crate::services::images::Trasher;
use crate::services::jobs::join_rel;

/// コピー中の一時ファイルの接尾辞。
pub const TEMP_SUFFIX: &str = ".gazooo-migrating";
const CHUNK: usize = 4 * 1024 * 1024;

/// モデルの移行対象(`models` 直下のフォルダ。docs/51 §4.3)。
pub const MODEL_FOLDERS: &[&str] = &[
    "checkpoints",
    "clip",
    "text_encoders",
    "vae",
    "unet",
    "diffusion_models",
    "loras",
    "controlnet",
    "ultralytics",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// `/` 区切りの相対パス。
    pub rel: String,
    pub size: u64,
}

#[derive(Debug, Default)]
pub struct Scan {
    pub items: Vec<Item>,
    /// 記録はあるがファイルが無い数(画像)。
    pub missing: usize,
    /// 移らないフォルダ(モデル)。
    pub skipped_folders: Vec<String>,
    /// たどらなかったリンクの数。
    pub skipped_links: usize,
}

fn storage_err(path: &Path, e: impl std::fmt::Display) -> AppError {
    AppError::Storage(format!("{}: {e}", path.display()))
}

fn cancelled() -> AppError {
    AppError::migration("STORAGE_CANCELLED", "移行を中止しました")
}

/// 指定した相対パスのうち、ファイルがあるものを対象にする(画像)。
pub fn scan_list(root: &Path, rels: &[String]) -> Scan {
    let mut out = Scan::default();
    let mut seen = std::collections::HashSet::new();
    for rel in rels {
        if !seen.insert(rel.as_str()) {
            continue;
        }
        match std::fs::metadata(join_rel(root, rel)) {
            Ok(m) if m.is_file() => out.items.push(Item {
                rel: rel.clone(),
                size: m.len(),
            }),
            _ => out.missing += 1,
        }
    }
    out
}

/// ComfyUI が空のフォルダに置く目印(`put_checkpoints_here` など)。
fn is_placeholder(name: &str, size: u64) -> bool {
    size == 0 && name.starts_with("put_") && name.ends_with("_here")
}

fn walk(dir: &Path, rel: &str, out: &mut Scan) -> AppResult<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| storage_err(dir, e))? {
        let entry = entry.map_err(|e| storage_err(dir, e))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| storage_err(&path, e))?;
        if kind.is_symlink() {
            out.skipped_links += 1;
            continue;
        }
        let Ok(name) = entry.file_name().into_string() else {
            return Err(storage_err(&path, "扱えない文字を含むファイル名です"));
        };
        let child = format!("{rel}/{name}");
        if kind.is_dir() {
            walk(&path, &child, out)?;
            continue;
        }
        let size = entry.metadata().map_err(|e| storage_err(&path, e))?.len();
        // 前回の中断で残った一時ファイルは移さない
        if !is_placeholder(&name, size) && !name.ends_with(TEMP_SUFFIX) {
            out.items.push(Item { rel: child, size });
        }
    }
    Ok(())
}

/// 目印以外のファイル(またはリンク)が 1 つでもあるか。
fn has_content(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| match e.file_type() {
        Ok(k) if k.is_dir() => has_content(&e.path()),
        Ok(_) => !e
            .metadata()
            .is_ok_and(|m| is_placeholder(&e.file_name().to_string_lossy(), m.len())),
        Err(_) => false,
    })
}

/// `models` フォルダを走査する。
pub fn scan_models(root: &Path) -> AppResult<Scan> {
    let mut out = Scan::default();
    let entries = std::fs::read_dir(root).map_err(|e| storage_err(root, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| storage_err(root, e))?;
        let kind = entry
            .file_type()
            .map_err(|e| storage_err(&entry.path(), e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let target = MODEL_FOLDERS.iter().any(|f| {
            if cfg!(any(windows, target_os = "macos")) {
                f.eq_ignore_ascii_case(&name)
            } else {
                *f == name
            }
        });
        if kind.is_symlink() {
            out.skipped_links += 1;
        } else if kind.is_dir() {
            if target {
                walk(&entry.path(), &name, &mut out)?;
            } else if has_content(&entry.path()) {
                out.skipped_folders.push(name);
            }
        }
    }
    out.items.sort_by(|a, b| a.rel.cmp(&b.rel));
    out.skipped_folders.sort();
    Ok(out)
}

/// 実在する部分をリンク解決した絶対パス(まだ無い部分はそのまま後ろに付ける)。
pub fn real_path(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cur = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(cur) {
            return rest.iter().rev().fold(real, |p, c| p.join(c));
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                cur = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// いちばん近い実在するフォルダ(空き容量・書き込みの確認に使う)。
pub fn existing_ancestor(path: &Path) -> Option<&Path> {
    path.ancestors().find(|p| p.is_dir())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Same,
    /// 片方がもう片方の内側。
    Nested,
    Distinct,
}

/// 2 つのフォルダの関係を、実体のパスで判定する。Windows・macOS では大文字・小文字を区別しない。
pub fn relation(a: &Path, b: &Path) -> Relation {
    fn parts(p: &Path) -> Vec<String> {
        real_path(p)
            .components()
            .map(|c| {
                let s = c.as_os_str().to_string_lossy().into_owned();
                if cfg!(any(windows, target_os = "macos")) {
                    s.to_lowercase()
                } else {
                    s
                }
            })
            .collect()
    }
    let (a, b) = (parts(a), parts(b));
    if a == b {
        Relation::Same
    } else if a.starts_with(&b) || b.starts_with(&a) {
        Relation::Nested
    } else {
        Relation::Distinct
    }
}

#[derive(Debug, Default)]
pub struct Analysis {
    /// コピーするもの。
    pub copy: Vec<Item>,
    /// 移行先に同じサイズのファイルがあり、コピーしないもの(元は残す)。
    pub same_size: Vec<Item>,
    /// 移行先にサイズの違う同名がある(開始できない)。
    pub conflicts: Vec<String>,
    pub copy_bytes: u64,
}

pub fn analyze(items: &[Item], dest: &Path) -> Analysis {
    let mut out = Analysis::default();
    for item in items {
        match std::fs::symlink_metadata(join_rel(dest, &item.rel)) {
            Err(_) => {
                out.copy_bytes += item.size;
                out.copy.push(item.clone());
            }
            Ok(m) if m.is_file() && m.len() == item.size => out.same_size.push(item.clone()),
            Ok(_) => out.conflicts.push(item.rel.clone()),
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct CopyProgress<'a> {
    pub done_bytes: u64,
    pub done_files: usize,
    pub current: &'a str,
}

#[derive(Debug, Default)]
pub struct Copied {
    /// 移行先に作ったファイル(取り消し用)。
    pub created: Vec<PathBuf>,
    /// コピーできた元ファイル(切り替え後に整理する)。
    pub sources: Vec<PathBuf>,
}

/// 作ったコピーを消す。
pub fn remove_created(created: &[PathBuf]) {
    for f in created {
        let _ = std::fs::remove_file(f);
    }
}

fn copy_one(
    src: &Path,
    temp: &Path,
    expected: u64,
    cancel: &AtomicBool,
    on_chunk: &mut dyn FnMut(u64),
) -> AppResult<()> {
    let mut input = std::fs::File::open(src).map_err(|e| storage_err(src, e))?;
    let modified: Option<SystemTime> = input.metadata().ok().and_then(|m| m.modified().ok());
    let mut output = std::fs::File::create(temp).map_err(|e| write_err(temp, e))?;
    let mut buf = vec![0u8; CHUNK];
    let mut written = 0u64;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        let n = input.read(&mut buf).map_err(|e| storage_err(src, e))?;
        if n == 0 {
            break;
        }
        output
            .write_all(&buf[..n])
            .map_err(|e| write_err(temp, e))?;
        written += n as u64;
        on_chunk(n as u64);
    }
    if written != expected {
        return Err(storage_err(src, "コピー後のサイズが一致しません"));
    }
    // 更新日時を引き継ぐ(失敗しても支障はない)
    if let Some(t) = modified {
        let _ = output.set_modified(t);
    }
    output.sync_all().map_err(|e| write_err(temp, e))?;
    let size = output.metadata().map_err(|e| storage_err(temp, e))?.len();
    if size != expected {
        return Err(storage_err(temp, "コピー後のサイズが一致しません"));
    }
    Ok(())
}

fn write_err(path: &Path, e: std::io::Error) -> AppError {
    if e.kind() == std::io::ErrorKind::StorageFull {
        AppError::migration("STORAGE_NO_SPACE", "移行先の空き容量が足りません")
    } else {
        storage_err(path, e)
    }
}

/// 一時名にコピーして確認し、正式な名前に変える。失敗・中止なら、今回作ったものを消す。
pub fn copy_all(
    items: &[Item],
    source: &Path,
    dest: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(CopyProgress<'_>),
) -> AppResult<Copied> {
    let mut out = Copied::default();
    let mut done_bytes = 0u64;
    for (index, item) in items.iter().enumerate() {
        let src = join_rel(source, &item.rel);
        let dst = join_rel(dest, &item.rel);
        let mut temp_name = dst.file_name().unwrap_or_default().to_os_string();
        temp_name.push(TEMP_SUFFIX);
        let temp = dst.with_file_name(temp_name);
        let result = (|| {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| write_err(parent, e))?;
            }
            copy_one(&src, &temp, item.size, cancel, &mut |n| {
                done_bytes += n;
                on_progress(CopyProgress {
                    done_bytes,
                    done_files: index,
                    current: &item.rel,
                });
            })?;
            // 上書きはしない(事前確認のあとに現れたファイルは残す)
            if dst.exists() {
                return Err(AppError::migration(
                    "STORAGE_CONFLICT",
                    format!("移行先に同名のファイルがあります: {}", item.rel),
                ));
            }
            std::fs::rename(&temp, &dst).map_err(|e| write_err(&dst, e))
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_file(&temp);
            remove_created(&out.created);
            return Err(e);
        }
        on_progress(CopyProgress {
            done_bytes,
            done_files: index + 1,
            current: &item.rel,
        });
        out.created.push(dst);
        out.sources.push(src);
    }
    Ok(out)
}

/// 元ファイルをゴミ箱へ移す。移せなかった数を返す。
pub fn trash_all(
    sources: &[PathBuf],
    trasher: &dyn Trasher,
    on_progress: &mut dyn FnMut(usize),
) -> usize {
    let mut failed = 0;
    for (i, path) in sources.iter().enumerate() {
        if trasher.trash(path).is_err() {
            tracing::warn!(path = %path.display(), "failed to move old file to trash");
            failed += 1;
        }
        on_progress(i + 1);
    }
    failed
}
