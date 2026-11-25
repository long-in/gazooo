//! ログファイルの分割書き込みと、合計サイズの上限を守るための自動削除(docs/31 §4)。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// 書き込み中のファイル(自動削除・手動削除の対象から外すために共有する)。
pub type CurrentFile = Arc<Mutex<Option<PathBuf>>>;
/// 今日の日付(`YYYY-MM-DD`)を返す。テストで差し替えられるようにしている。
pub type DateFn = Arc<dyn Fn() -> String + Send + Sync>;

/// アプリのログ(`gazooo-*.log`。旧方式の `gazooo.log*` を含む)か。`comfyui.log` などは含めない。
pub fn is_managed(name: &str) -> bool {
    (name.starts_with("gazooo-") && name.ends_with(".log")) || name.starts_with("gazooo.log")
}

fn file_name(date: &str, index: u32) -> String {
    if index == 0 {
        format!("gazooo-{date}.log")
    } else {
        format!("gazooo-{date}.{index}.log")
    }
}

/// 日次 + 容量で分割するログの書き込み先。`tracing_appender::non_blocking` に渡す。
pub struct SizeRotatingWriter {
    dir: PathBuf,
    max_bytes: u64,
    date_fn: DateFn,
    current: CurrentFile,
    /// 新しいファイルを開くたびに呼ぶ(合計サイズの整理)。
    on_open: Box<dyn Fn() + Send>,
    date: String,
    index: u32,
    file: Option<File>,
    written: u64,
}

impl SizeRotatingWriter {
    pub fn new(
        dir: PathBuf,
        max_bytes: u64,
        date_fn: DateFn,
        current: CurrentFile,
        on_open: Box<dyn Fn() + Send>,
    ) -> io::Result<Self> {
        let mut w = Self {
            dir,
            max_bytes,
            date: date_fn(),
            date_fn,
            current,
            on_open,
            index: 0,
            file: None,
            written: 0,
        };
        w.open_latest()?;
        Ok(w)
    }

    fn path(&self) -> PathBuf {
        self.dir.join(file_name(&self.date, self.index))
    }

    /// 今日のファイルのうち最新のものを開く(満杯なら次の番号へ)。
    fn open_latest(&mut self) -> io::Result<()> {
        self.index = 0;
        while self
            .dir
            .join(file_name(&self.date, self.index + 1))
            .exists()
        {
            self.index += 1;
        }
        if fs::metadata(self.path()).is_ok_and(|m| m.len() >= self.max_bytes) {
            self.index += 1;
        }
        self.open_current()
    }

    fn open_current(&mut self) -> io::Result<()> {
        let path = self.path();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        self.written = file.metadata().map(|m| m.len()).unwrap_or(0);
        self.file = Some(file);
        if let Ok(mut c) = self.current.lock() {
            *c = Some(path);
        }
        (self.on_open)();
        Ok(())
    }

    /// 日付が変わった・容量を超える場合に、書き込み先を切り替える。
    fn roll_if_needed(&mut self, incoming: usize) -> io::Result<()> {
        let today = (self.date_fn)();
        if today != self.date {
            self.date = today;
            self.open_latest()
        } else if self.file.is_none() {
            self.open_current()
        } else if self.written > 0 && self.written + incoming as u64 > self.max_bytes {
            self.index += 1;
            self.open_current()
        } else {
            Ok(())
        }
    }
}

impl Write for SizeRotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.roll_if_needed(buf.len())?;
        if let Some(f) = self.file.as_mut() {
            f.write_all(buf)?;
            self.written += buf.len() as u64;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

/// 削除したファイル。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pruned {
    pub name: String,
    pub bytes: u64,
}

/// ログフォルダの直下のファイル数と合計サイズ(`comfyui.log` などを含む)。
pub fn usage(dir: &Path) -> (usize, u64) {
    let mut count = 0;
    let mut total = 0;
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            if let Ok(m) = e.metadata()
                && m.is_file()
            {
                count += 1;
                total += m.len();
            }
        }
    }
    (count, total)
}

fn name_of(p: Option<&Path>) -> Option<String> {
    p.and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
}

/// 合計が `limit` を超えていたら、アプリのログを更新日時の古い順に削除して収める。
/// 書き込み中のファイル(`current`)と、アプリのログ以外(`comfyui.log` など)は削除しない。
pub fn prune(dir: &Path, limit: u64, current: Option<&Path>) -> Vec<Pruned> {
    let current_name = name_of(current);
    let mut total = 0u64;
    let mut candidates: Vec<(SystemTime, String, u64)> = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() {
            continue;
        }
        total += m.len();
        let name = e.file_name().to_string_lossy().into_owned();
        if is_managed(&name) && Some(&name) != current_name.as_ref() {
            candidates.push((
                m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                name,
                m.len(),
            ));
        }
    }
    candidates.sort();
    let mut removed = Vec::new();
    for (_, name, bytes) in candidates {
        if total <= limit {
            break;
        }
        match fs::remove_file(dir.join(&name)) {
            Ok(()) => {
                total -= bytes;
                removed.push(Pruned { name, bytes });
            }
            Err(e) => tracing::warn!(file = %name, error = %e, "failed to delete old log"),
        }
    }
    removed
}

/// アプリのログをすべて削除する(手動削除)。書き込み中のファイルは空にする。処理したファイル数を返す。
pub fn clear(dir: &Path, current: Option<&Path>) -> usize {
    let current_name = name_of(current);
    let mut n = 0;
    let Ok(rd) = fs::read_dir(dir) else { return 0 };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !is_managed(&name) || !e.metadata().is_ok_and(|m| m.is_file()) {
            continue;
        }
        if Some(&name) == current_name.as_ref() {
            if OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(e.path())
                .is_ok()
            {
                n += 1;
            }
        } else if fs::remove_file(e.path()).is_ok() {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
pub(crate) mod testdir {
    use std::path::{Path, PathBuf};

    pub struct TestDir(pub PathBuf);

    impl TestDir {
        pub fn new() -> Self {
            let p = std::env::temp_dir().join(format!("gazooo-log-test-{}", uuid::Uuid::now_v7()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::testdir::TestDir;
    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn fixed_date(s: &str) -> (DateFn, Arc<Mutex<String>>) {
        let cell = Arc::new(Mutex::new(s.to_string()));
        let c = Arc::clone(&cell);
        (Arc::new(move || c.lock().unwrap().clone()), cell)
    }

    #[test]
    fn splits_by_size_and_continues_the_same_day() {
        let d = TestDir::new();
        let (date, _) = fixed_date("2026-11-21");
        let cur = CurrentFile::default();
        let mut w =
            SizeRotatingWriter::new(d.path().into(), 10, date, cur.clone(), Box::new(|| {}))
                .unwrap();
        w.write_all(b"123456").unwrap();
        w.write_all(b"789012").unwrap(); // 合計 12 > 10 なので次のファイルへ
        w.write_all(b"ab").unwrap();
        drop(w);
        assert_eq!(
            names(d.path()),
            ["gazooo-2026-11-21.1.log", "gazooo-2026-11-21.log"]
        );
        assert_eq!(
            fs::read(d.path().join("gazooo-2026-11-21.log")).unwrap(),
            b"123456"
        );
        assert_eq!(
            fs::read(d.path().join("gazooo-2026-11-21.1.log")).unwrap(),
            b"789012ab"
        );
        let cur = cur.lock().unwrap().clone().unwrap();
        assert!(cur.ends_with("gazooo-2026-11-21.1.log"));

        // 再起動後は、満杯でない最新のファイルに追記する
        let (date, _) = fixed_date("2026-11-21");
        let mut w = SizeRotatingWriter::new(
            d.path().into(),
            100,
            date,
            Default::default(),
            Box::new(|| {}),
        )
        .unwrap();
        w.write_all(b"!").unwrap();
        drop(w);
        assert_eq!(
            fs::read(d.path().join("gazooo-2026-11-21.1.log")).unwrap(),
            b"789012ab!"
        );
    }

    #[test]
    fn new_file_when_the_date_changes() {
        let d = TestDir::new();
        let (date, cell) = fixed_date("2026-11-21");
        let opened = Arc::new(Mutex::new(0));
        let o = Arc::clone(&opened);
        let mut w = SizeRotatingWriter::new(
            d.path().into(),
            1000,
            date,
            Default::default(),
            Box::new(move || *o.lock().unwrap() += 1),
        )
        .unwrap();
        w.write_all(b"a").unwrap();
        *cell.lock().unwrap() = "2026-11-22".into();
        w.write_all(b"b").unwrap();
        drop(w);
        assert_eq!(
            names(d.path()),
            ["gazooo-2026-11-21.log", "gazooo-2026-11-22.log"]
        );
        assert_eq!(
            *opened.lock().unwrap(),
            2,
            "ファイルを開くたびに整理が呼ばれる"
        );
    }

    fn put(dir: &Path, name: &str, bytes: usize, age_secs: u64) {
        let p = dir.join(name);
        fs::write(&p, vec![b'x'; bytes]).unwrap();
        let t = SystemTime::now() - Duration::from_secs(age_secs);
        File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn prune_deletes_oldest_managed_logs_until_under_the_limit() {
        let d = TestDir::new();
        put(d.path(), "gazooo-2026-11-18.log", 40, 400);
        put(d.path(), "gazooo-2026-11-19.log", 40, 300);
        put(d.path(), "gazooo-2026-11-20.log", 40, 200);
        put(d.path(), "gazooo-2026-11-21.log", 40, 0); // 書き込み中
        put(d.path(), "comfyui.log", 100, 1000); // 古くても削除しない
        let cur = d.path().join("gazooo-2026-11-21.log");

        // 合計 260、上限 150 → 古い順に 3 つ削除して 140
        let removed = prune(d.path(), 150, Some(&cur));
        assert_eq!(
            removed.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            [
                "gazooo-2026-11-18.log",
                "gazooo-2026-11-19.log",
                "gazooo-2026-11-20.log"
            ]
        );
        assert_eq!(names(d.path()), ["comfyui.log", "gazooo-2026-11-21.log"]);
        assert_eq!(usage(d.path()), (2, 140));

        // 上限内なら何も消さない
        assert!(prune(d.path(), 1000, Some(&cur)).is_empty());
        // 書き込み中のファイルと comfyui.log だけで超えていても、それらは消さない
        assert!(prune(d.path(), 10, Some(&cur)).is_empty());
        assert_eq!(names(d.path()), ["comfyui.log", "gazooo-2026-11-21.log"]);
    }

    #[test]
    fn clear_removes_app_logs_and_empties_the_current_one() {
        let d = TestDir::new();
        put(d.path(), "gazooo-2026-11-20.log", 10, 100);
        put(d.path(), "gazooo.log.2026-11-19", 10, 200); // 旧方式
        put(d.path(), "gazooo-2026-11-21.log", 10, 0);
        put(d.path(), "comfyui.log", 10, 0);
        let cur = d.path().join("gazooo-2026-11-21.log");
        assert_eq!(clear(d.path(), Some(&cur)), 3);
        assert_eq!(names(d.path()), ["comfyui.log", "gazooo-2026-11-21.log"]);
        assert_eq!(fs::metadata(&cur).unwrap().len(), 0);
    }
}
