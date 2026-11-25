//! 調査用の詳細ログ(docs/31)。デバッグの ON/OFF でログレベルを即時に切り替え、
//! ログファイルの分割・合計サイズの上限・手動取得(ZIP)を扱う。

mod export;
pub mod rotate;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, reload};

use crate::error::AppResult;
use crate::util::{date_string, now_ms};
use rotate::{CurrentFile, DateFn, SizeRotatingWriter};

/// ログの合計サイズの上限(1GB)。超えたら古いアプリのログから削除する。
pub const LIMIT_BYTES: u64 = 1_073_741_824;
/// 1 ファイルの最大サイズ(100MB)。超えたら同じ日付のまま次の番号のファイルへ分ける。
pub const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;
/// デバッグが ON の間に、合計サイズを確認する間隔。
const PRUNE_INTERVAL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugStatus {
    pub enabled: bool,
    pub log_dir: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub limit_bytes: u64,
}

/// ログレベルの設定。デバッグ ON のときだけ gazooo 自身を DEBUG にする(依存クレートは INFO のまま)。
fn level_filter(debug: bool) -> Targets {
    Targets::new().with_default(LevelFilter::INFO).with_target(
        "gazooo_lib",
        if debug {
            LevelFilter::DEBUG
        } else {
            LevelFilter::INFO
        },
    )
}

pub struct DebugLog {
    dir: PathBuf,
    enabled: AtomicBool,
    apply: Box<dyn Fn(bool) + Send + Sync>,
    current: CurrentFile,
}

impl DebugLog {
    /// ログの出力先(`dir` の `gazooo-YYYY-MM-DD.log`)と、グローバルな `tracing` の購読者を設定する。
    /// 起動直後は OFF(INFO のみ)。設定の復元は `set_enabled` で行う。
    pub fn init(dir: &Path) -> io::Result<(Arc<Self>, WorkerGuard)> {
        let current = CurrentFile::default();
        let on_open: Box<dyn Fn() + Send> = {
            let (dir, current) = (dir.to_path_buf(), Arc::clone(&current));
            Box::new(move || prune_and_log(&dir, &current))
        };
        let date: DateFn = Arc::new(|| date_string(now_ms()));
        let writer = SizeRotatingWriter::new(
            dir.to_path_buf(),
            MAX_FILE_BYTES,
            date,
            Arc::clone(&current),
            on_open,
        )?;
        let (non_blocking, guard) = tracing_appender::non_blocking(writer);

        let (filter, handle) = reload::Layer::new(level_filter(false));
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(non_blocking)
            .with_ansi(false)
            .with_filter(filter);
        tracing_subscriber::registry()
            .with(layer)
            .try_init()
            .map_err(io::Error::other)?;

        let apply = move |on: bool| {
            let _ = handle.modify(|f| *f = level_filter(on));
        };
        Ok((Self::new(dir, Box::new(apply), current), guard))
    }

    fn new(dir: &Path, apply: Box<dyn Fn(bool) + Send + Sync>, current: CurrentFile) -> Arc<Self> {
        Arc::new(Self {
            dir: dir.to_path_buf(),
            enabled: AtomicBool::new(false),
            apply,
            current,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// ログレベルを即時に切り替える(再起動不要)。
    pub fn set_enabled(&self, on: bool) {
        let was = self.enabled.swap(on, Ordering::Relaxed);
        (self.apply)(on);
        if was != on {
            tracing::info!(enabled = on, "debug logging switched");
        }
    }

    /// ON にした時点・起動時の環境の記録(DEBUG)。
    pub fn log_environment(&self, info: &str) {
        tracing::debug!(env = %info.replace('\n', " | "), "debug logging environment");
    }

    fn current_path(&self) -> Option<PathBuf> {
        self.current.lock().ok().and_then(|g| g.clone())
    }

    /// 件数・サイズはフォルダを走査するため、ブロッキング扱い。
    pub fn status(&self) -> DebugStatus {
        let (file_count, total_bytes) = rotate::usage(&self.dir);
        DebugStatus {
            enabled: self.is_enabled(),
            log_dir: self.dir.to_string_lossy().into_owned(),
            file_count,
            total_bytes,
            limit_bytes: LIMIT_BYTES,
        }
    }

    /// アプリのログをすべて削除する(書き込み中のファイルは空にする)。
    pub fn clear(&self) -> usize {
        let n = rotate::clear(&self.dir, self.current_path().as_deref());
        tracing::info!(count = n, "logs cleared by user");
        n
    }

    /// ログ一式を ZIP にして `dest` へ書く。
    pub fn export(&self, dest: &Path, info: &str) -> AppResult<()> {
        export::write_zip(&self.dir, dest, info)?;
        tracing::info!("logs exported");
        Ok(())
    }

    /// デバッグが ON の間、定期的に合計サイズを確認して上限に収める。
    pub async fn run_pruner(self: Arc<Self>) {
        loop {
            tokio::time::sleep(PRUNE_INTERVAL).await;
            if self.is_enabled() {
                let (dir, current) = (self.dir.clone(), Arc::clone(&self.current));
                let _ = tokio::task::spawn_blocking(move || prune_and_log(&dir, &current)).await;
            }
        }
    }
}

fn prune_and_log(dir: &Path, current: &CurrentFile) {
    let cur = current.lock().ok().and_then(|g| g.clone());
    for p in rotate::prune(dir, LIMIT_BYTES, cur.as_deref()) {
        tracing::info!(file = %p.name, bytes = p.bytes, "old log deleted (size limit)");
    }
}

/// 環境の説明(ZIP の `info.txt`・ON にしたときの DEBUG ログ)。API キーなどの秘密は含めない。
pub fn describe_environment(
    version: &str,
    comfyui_url: &str,
    managed: bool,
    debug_enabled: bool,
) -> String {
    format!(
        "app: gazooo {version}\nos: {} {}\ncomfyui_url: {comfyui_url}\nengine_mode: {}\ndebug_enabled: {debug_enabled}\ngenerated_at_utc: {}\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if managed { "managed" } else { "external" },
        crate::util::compact_timestamp(now_ms()),
    )
}

/// 手動取得の保存名(`gazooo-logs-YYYYMMDD-HHMMSS.zip`。UTC)。
pub fn export_file_name() -> String {
    format!(
        "gazooo-logs-{}.zip",
        crate::util::compact_timestamp(now_ms())
    )
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Mutex;

    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);

    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buf {
        type Writer = Buf;
        fn make_writer(&'a self) -> Buf {
            self.clone()
        }
    }

    #[test]
    fn debug_lines_appear_only_while_enabled() {
        let buf = Buf::default();
        let (filter, handle) = reload::Layer::new(level_filter(false));
        let sub = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(buf.clone())
                .with_ansi(false)
                .with_filter(filter),
        );
        tracing::subscriber::with_default(sub, || {
            tracing::debug!("hidden-1");
            tracing::info!("shown-1");
            handle.modify(|f| *f = level_filter(true)).unwrap();
            tracing::debug!("shown-2");
            handle.modify(|f| *f = level_filter(false)).unwrap();
            tracing::debug!("hidden-2");
        });
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains("shown-1") && out.contains("shown-2"), "{out}");
        assert!(
            !out.contains("hidden-1") && !out.contains("hidden-2"),
            "{out}"
        );
    }

    #[test]
    fn dependency_crates_stay_at_info() {
        let buf = Buf::default();
        let sub = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(buf.clone())
                .with_ansi(false)
                .with_filter(level_filter(true)),
        );
        tracing::subscriber::with_default(sub, || {
            tracing::debug!(target: "hyper::proto", "noisy-dependency");
            tracing::debug!("own-debug");
        });
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(
            out.contains("own-debug") && !out.contains("noisy-dependency"),
            "{out}"
        );
    }

    /// 実際の購読者(自前の書き込み部品 + 非同期書き込み + レベルの切り替え)を通して、ファイルに出ること。
    /// グローバルな購読者を設定するため、このテストだけが `init` を呼ぶ。
    #[test]
    fn init_writes_to_a_file_and_switches_level_at_runtime() {
        let d = rotate::testdir::TestDir::new();
        let (log, guard) = DebugLog::init(d.path()).unwrap();
        tracing::debug!("debug-before-enable");
        tracing::info!("info-line");
        log.set_enabled(true);
        tracing::debug!("debug-while-enabled");
        log.set_enabled(false);
        tracing::debug!("debug-after-disable");
        drop(guard); // フラッシュ

        let cur = log.current_path().unwrap();
        let text = std::fs::read_to_string(cur).unwrap();
        assert!(
            text.contains("info-line") && text.contains("debug-while-enabled"),
            "{text}"
        );
        assert!(
            !text.contains("debug-before-enable") && !text.contains("debug-after-disable"),
            "{text}"
        );
    }

    #[test]
    fn status_counts_files_and_reports_the_limit() {
        let d = rotate::testdir::TestDir::new();
        std::fs::write(d.path().join("gazooo-2026-11-21.log"), "abc").unwrap();
        std::fs::write(d.path().join("comfyui.log"), "de").unwrap();
        let log = DebugLog::new(d.path(), Box::new(|_| {}), Default::default());
        let s = log.status();
        assert_eq!(
            (s.file_count, s.total_bytes, s.limit_bytes),
            (2, 5, LIMIT_BYTES)
        );
        assert!(!s.enabled);
        log.set_enabled(true);
        assert!(log.status().enabled);
    }

    #[test]
    fn environment_description_has_no_secrets_shape() {
        let s = describe_environment("0.1.0", "http://127.0.0.1:8188", true, true);
        assert!(s.contains("engine_mode: managed") && s.contains("debug_enabled: true"));
        assert!(!s.to_lowercase().contains("api_key"));
    }
}
