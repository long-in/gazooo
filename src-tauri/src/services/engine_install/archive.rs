//! アーカイブの安全な展開(docs/23 §3)。
//!
//! `..`・絶対パス・ドライブ指定・展開先の外を指すリンクを含むアーカイブは拒否する。
//! 同期処理なので、呼び出し側は `spawn_blocking` で実行する。

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{AppError, AppResult};

fn unsafe_archive(what: &str) -> AppError {
    AppError::engine_proc(
        "ENGINE_INSTALL_ARCHIVE_UNSAFE",
        format!("アーカイブに安全でない項目があるため展開しませんでした({what})"),
    )
}

fn cancelled() -> AppError {
    AppError::engine_proc("ENGINE_INSTALL_CANCELLED", "導入を中止しました")
}

fn io_error(e: io::Error) -> AppError {
    if e.kind() == io::ErrorKind::StorageFull {
        AppError::engine_proc("ENGINE_INSTALL_DISK_FULL", "ディスクの空き容量が足りません")
    } else {
        AppError::engine_proc(
            "ENGINE_INSTALL_EXTRACT_FAILED",
            format!("ファイルの展開に失敗しました: {e}"),
        )
    }
}

/// 相対パスとして安全か(通常の要素だけ)。`strip` 個の先頭要素を取り除いた結果を返す。
fn safe_relative(path: &Path, strip: usize) -> AppResult<Option<PathBuf>> {
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            // `C:` のようなドライブ指定は、OS によって通常の名前として通ってしまうので、どの OS でも拒否する
            Component::Normal(p) if p.to_string_lossy().contains(':') => {
                return Err(unsafe_archive(&path.to_string_lossy()));
            }
            Component::Normal(p) => parts.push(p.to_os_string()),
            Component::CurDir => {}
            _ => return Err(unsafe_archive(&path.to_string_lossy())),
        }
    }
    if parts.len() <= strip {
        return Ok(None);
    }
    Ok(Some(parts.into_iter().skip(strip).collect()))
}

/// リンク先が、展開先の内側を指す相対パスか。`entry_dir` はリンク自身の置かれるディレクトリ(展開先からの相対)。
fn link_target_inside(entry_dir: &Path, target: &Path) -> bool {
    let mut depth = entry_dir.components().count() as i64;
    for c in target.components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

/// zip を `dest` に展開する。先頭の 1 階層(GitHub の `<repo>-<commit>/`)は取り除く。
pub fn extract_zip(
    zip_path: &Path,
    dest: &Path,
    strip: usize,
    cancel: &AtomicBool,
) -> AppResult<()> {
    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        AppError::engine_proc(
            "ENGINE_INSTALL_EXTRACT_FAILED",
            format!("zip を開けません: {e}"),
        )
    })?;
    fs::create_dir_all(dest).map_err(io_error)?;
    for i in 0..archive.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let mut entry = archive.by_index(i).map_err(|e| {
            AppError::engine_proc(
                "ENGINE_INSTALL_EXTRACT_FAILED",
                format!("zip の読み取りに失敗しました: {e}"),
            )
        })?;
        // シンボリックリンクは扱わない(GitHub のソース zip には通常含まれない)
        if let Some(mode) = entry.unix_mode()
            && mode & 0o170000 == 0o120000
        {
            return Err(unsafe_archive("シンボリックリンク"));
        }
        let name = entry
            .name()
            .map_err(|_| unsafe_archive("名前を解釈できない項目"))?
            .replace('\\', "/");
        let Some(rel) = safe_relative(Path::new(&name), strip)? else {
            continue;
        };
        let out = dest.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(io_error)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let mut f = fs::File::create(&out).map_err(io_error)?;
        io::copy(&mut entry, &mut f).map_err(io_error)?;
    }
    Ok(())
}

/// `.tar.gz` を `dest` に展開する(python-build-standalone の `install_only`)。
pub fn extract_tar_gz(path: &Path, dest: &Path, cancel: &AtomicBool) -> AppResult<()> {
    let file = fs::File::open(path)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(io::BufReader::new(file)));
    archive.set_preserve_permissions(true);
    archive.set_unpack_xattrs(false);
    fs::create_dir_all(dest).map_err(io_error)?;
    for entry in archive.entries().map_err(io_error)? {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let mut entry = entry.map_err(io_error)?;
        let path = entry.path().map_err(io_error)?.into_owned();
        let Some(rel) = safe_relative(&path, 0)? else {
            continue;
        };
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            let target = entry
                .link_name()
                .map_err(io_error)?
                .ok_or_else(|| unsafe_archive(&path.to_string_lossy()))?
                .into_owned();
            let entry_dir = rel.parent().unwrap_or(Path::new(""));
            // ハードリンクのリンク先は展開先からの相対、シンボリックリンクはリンクの位置からの相対
            let ok = if kind.is_hard_link() {
                link_target_inside(Path::new(""), &target)
            } else {
                link_target_inside(entry_dir, &target)
            };
            if !ok {
                return Err(unsafe_archive(&path.to_string_lossy()));
            }
        } else if !(kind.is_file() || kind.is_dir()) {
            // デバイスファイルなどは作らない
            continue;
        }
        // `unpack_in` は展開先の外へ出るパスを拒否する(false を返す)
        let ok = entry.unpack_in(dest).map_err(io_error)?;
        if !ok {
            return Err(unsafe_archive(&path.to_string_lossy()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gazooo-arc-{name}-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let f = fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn zip_strips_top_directory() {
        let d = tmp("zip-ok");
        let z = d.join("a.zip");
        make_zip(
            &z,
            &[
                ("repo-abc/main.py", b"print(1)"),
                ("repo-abc/sub/x.txt", b"x"),
            ],
        );
        let out = d.join("out");
        extract_zip(&z, &out, 1, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(out.join("main.py")).unwrap(), b"print(1)");
        assert_eq!(fs::read(out.join("sub").join("x.txt")).unwrap(), b"x");
    }

    #[test]
    fn zip_rejects_traversal_and_absolute() {
        let d = tmp("zip-bad");
        for bad in ["repo/../../evil.txt", "/abs/evil.txt", "C:/evil.txt"] {
            let z = d.join("b.zip");
            make_zip(&z, &[(bad, b"x")]);
            let out = d.join("out");
            let r = extract_zip(&z, &out, 0, &AtomicBool::new(false));
            assert!(r.is_err(), "{bad}");
            assert!(!d.join("evil.txt").exists());
        }
    }

    #[test]
    fn zip_honours_cancel() {
        let d = tmp("zip-cancel");
        let z = d.join("a.zip");
        make_zip(&z, &[("r/a", b"a")]);
        let r = extract_zip(&z, &d.join("out"), 1, &AtomicBool::new(true));
        assert!(matches!(
            r,
            Err(AppError::EngineProc {
                code: "ENGINE_INSTALL_CANCELLED",
                ..
            })
        ));
    }

    fn make_tar_gz(
        path: &Path,
        build: impl FnOnce(&mut tar::Builder<flate2::write::GzEncoder<fs::File>>),
    ) {
        let f = fs::File::create(path).unwrap();
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(
            f,
            flate2::Compression::fast(),
        ));
        build(&mut b);
        b.into_inner().unwrap().finish().unwrap();
    }

    fn add_file(b: &mut tar::Builder<flate2::write::GzEncoder<fs::File>>, name: &str, data: &[u8]) {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, name, data).unwrap();
    }

    #[test]
    fn tar_extracts_files_with_internal_symlink() {
        let d = tmp("tar-ok");
        let t = d.join("p.tar.gz");
        make_tar_gz(&t, |b| {
            add_file(b, "python/python.exe", b"exe");
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            h.set_mode(0o777);
            b.append_link(&mut h, "python/bin/python3", "python3.12")
                .unwrap();
        });
        let out = d.join("out");
        let r = extract_tar_gz(&t, &out, &AtomicBool::new(false));
        // Windows は管理者権限なしでシンボリックリンクを作れないことがある。安全検査を通ったかだけを見る
        if cfg!(windows) {
            let _ = r;
        } else {
            r.unwrap();
        }
        assert_eq!(
            fs::read(out.join("python").join("python.exe")).unwrap(),
            b"exe"
        );
    }

    #[test]
    fn tar_rejects_escaping_symlink_and_traversal() {
        let d = tmp("tar-bad");
        let t = d.join("s.tar.gz");
        make_tar_gz(&t, |b| {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            b.append_link(&mut h, "python/evil", "../../outside")
                .unwrap();
        });
        let r = extract_tar_gz(&t, &d.join("out"), &AtomicBool::new(false));
        assert!(matches!(
            r,
            Err(AppError::EngineProc {
                code: "ENGINE_INSTALL_ARCHIVE_UNSAFE",
                ..
            })
        ));

        // `..` を含むファイル名は、tar の書き込み側が拒否するので、ヘッダを直接作る
        let t2 = d.join("t.tar.gz");
        make_tar_gz(&t2, |b| {
            let mut h = tar::Header::new_gnu();
            h.set_size(1);
            h.set_mode(0o644);
            let name = b"python/../../evil.txt";
            h.as_old_mut().name[..name.len()].copy_from_slice(name);
            h.set_cksum();
            b.append(&h, &b"x"[..]).unwrap();
        });
        let r = extract_tar_gz(&t2, &d.join("out2"), &AtomicBool::new(false));
        assert!(r.is_err());
        assert!(!d.join("evil.txt").exists());
    }

    #[test]
    fn link_target_depth_rules() {
        assert!(link_target_inside(
            Path::new("python/bin"),
            Path::new("python3.12")
        ));
        assert!(link_target_inside(
            Path::new("python/bin"),
            Path::new("../lib/x")
        ));
        assert!(!link_target_inside(
            Path::new("python/bin"),
            Path::new("../../../x")
        ));
        assert!(!link_target_inside(
            Path::new("python"),
            Path::new("/etc/passwd")
        ));
    }
}
