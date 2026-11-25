//! 画像の形式変換(docs/43)。複数の画像を PNG / JPEG / WebP へ一括変換する。
//!
//! - 入力の形式は拡張子ではなく中身(先頭バイト)で判定する(§3.1)。
//! - 取り込んだファイルは Rust 側の一時リスト(`id` → パス)で持ち、フロントには `id` だけ渡す(§3.2)。
//! - 元ファイルは変更・削除しない。出力先へ新しいファイルを書くだけ(§5)。
//! - メタデータ(EXIF・テキストチャンク・ICC)は引き継がない(§4.4)。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Limits};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::services::heic;

/// 一覧に載せられる枚数の上限(§3.3)。
pub const MAX_ENTRIES: usize = 1000;
/// 1 枚あたりの画素数の上限(1 億画素。§3.3)。
pub const MAX_PIXELS: u64 = 100_000_000;
/// フォルダを辿る深さの上限(循環・極端な階層を避ける)。
const MAX_DEPTH: usize = 32;
/// 一時ファイルの拡張子(出力先と同じフォルダに作る。§6)。
const TMP_EXT: &str = "gazooo-tmp";

// ---------------------------------------------------------------- 型

/// 入力として検出した形式(§3.1)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InFormat {
    Webp,
    Png,
    Jpeg,
    Gif,
    Bmp,
    Tiff,
    /// HEIC / HEIF(docs/43 §13)。
    Heic,
}

impl InFormat {
    fn from_image(f: ImageFormat) -> Option<Self> {
        Some(match f {
            ImageFormat::WebP => Self::Webp,
            ImageFormat::Png => Self::Png,
            ImageFormat::Jpeg => Self::Jpeg,
            ImageFormat::Gif => Self::Gif,
            ImageFormat::Bmp => Self::Bmp,
            ImageFormat::Tiff => Self::Tiff,
            _ => return None,
        })
    }
}

/// 一覧の 1 行。`error` があるものは変換の対象にならない。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertEntry {
    pub id: String,
    pub name: String,
    pub detected_format: Option<InFormat>,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutFormat {
    Png,
    Jpeg,
    Webp,
}

impl OutFormat {
    fn ext(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PngCompression {
    Fast,
    Default,
    Best,
}

/// 変換の指定(§4.1)。出力先は含まない(保存済みの設定を使う)。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertOptions {
    pub format: OutFormat,
    pub quality: u8,
    pub webp_lossless: bool,
    pub png_compression: PngCompression,
    /// `#rrggbb`。アルファを JPEG にするときの背景。
    pub background: String,
    pub overwrite: bool,
}

/// 保存する設定(§8)。`output_dir` が `None` のときは既定の場所を使う。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConvertSettings {
    pub format: OutFormat,
    pub quality: u8,
    pub webp_lossless: bool,
    pub png_compression: PngCompression,
    pub background: String,
    pub overwrite: bool,
    pub output_dir: Option<String>,
}

impl Default for ConvertSettings {
    fn default() -> Self {
        Self {
            format: OutFormat::Webp,
            quality: 85,
            webp_lossless: false,
            png_compression: PngCompression::Default,
            background: "#ffffff".into(),
            overwrite: false,
            output_dir: None,
        }
    }
}

/// フロントへ返す設定。`output_dir` は実際に使う場所(未設定なら既定の場所)。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertSettingsView {
    #[serde(flatten)]
    pub settings: ConvertSettings,
    pub output_dir: String,
}

/// 設定の部分更新(出力先は専用コマンドで変える)。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConvertSettingsPatch {
    pub format: Option<OutFormat>,
    pub quality: Option<u8>,
    pub webp_lossless: Option<bool>,
    pub png_compression: Option<PngCompression>,
    pub background: Option<String>,
    pub overwrite: Option<bool>,
}

impl ConvertSettings {
    pub fn apply(&mut self, p: ConvertSettingsPatch) -> AppResult<()> {
        if let Some(q) = p.quality {
            validate_quality(q)?;
            self.quality = q;
        }
        if let Some(b) = p.background {
            parse_color(&b)?;
            self.background = b;
        }
        if let Some(v) = p.format {
            self.format = v;
        }
        if let Some(v) = p.webp_lossless {
            self.webp_lossless = v;
        }
        if let Some(v) = p.png_compression {
            self.png_compression = v;
        }
        if let Some(v) = p.overwrite {
            self.overwrite = v;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemResult {
    pub id: String,
    pub ok: bool,
    pub output_name: Option<String>,
    pub in_bytes: u64,
    pub out_bytes: Option<u64>,
    pub error: Option<String>,
}

/// 実行中の集計。
#[derive(Debug, Clone, Copy, Default)]
struct Tally {
    done: usize,
    ok: usize,
    failed: usize,
    in_bytes: u64,
    out_bytes: u64,
}

/// 1 枚の結果を集計し、`item` と `progress` を通知する(順序を保つため、ロックを持ったまま送る)。
fn record(
    events: &dyn ConvertEvents,
    tally: &Mutex<Tally>,
    run_id: &str,
    total: usize,
    r: &ItemResult,
) {
    let Ok(mut t) = tally.lock() else { return };
    t.done += 1;
    if r.ok {
        t.ok += 1;
        t.in_bytes += r.in_bytes;
        t.out_bytes += r.out_bytes.unwrap_or(0);
    } else {
        t.failed += 1;
    }
    events.item(r);
    events.progress(&Progress {
        run_id: run_id.to_owned(),
        done: t.done,
        total,
    });
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub run_id: String,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finished {
    pub run_id: String,
    pub ok: usize,
    pub failed: usize,
    pub in_bytes: u64,
    pub out_bytes: u64,
    pub cancelled: bool,
    pub output_dir: String,
}

/// フロントへの通知先(テストでは記録用の実装に差し替える)。
pub trait ConvertEvents: Send + Sync {
    fn added(&self, _entries: &[ConvertEntry]) {}
    fn progress(&self, _p: &Progress) {}
    fn item(&self, _r: &ItemResult) {}
    fn finished(&self, _f: &Finished) {}
}

// ---------------------------------------------------------------- 検証・補助

pub fn validate_quality(q: u8) -> AppResult<()> {
    if (1..=100).contains(&q) {
        Ok(())
    } else {
        Err(AppError::validation(
            "quality",
            "品質は 1〜100 で指定してください",
        ))
    }
}

/// `#rrggbb` を RGB にする。
pub fn parse_color(s: &str) -> AppResult<[u8; 3]> {
    let bad = || AppError::validation("background", "背景色は #rrggbb で指定してください");
    let hex = s.strip_prefix('#').ok_or_else(bad)?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let n = u32::from_str_radix(hex, 16).map_err(|_| bad())?;
    Ok([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// Windows で使えない文字を `_` にし、末尾の空白・ドットを除き、長すぎる名前を切り詰める。
pub fn sanitize_stem(stem: &str) -> String {
    let mut s: String = stem
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    s = s.trim_end_matches([' ', '.']).to_string();
    // 予約名(CON など)は先頭に `_` を付ける
    let upper = s.to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|r| upper == *r || upper.starts_with(&format!("{r}.")))
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        s.insert(0, '_');
    }
    if s.chars().count() > 120 {
        s = s.chars().take(120).collect();
    }
    if s.is_empty() { "image".into() } else { s }
}

/// 出力先の既定の場所(`<ピクチャ>/gazooo-converted`。ピクチャが無ければホーム直下)。
pub fn default_output_dir(picture_dir: Option<PathBuf>) -> PathBuf {
    picture_dir
        .unwrap_or_else(std::env::temp_dir)
        .join("gazooo-converted")
}

// ---------------------------------------------------------------- 取り込み(検査)

struct Probe {
    format: InFormat,
    width: u32,
    height: u32,
    bytes: u64,
}

/// ファイルの先頭だけ読んで、形式・寸法・サイズを調べる。
fn probe(path: &Path) -> Result<Probe, ProbeError> {
    let meta = std::fs::metadata(path).map_err(|e| ProbeError::Io(e.to_string()))?;
    let mut file = std::fs::File::open(path).map_err(|e| ProbeError::Io(e.to_string()))?;
    // HEIC は `image` が判定できないため、先頭の `ftyp` を自前で見る(docs/43 §13)
    let mut head = [0u8; 64];
    let n = std::io::Read::read(&mut file, &mut head).map_err(|e| ProbeError::Io(e.to_string()))?;
    if heic::is_heic(&head[..n]) {
        let data = std::fs::read(path).map_err(|e| ProbeError::Io(e.to_string()))?;
        let (width, height) = heic::dimensions(&data).map_err(|_| ProbeError::Broken)?;
        return Ok(Probe {
            format: InFormat::Heic,
            width,
            height,
            bytes: meta.len(),
        });
    }
    std::io::Seek::rewind(&mut file).map_err(|e| ProbeError::Io(e.to_string()))?;
    let reader = ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .map_err(|e| ProbeError::Io(e.to_string()))?;
    let Some(fmt) = reader.format() else {
        return Err(ProbeError::NotImage);
    };
    let Some(format) = InFormat::from_image(fmt) else {
        return Err(ProbeError::Unsupported);
    };
    let (width, height) = reader.into_dimensions().map_err(|_| ProbeError::Broken)?;
    Ok(Probe {
        format,
        width,
        height,
        bytes: meta.len(),
    })
}

enum ProbeError {
    /// 画像の形式として判定できない。
    NotImage,
    /// 画像だが対応していない形式(HEIC・AVIF など)。
    Unsupported,
    /// 形式は判定できたが、読み込めない。
    Broken,
    Io(String),
}

impl ProbeError {
    fn message(&self) -> String {
        match self {
            Self::NotImage => "画像として読めません".into(),
            Self::Unsupported => "対応していない形式です".into(),
            Self::Broken => "画像が壊れているか、読み込めません".into(),
            Self::Io(e) => format!("ファイルを読めません: {e}"),
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// フォルダを再帰的に辿り、ファイルを集める(隠しフォルダ・シンボリックリンクは辿らない。§3.3)。
fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>, limit: usize) {
    if depth > MAX_DEPTH || out.len() >= limit {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<_> = rd.filter_map(Result::ok).collect();
    items.sort_by_key(|e| e.file_name());
    for e in items {
        if out.len() >= limit {
            return;
        }
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            walk(&e.path(), depth + 1, out, limit);
        } else if ft.is_file() {
            out.push(e.path());
        }
    }
}

// ---------------------------------------------------------------- 変換(純粋関数)

fn has_alpha(img: &DynamicImage) -> bool {
    img.color().has_alpha()
}

/// 画像を読み込む。EXIF の向きをピクセルへ反映する(§4.3)。
fn decode(path: &Path) -> Result<DynamicImage, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("ファイルを読めません: {e}"))?;
    let mut head = [0u8; 64];
    let n = std::io::Read::read(&mut file, &mut head)
        .map_err(|e| format!("ファイルを読めません: {e}"))?;
    if heic::is_heic(&head[..n]) {
        // 向き・クロップはデコーダが適用済み(§13.1)
        let data = std::fs::read(path).map_err(|e| format!("ファイルを読めません: {e}"))?;
        return heic::decode(&data, MAX_PIXELS).map_err(|e| {
            if e.contains("大きすぎ") {
                "画像が大きすぎます(1 億画素まで)".to_string()
            } else {
                ProbeError::Broken.message()
            }
        });
    }
    std::io::Seek::rewind(&mut file).map_err(|e| format!("ファイルを読めません: {e}"))?;
    let reader = ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .map_err(|e| format!("ファイルを読めません: {e}"))?;
    if reader.format().and_then(InFormat::from_image).is_none() {
        return Err(ProbeError::NotImage.message());
    }
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| ProbeError::Broken.message())?;
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err("画像が大きすぎます(1 億画素まで)".into());
    }
    let mut limits = Limits::default();
    limits.max_alloc = Some(2 * 1024 * 1024 * 1024);
    let _ = decoder.set_limits(limits);
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(|_| ProbeError::Broken.message())?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(img)
}

/// アルファを背景色に合成した RGB 画像を作る(JPEG 用。§4.2)。
fn flatten(img: &DynamicImage, bg: [u8; 3]) -> image::RgbImage {
    let rgba = img.to_rgba8();
    let mut out = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = u32::from(p.0[3]);
        let mix = |c: u8, b: u8| ((u32::from(c) * a + u32::from(b) * (255 - a) + 127) / 255) as u8;
        out.put_pixel(
            x,
            y,
            image::Rgb([mix(p.0[0], bg[0]), mix(p.0[1], bg[1]), mix(p.0[2], bg[2])]),
        );
    }
    out
}

/// 指定の形式・品質でエンコードする。
pub fn encode(img: &DynamicImage, opts: &ConvertOptions) -> AppResult<Vec<u8>> {
    validate_quality(opts.quality)?;
    let bg = parse_color(&opts.background)?;
    let enc_err = |e: String| AppError::Storage(format!("変換に失敗しました: {e}"));
    let mut out = Vec::new();
    match opts.format {
        OutFormat::Png => {
            let level = match opts.png_compression {
                PngCompression::Fast => CompressionType::Fast,
                PngCompression::Default => CompressionType::Default,
                PngCompression::Best => CompressionType::Best,
            };
            let enc = PngEncoder::new_with_quality(&mut out, level, FilterType::Adaptive);
            if has_alpha(img) {
                let b = img.to_rgba8();
                enc.write_image(
                    b.as_raw(),
                    b.width(),
                    b.height(),
                    image::ExtendedColorType::Rgba8,
                )
            } else {
                let b = img.to_rgb8();
                enc.write_image(
                    b.as_raw(),
                    b.width(),
                    b.height(),
                    image::ExtendedColorType::Rgb8,
                )
            }
            .map_err(|e| enc_err(e.to_string()))?;
        }
        OutFormat::Jpeg => {
            let b = flatten(img, bg);
            JpegEncoder::new_with_quality(&mut out, opts.quality)
                .write_image(
                    b.as_raw(),
                    b.width(),
                    b.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(|e| enc_err(e.to_string()))?;
        }
        OutFormat::Webp => {
            let mem = if has_alpha(img) {
                let b = img.to_rgba8();
                let e = webp::Encoder::from_rgba(b.as_raw(), b.width(), b.height());
                if opts.webp_lossless {
                    e.encode_lossless()
                } else {
                    e.encode(f32::from(opts.quality))
                }
                .to_vec()
            } else {
                let b = img.to_rgb8();
                let e = webp::Encoder::from_rgb(b.as_raw(), b.width(), b.height());
                if opts.webp_lossless {
                    e.encode_lossless()
                } else {
                    e.encode(f32::from(opts.quality))
                }
                .to_vec()
            };
            out = mem;
        }
    }
    Ok(out)
}

/// 出力ファイルの名前を決める。同名があれば ` (1)`・` (2)` … を付ける(§5)。
/// `reserved` は今回の実行で決めた名前(同時に走る別の画像との衝突を避ける)。
/// 元ファイル自身は、上書き設定でも決して選ばない。
pub(crate) fn choose_output(
    dir: &Path,
    stem: &str,
    ext: &str,
    overwrite: bool,
    src: &Path,
    reserved: &mut HashSet<PathBuf>,
) -> PathBuf {
    let src_canon = std::fs::canonicalize(src).ok();
    let is_src = |p: &Path| {
        src_canon.is_some() && std::fs::canonicalize(p).ok().as_ref() == src_canon.as_ref()
    };
    let mut n = 0u32;
    loop {
        let name = if n == 0 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let cand = dir.join(&name);
        let taken = reserved.contains(&cand) || is_src(&cand) || (!overwrite && cand.exists());
        if !taken {
            reserved.insert(cand.clone());
            return cand;
        }
        n += 1;
    }
}

/// 1 枚を変換して出力先へ書く。成功したら出力のパスとサイズを返す。
fn convert_one(
    src: &Path,
    out_dir: &Path,
    opts: &ConvertOptions,
    reserved: &Mutex<HashSet<PathBuf>>,
) -> Result<(PathBuf, u64), String> {
    let img = decode(src)?;
    let bytes = encode(&img, opts).map_err(|e| e.to_string())?;
    let stem = sanitize_stem(
        &src.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let target = {
        let mut r = reserved.lock().map_err(|_| "内部エラー".to_string())?;
        choose_output(
            out_dir,
            &stem,
            opts.format.ext(),
            opts.overwrite,
            src,
            &mut r,
        )
    };
    let tmp = target.with_file_name(format!(
        "{}.{TMP_EXT}",
        target.file_name().unwrap_or_default().to_string_lossy()
    ));
    let write = || -> std::io::Result<()> {
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &target)
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("書き込めません: {e}"));
    }
    Ok((target, bytes.len() as u64))
}

// ---------------------------------------------------------------- サービス

struct Held {
    entry: ConvertEntry,
    path: PathBuf,
}

#[derive(Default)]
struct State {
    items: Vec<Held>,
    accepting: bool,
    runs: Vec<(String, Arc<AtomicBool>)>,
}

pub struct ConvertService {
    state: Mutex<State>,
}

impl ConvertService {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
        })
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| AppError::internal("convert state poisoned"))
    }

    /// 変換ページが開いている間だけ、ドロップを受け付ける(§2)。
    pub fn set_accepting(&self, on: bool) {
        if let Ok(mut s) = self.lock() {
            s.accepting = on;
        }
    }

    pub fn accepting(&self) -> bool {
        self.lock().map(|s| s.accepting).unwrap_or(false)
    }

    /// ファイル・フォルダのパス(ダイアログ・ドロップ由来)を一覧へ加える。追加した行を返す。
    /// フォルダは再帰的に展開し、画像と判定できたものだけを加える。
    pub fn add_paths(&self, paths: Vec<PathBuf>) -> AppResult<Vec<ConvertEntry>> {
        let mut direct: Vec<PathBuf> = Vec::new();
        let mut found: Vec<PathBuf> = Vec::new();
        for p in paths {
            match std::fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => walk(&p, 0, &mut found, MAX_ENTRIES + 1),
                Ok(m) if m.is_file() => direct.push(p),
                _ => {}
            }
        }
        let mut added = Vec::new();
        let mut s = self.lock()?;
        let mut push = |s: &mut State, path: PathBuf, from_dir: bool| {
            if s.items.len() >= MAX_ENTRIES || s.items.iter().any(|h| h.path == path) {
                return;
            }
            let entry = match probe(&path) {
                Ok(p) => ConvertEntry {
                    id: uuid::Uuid::now_v7().to_string(),
                    name: file_name(&path),
                    detected_format: Some(p.format),
                    width: p.width,
                    height: p.height,
                    bytes: p.bytes,
                    error: if u64::from(p.width) * u64::from(p.height) > MAX_PIXELS {
                        Some("画像が大きすぎます(1 億画素まで)".into())
                    } else {
                        None
                    },
                },
                // フォルダ内の画像でないファイルは、黙って除く
                Err(ProbeError::NotImage) if from_dir => return,
                Err(e) => ConvertEntry {
                    id: uuid::Uuid::now_v7().to_string(),
                    name: file_name(&path),
                    detected_format: None,
                    width: 0,
                    height: 0,
                    bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                    error: Some(e.message()),
                },
            };
            added.push(entry.clone());
            s.items.push(Held { entry, path });
        };
        for p in direct {
            push(&mut s, p, false);
        }
        for p in found {
            push(&mut s, p, true);
        }
        Ok(added)
    }

    pub fn list(&self) -> AppResult<Vec<ConvertEntry>> {
        Ok(self.lock()?.items.iter().map(|h| h.entry.clone()).collect())
    }

    pub fn remove(&self, ids: &[String]) -> AppResult<()> {
        self.lock()?.items.retain(|h| !ids.contains(&h.entry.id));
        Ok(())
    }

    pub fn clear(&self) -> AppResult<()> {
        self.lock()?.items.clear();
        Ok(())
    }

    pub fn cancel(&self, run_id: &str) {
        if let Ok(s) = self.lock()
            && let Some((_, f)) = s.runs.iter().find(|(id, _)| id == run_id)
        {
            f.store(true, Ordering::SeqCst);
        }
    }

    /// 変換を始める。実行は別タスクで進み、進捗と結果は `events` で通知する。`runId` を返す。
    pub fn run(
        self: &Arc<Self>,
        ids: Vec<String>,
        opts: ConvertOptions,
        out_dir: PathBuf,
        events: Arc<dyn ConvertEvents>,
    ) -> AppResult<String> {
        validate_quality(opts.quality)?;
        parse_color(&opts.background)?;
        let targets: Vec<(String, PathBuf, u64)> = {
            let s = self.lock()?;
            let mut v = Vec::new();
            for id in &ids {
                let h =
                    s.items.iter().find(|h| &h.entry.id == id).ok_or_else(|| {
                        AppError::validation("ids", "変換する画像が見つかりません")
                    })?;
                if h.entry.error.is_none() {
                    v.push((h.entry.id.clone(), h.path.clone(), h.entry.bytes));
                }
            }
            v
        };
        if targets.is_empty() {
            return Err(AppError::validation("ids", "変換できる画像がありません"));
        }
        std::fs::create_dir_all(&out_dir).map_err(|e| {
            AppError::validation(
                "outputDir",
                format!("出力先を作成できません({}): {e}", out_dir.display()),
            )
        })?;
        // 書き込みできるか先に確かめる
        let probe_file = out_dir.join(format!(".gazooo-write-test.{TMP_EXT}"));
        std::fs::write(&probe_file, b"").map_err(|e| {
            AppError::validation("outputDir", format!("出力先に書き込めません: {e}"))
        })?;
        let _ = std::fs::remove_file(&probe_file);

        let run_id = uuid::Uuid::now_v7().to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        self.lock()?
            .runs
            .push((run_id.clone(), Arc::clone(&cancel)));

        let svc = Arc::clone(self);
        let rid = run_id.clone();
        tauri::async_runtime::spawn(async move {
            svc.execute(rid, targets, opts, out_dir, cancel, events)
                .await;
        });
        Ok(run_id)
    }

    async fn execute(
        self: Arc<Self>,
        run_id: String,
        targets: Vec<(String, PathBuf, u64)>,
        opts: ConvertOptions,
        out_dir: PathBuf,
        cancel: Arc<AtomicBool>,
        events: Arc<dyn ConvertEvents>,
    ) {
        let total = targets.len();
        let workers = std::thread::available_parallelism()
            .map(|n| n.get() / 2)
            .unwrap_or(1)
            .clamp(1, 4);
        let sem = Arc::new(tokio::sync::Semaphore::new(workers));
        let reserved = Arc::new(Mutex::new(HashSet::new()));
        let opts = Arc::new(opts);
        let out_dir = Arc::new(out_dir);
        let tally = Arc::new(Mutex::new(Tally::default()));
        let mut set = tokio::task::JoinSet::new();
        for (id, path, in_bytes) in targets {
            let Ok(permit) = Arc::clone(&sem).acquire_owned().await else {
                break;
            };
            // 中止後は未着手の画像を処理しない
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let (opts, out_dir, reserved) = (
                Arc::clone(&opts),
                Arc::clone(&out_dir),
                Arc::clone(&reserved),
            );
            let (events, tally, rid) = (Arc::clone(&events), Arc::clone(&tally), run_id.clone());
            // 完了のたびにここで集計・通知する(回収を起動ループの後ろに置くと、
            // 進捗が最後にまとめて届き 0 → 100 に見えてしまう)
            set.spawn_blocking(move || {
                let _permit = permit;
                let r = match convert_one(&path, &out_dir, &opts, &reserved) {
                    Ok((out, out_bytes)) => ItemResult {
                        id,
                        ok: true,
                        output_name: Some(file_name(&out)),
                        in_bytes,
                        out_bytes: Some(out_bytes),
                        error: None,
                    },
                    Err(e) => ItemResult {
                        id,
                        ok: false,
                        output_name: None,
                        in_bytes,
                        out_bytes: None,
                        error: Some(e),
                    },
                };
                record(&*events, &tally, &rid, total, &r);
            });
        }
        // 変換スレッドが異常終了した分は、失敗として数える
        while let Some(res) = set.join_next().await {
            if let Err(e) = res {
                let r = ItemResult {
                    id: String::new(),
                    ok: false,
                    output_name: None,
                    in_bytes: 0,
                    out_bytes: None,
                    error: Some(format!("内部エラー: {e}")),
                };
                record(&*events, &tally, &run_id, total, &r);
            }
        }
        let t = tally.lock().map(|t| *t).unwrap_or_default();
        let (ok, failed, in_b, out_b) = (t.ok, t.failed, t.in_bytes, t.out_bytes);
        if let Ok(mut s) = self.lock() {
            s.runs.retain(|(id, _)| id != &run_id);
        }
        events.finished(&Finished {
            run_id,
            ok,
            failed,
            in_bytes: in_b,
            out_bytes: out_b,
            cancelled: cancel.load(Ordering::SeqCst),
            output_dir: out_dir.to_string_lossy().into_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("gazooo-convert-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn opts(format: OutFormat) -> ConvertOptions {
        ConvertOptions {
            format,
            quality: 80,
            webp_lossless: false,
            png_compression: PngCompression::Default,
            background: "#ffffff".into(),
            overwrite: false,
        }
    }

    /// グラデーションの RGBA 画像(アルファは左半分だけ透明)。
    fn sample(w: u32, h: u32) -> DynamicImage {
        let mut img = image::RgbaImage::new(w, h);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let a = if x < w / 2 { 0 } else { 255 };
            *p = image::Rgba([(x * 255 / w) as u8, (y * 255 / h) as u8, 128, a]);
        }
        DynamicImage::ImageRgba8(img)
    }

    fn write_as(dir: &Path, name: &str, img: &DynamicImage, o: OutFormat) -> PathBuf {
        let bytes = encode(img, &opts(o)).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn webp_round_trips_lossy_lossless_and_alpha() {
        let img = sample(64, 48);
        let mut o = opts(OutFormat::Webp);
        o.webp_lossless = true;
        let lossless = encode(&img, &o).unwrap();
        let back = image::load_from_memory(&lossless).unwrap();
        assert_eq!((back.width(), back.height()), (64, 48));
        assert!(has_alpha(&back));
        assert_eq!(back.to_rgba8().get_pixel(0, 0).0[3], 0);
        assert_eq!(back.to_rgba8().get_pixel(60, 10).0[3], 255);

        o.webp_lossless = false;
        let lossy = encode(&img, &o).unwrap();
        let back = image::load_from_memory(&lossy).unwrap();
        assert_eq!((back.width(), back.height()), (64, 48));
    }

    #[test]
    fn quality_changes_size_for_jpeg_and_webp() {
        let mut img = image::RgbImage::new(128, 128);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgb([
                (x * 7 % 256) as u8,
                (y * 13 % 256) as u8,
                ((x ^ y) * 5 % 256) as u8,
            ]);
        }
        let img = DynamicImage::ImageRgb8(img);
        for f in [OutFormat::Jpeg, OutFormat::Webp] {
            let mut lo = opts(f);
            lo.quality = 10;
            let mut hi = opts(f);
            hi.quality = 95;
            assert!(
                encode(&img, &lo).unwrap().len() < encode(&img, &hi).unwrap().len(),
                "{f:?}"
            );
        }
    }

    #[test]
    fn jpeg_flattens_alpha_onto_background() {
        let img = sample(8, 8);
        let mut o = opts(OutFormat::Jpeg);
        o.quality = 100;
        o.background = "#ff0000".into();
        let back = image::load_from_memory(&encode(&img, &o).unwrap())
            .unwrap()
            .to_rgb8();
        let p = back.get_pixel(0, 0).0; // 透明 → 背景の赤
        assert!(p[0] > 200 && p[1] < 60 && p[2] < 60, "{p:?}");
        assert!(parse_color("red").is_err() && parse_color("#12345").is_err());
    }

    #[test]
    fn quality_is_validated() {
        let img = sample(4, 4);
        for q in [0u8, 101] {
            let mut o = opts(OutFormat::Jpeg);
            o.quality = q;
            assert!(encode(&img, &o).is_err());
        }
    }

    #[test]
    fn detects_format_by_content_not_extension() {
        let dir = temp_dir();
        // WebP を .jpg、PNG を .webp という名前で保存する
        let a = write_as(&dir, "a.jpg", &sample(10, 6), OutFormat::Webp);
        let b = write_as(&dir, "b.webp", &sample(5, 5), OutFormat::Png);
        let c = dir.join("c.png");
        std::fs::write(&c, b"not an image at all").unwrap();
        assert_eq!(probe(&a).ok().map(|p| p.format), Some(InFormat::Webp));
        assert_eq!(probe(&b).ok().map(|p| p.format), Some(InFormat::Png));
        assert!(matches!(probe(&c), Err(ProbeError::NotImage)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn heic_is_detected_by_content_and_broken_one_fails_alone() {
        let dir = temp_dir();
        // `ftyp` だけある壊れた HEIC(拡張子は .jpg)。形式は HEIC と判定され、読み込みは失敗する
        let mut data = vec![0, 0, 0, 20];
        data.extend(b"ftypheic    mif1");
        data.extend(vec![0xAB; 64]);
        let f = dir.join("broken.jpg");
        std::fs::write(&f, &data).unwrap();
        assert!(matches!(probe(&f), Err(ProbeError::Broken)));
        assert!(decode(&f).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 実ファイルでの確認(`GAZOOO_HEIC_SAMPLE=<path> cargo test heic_real -- --ignored`)。
    #[test]
    #[ignore]
    fn heic_real_file_probes_decodes_and_encodes() {
        let p = PathBuf::from(std::env::var("GAZOOO_HEIC_SAMPLE").expect("GAZOOO_HEIC_SAMPLE"));
        let pr = probe(&p).ok().expect("probe");
        assert_eq!(pr.format, InFormat::Heic);
        let img = decode(&p).expect("decode");
        assert_eq!((img.width(), img.height()), (pr.width, pr.height));
        for f in [OutFormat::Png, OutFormat::Jpeg, OutFormat::Webp] {
            assert!(!encode(&img, &opts(f)).unwrap().is_empty());
        }
    }

    #[test]
    fn add_paths_expands_folders_and_skips_non_images_and_dups() {
        let dir = temp_dir();
        let sub = dir.join("sub");
        std::fs::create_dir_all(sub.join(".hidden")).unwrap();
        write_as(&dir, "a.png", &sample(4, 4), OutFormat::Png);
        write_as(&sub, "b.webp", &sample(4, 4), OutFormat::Webp);
        write_as(&sub.join(".hidden"), "h.png", &sample(4, 4), OutFormat::Png);
        std::fs::write(sub.join("notes.txt"), b"hello").unwrap();
        let direct_bad = dir.join("direct.txt");
        std::fs::write(&direct_bad, b"hello").unwrap();

        let svc = ConvertService::new();
        let added = svc
            .add_paths(vec![dir.clone(), direct_bad.clone()])
            .unwrap();
        let names: Vec<_> = added.iter().map(|e| e.name.as_str()).collect();
        // 直接指定した画像でないファイルはエラー行として出し、フォルダ内の .txt・隠しフォルダは除く
        assert!(
            names.contains(&"a.png") && names.contains(&"b.webp"),
            "{names:?}"
        );
        assert!(
            !names.contains(&"notes.txt") && !names.contains(&"h.png"),
            "{names:?}"
        );
        let bad = added.iter().find(|e| e.name == "direct.txt").unwrap();
        assert!(bad.error.is_some());
        // 同じパスの再登録は無視される
        assert!(svc.add_paths(vec![dir.clone()]).unwrap().is_empty());
        assert_eq!(svc.list().unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn output_names_avoid_collision_and_never_clobber_source() {
        let dir = temp_dir();
        let src = write_as(&dir, "a.png", &sample(4, 4), OutFormat::Png);
        let reserved = Mutex::new(HashSet::new());

        // 出力先 = 元と同じフォルダ・同じ形式。上書き設定でも元ファイルは壊さず連番にする
        let mut o = opts(OutFormat::Png);
        o.overwrite = true;
        let (out, _) = convert_one(&src, &dir, &o, &reserved).unwrap();
        assert_eq!(file_name(&out), "a (1).png");
        assert!(src.exists());

        // 上書きしない設定では、既存の同名の隣に連番で書く
        let (out2, _) = convert_one(
            &src,
            &dir,
            &opts(OutFormat::Png),
            &Mutex::new(HashSet::new()),
        )
        .unwrap();
        assert_eq!(file_name(&out2), "a (2).png");

        // 別フォルダ + 上書き設定: 同名を置き換える(今回の実行内の重複は連番)
        let out_dir = temp_dir();
        std::fs::write(out_dir.join("a.webp"), b"old").unwrap();
        let mut ow = opts(OutFormat::Webp);
        ow.overwrite = true;
        let r = Mutex::new(HashSet::new());
        let (o1, _) = convert_one(&src, &out_dir, &ow, &r).unwrap();
        let (o2, _) = convert_one(&src, &out_dir, &ow, &r).unwrap();
        assert_eq!(file_name(&o1), "a.webp");
        assert_eq!(file_name(&o2), "a (1).webp");
        assert!(std::fs::read(&o1).unwrap().len() > 3);
        // 一時ファイルは残らない
        assert!(
            !std::fs::read_dir(&out_dir)
                .unwrap()
                .any(|e| { e.unwrap().file_name().to_string_lossy().ends_with(TMP_EXT) })
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&out_dir);
    }

    #[test]
    fn sanitize_handles_windows_unsafe_names() {
        assert_eq!(sanitize_stem("a:b*c?"), "a_b_c_");
        assert_eq!(sanitize_stem("con"), "_con");
        assert_eq!(sanitize_stem("name. "), "name");
        assert_eq!(sanitize_stem(""), "image");
        assert_eq!(sanitize_stem(&"あ".repeat(300)).chars().count(), 120);
    }

    #[derive(Default)]
    struct Rec {
        items: Mutex<Vec<ItemResult>>,
        fin: Mutex<Option<Finished>>,
    }
    impl ConvertEvents for Rec {
        fn item(&self, r: &ItemResult) {
            self.items.lock().unwrap().push(r.clone());
        }
        fn finished(&self, f: &Finished) {
            *self.fin.lock().unwrap() = Some(f.clone());
        }
    }

    #[tokio::test]
    async fn run_converts_mixed_inputs_and_isolates_failures() {
        let dir = temp_dir();
        let out = temp_dir();
        write_as(&dir, "1.webp", &sample(32, 32), OutFormat::Webp);
        write_as(&dir, "2.png", &sample(32, 32), OutFormat::Png);
        write_as(&dir, "3.jpg", &sample(32, 32), OutFormat::Jpeg);
        // 先頭バイトは PNG だが中身が壊れている
        let mut broken = encode(&sample(8, 8), &opts(OutFormat::Png)).unwrap();
        broken.truncate(40);
        std::fs::write(dir.join("4.png"), broken).unwrap();

        let svc = ConvertService::new();
        let entries = svc.add_paths(vec![dir.clone()]).unwrap();
        // 壊れたファイルは、取り込みの時点でエラー行になり、変換の対象から外れる
        let bad = entries.iter().find(|e| e.name == "4.png").unwrap();
        assert!(bad.error.is_some());
        let ids: Vec<_> = entries
            .iter()
            .filter(|e| e.error.is_none())
            .map(|e| e.id.clone())
            .collect();
        assert_eq!(ids.len(), 3);
        // 取り込みの後で消えたファイルは、その 1 枚だけが失敗する
        std::fs::remove_file(dir.join("3.jpg")).unwrap();
        let rec = Arc::new(Rec::default());
        svc.run(ids, opts(OutFormat::Jpeg), out.clone(), rec.clone())
            .unwrap();
        for _ in 0..200 {
            if rec.fin.lock().unwrap().is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let fin = rec.fin.lock().unwrap().clone().expect("finished");
        assert_eq!(fin.ok, 2, "{fin:?} {:?}", rec.items.lock().unwrap());
        assert_eq!(fin.failed, 1);
        assert!(!fin.cancelled);
        for n in ["1.jpg", "2.jpg"] {
            let img = image::open(out.join(n)).unwrap();
            assert_eq!((img.width(), img.height()), (32, 32));
        }
        // 元のファイルは変わらない
        assert!(dir.join("1.webp").exists() && dir.join("4.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[tokio::test]
    async fn run_requires_valid_ids_and_options() {
        let svc = ConvertService::new();
        let rec = Arc::new(Rec::default());
        let out = temp_dir();
        assert!(
            svc.run(
                vec!["nope".into()],
                opts(OutFormat::Png),
                out.clone(),
                rec.clone()
            )
            .is_err()
        );
        let mut bad = opts(OutFormat::Jpeg);
        bad.quality = 0;
        assert!(svc.run(vec![], bad, out.clone(), rec).is_err());
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn settings_patch_validates() {
        let mut s = ConvertSettings::default();
        assert!(
            s.apply(ConvertSettingsPatch {
                quality: Some(0),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            s.apply(ConvertSettingsPatch {
                background: Some("white".into()),
                ..Default::default()
            })
            .is_err()
        );
        s.apply(ConvertSettingsPatch {
            format: Some(OutFormat::Png),
            quality: Some(50),
            ..Default::default()
        })
        .unwrap();
        assert_eq!((s.format, s.quality), (OutFormat::Png, 50));
        assert_eq!(
            default_output_dir(Some(PathBuf::from("/p"))),
            PathBuf::from("/p").join("gazooo-converted")
        );
    }
}
