//! ログ一式の ZIP 書き出し(docs/31 §5)。

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;

use crate::error::{AppError, AppResult};

fn zip_err(e: zip::result::ZipError) -> AppError {
    AppError::internal(format!("ZIP の作成に失敗しました: {e}"))
}

/// `dir` 直下のファイル(アプリのログ・`comfyui.log`・LLM のログ)と `info.txt` を `dest` の ZIP にまとめる。
/// 書き込み中のログも読み取りだけで取り込む(ログの出力は止めない)。
pub fn write_zip(dir: &Path, dest: &Path, info: &str) -> AppResult<()> {
    let out = File::create(dest).map_err(|e| AppError::Storage(e.to_string()))?;
    let mut zip = zip::ZipWriter::new(out);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("info.txt", opts).map_err(zip_err)?;
    zip.write_all(info.as_bytes())
        .map_err(|e| AppError::Storage(e.to_string()))?;

    let mut files: Vec<_> = fs::read_dir(dir)
        .map_err(|e| AppError::Storage(e.to_string()))?
        .flatten()
        .filter(|e| e.metadata().is_ok_and(|m| m.is_file()))
        .collect();
    files.sort_by_key(|e| e.file_name());
    for e in files {
        if e.path() == dest {
            continue; // 出力先がログフォルダ内でも、書き出し中の ZIP 自身は含めない
        }
        let name = e.file_name().to_string_lossy().into_owned();
        // 読めないファイル(他のプロセスが排他で開いているなど)は飛ばして続ける
        let Ok(mut f) = File::open(e.path()) else {
            tracing::warn!(file = %name, "log file skipped in export");
            continue;
        };
        zip.start_file(format!("logs/{name}"), opts)
            .map_err(zip_err)?;
        io::copy(&mut f, &mut zip).map_err(|e| AppError::Storage(e.to_string()))?;
    }
    zip.finish().map_err(zip_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::super::rotate::testdir::TestDir;
    use super::*;

    #[test]
    fn zip_contains_logs_and_info() {
        let d = TestDir::new();
        fs::write(d.path().join("gazooo-2026-11-21.log"), "app log").unwrap();
        fs::write(d.path().join("comfyui.log"), "comfy log").unwrap();
        let dest = d.path().join("out.zip");
        // 出力先が同じフォルダでも壊れない(後から作られる ZIP 自身は読まれない)
        write_zip(d.path(), &dest, "version: 1\n").unwrap();

        let mut zip = zip::ZipArchive::new(File::open(&dest).unwrap()).unwrap();
        let mut names: Vec<_> = zip.file_names().map(|n| n.unwrap().into_owned()).collect();
        names.sort();
        assert!(names.contains(&"info.txt".to_string()));
        assert!(names.contains(&"logs/gazooo-2026-11-21.log".to_string()));
        assert!(names.contains(&"logs/comfyui.log".to_string()));
        let mut s = String::new();
        zip.by_name("info.txt")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        assert_eq!(s, "version: 1\n");
        let mut s = String::new();
        zip.by_name("logs/gazooo-2026-11-21.log")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        assert_eq!(s, "app log");
    }
}
