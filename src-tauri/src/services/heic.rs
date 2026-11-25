//! HEIC / HEIF のデコード(docs/43 §13)。純 Rust の `zune-heic` を使い、C/C++ のビルド要件を増やさない。
//!
//! - 判定は拡張子ではなく先頭の `ftyp` ボックスのブランドで行う。AVIF は対象外。
//! - 向き(`irot`・`imir`)とクロップはデコーダが適用済みで、10bit は 8bit に落ちる(トーンマッピングなし)。
//! - 複数画像を含む場合はプライマリ画像 1 枚だけを返す。

use image::DynamicImage;
use zune_heic::HeifDecoder;
use zune_heic::zune_core::bytestream::ZCursor;
use zune_heic::zune_core::colorspace::ColorSpace;

/// HEIC(HEVC)を示すブランド。
const HEIC_BRANDS: [&[u8; 4]; 6] = [b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis"];
/// 汎用の HEIF ブランド(AVIF も使うため、AVIF のブランドが無いときだけ HEIC とみなす)。
const GENERIC_BRANDS: [&[u8; 4]; 2] = [b"mif1", b"msf1"];
const AVIF_BRANDS: [&[u8; 4]; 2] = [b"avif", b"avis"];

/// 先頭バイトが HEIC / HEIF か(`ftyp` のメジャー・互換ブランドで判定)。
pub fn is_heic(head: &[u8]) -> bool {
    if head.len() < 12 || &head[4..8] != b"ftyp" {
        return false;
    }
    let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let end = size.clamp(12, head.len());
    // メジャーブランド(8..12)、マイナーバージョン(12..16)を飛ばして互換ブランド(16..)
    let mut brands: Vec<&[u8]> = vec![&head[8..12]];
    if end > 16 {
        let (chunks, _) = head[16..end].as_chunks::<4>();
        brands.extend(chunks.iter().map(|c| c.as_slice()));
    }
    let has = |set: &[&[u8; 4]]| {
        brands
            .iter()
            .any(|b| set.iter().any(|s| s.as_slice() == *b))
    };
    if has(&AVIF_BRANDS) {
        return false;
    }
    has(&HEIC_BRANDS) || has(&GENERIC_BRANDS)
}

/// 向き適用後の寸法をヘッダだけから読む(画素は展開しない)。
pub fn dimensions(data: &[u8]) -> Result<(u32, u32), String> {
    let mut d = HeifDecoder::new(ZCursor::new(data));
    d.decode_headers().map_err(|e| format!("{e:?}"))?;
    match (d.width(), d.height()) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Ok((w as u32, h as u32)),
        _ => Err("寸法を読めません".into()),
    }
}

/// デコードして 8bit の画像にする。`max_pixels` を超える画像は展開せずに断る。
pub fn decode(data: &[u8], max_pixels: u64) -> Result<DynamicImage, String> {
    let (w, h) = dimensions(data)?;
    if u64::from(w) * u64::from(h) > max_pixels {
        return Err("画像が大きすぎます".into());
    }
    // 不正なデータでのパニックは 1 枚の失敗として扱う
    let data = data.to_vec();
    std::panic::catch_unwind(move || {
        let mut d = HeifDecoder::new(ZCursor::new(&data));
        let px = d.decode().map_err(|e| format!("{e:?}"))?;
        let cs = d.colorspace().ok_or("色空間を取得できません")?;
        let (w, h) = (
            d.width().ok_or("寸法を読めません")? as u32,
            d.height().ok_or("寸法を読めません")? as u32,
        );
        let bad = || "デコード結果の大きさが不正です".to_string();
        Ok(match cs {
            ColorSpace::RGB => {
                DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, px).ok_or_else(bad)?)
            }
            ColorSpace::RGBA => {
                DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, px).ok_or_else(bad)?)
            }
            ColorSpace::Luma => {
                DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, px).ok_or_else(bad)?)
            }
            ColorSpace::LumaA => DynamicImage::ImageLumaA8(
                image::GrayAlphaImage::from_raw(w, h, px).ok_or_else(bad)?,
            ),
            other => return Err(format!("未対応の色空間です: {other:?}")),
        })
    })
    .unwrap_or_else(|_| Err("デコード中にエラーが起きました".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ftyp(major: &[u8; 4], compat: &[&[u8; 4]]) -> Vec<u8> {
        let size = 16 + 4 * compat.len();
        let mut v = (size as u32).to_be_bytes().to_vec();
        v.extend(b"ftyp");
        v.extend(major);
        v.extend([0, 0, 0, 0]);
        for c in compat {
            v.extend(*c);
        }
        v
    }

    #[test]
    fn detects_heic_by_brand_not_avif() {
        assert!(is_heic(&ftyp(b"heic", &[b"mif1", b"heic"])));
        assert!(is_heic(&ftyp(b"mif1", &[b"heic"])));
        assert!(is_heic(&ftyp(b"mif1", &[b"miaf"])));
        assert!(!is_heic(&ftyp(b"avif", &[b"mif1", b"miaf"])));
        assert!(!is_heic(&ftyp(b"mif1", &[b"avif", b"miaf"])));
        assert!(!is_heic(&ftyp(b"mp42", &[b"isom"])));
        assert!(!is_heic(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"));
        assert!(!is_heic(b"short"));
    }

    #[test]
    fn broken_heic_is_an_error_not_a_panic() {
        let mut data = ftyp(b"heic", &[b"mif1"]);
        data.extend(vec![0xAB; 64]);
        assert!(dimensions(&data).is_err());
        assert!(decode(&data, 1_000_000).is_err());
    }

    /// 実ファイルでの確認用(`GAZOOO_HEIC_SAMPLE=<path> cargo test heic_real -- --ignored`)。
    #[test]
    #[ignore]
    fn heic_real_file_decodes() {
        let p = std::env::var("GAZOOO_HEIC_SAMPLE").expect("GAZOOO_HEIC_SAMPLE");
        let data = std::fs::read(p).unwrap();
        assert!(is_heic(&data));
        let (w, h) = dimensions(&data).unwrap();
        let img = decode(&data, 100_000_000).unwrap();
        assert_eq!((img.width(), img.height()), (w, h));
        assert!(decode(&data, 10).is_err());
    }
}
