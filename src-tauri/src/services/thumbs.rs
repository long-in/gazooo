//! サムネイル生成(docs/03-data-model.md「ファイル配置」)。

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;

/// サムネイルの長辺(px)。
pub const THUMB_MAX: u32 = 384;
const JPEG_QUALITY: u8 = 85;

/// 画像 ID に対応するサムネイルの相対パス(`/` 区切り)。
pub fn thumb_rel_path(image_id: &str) -> String {
    format!("thumbs/{image_id}.jpg")
}

/// 画像データから、長辺 `THUMB_MAX` の JPEG サムネイルを作る。デコードできない形式は `None`。
pub fn make_thumbnail(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    // 縦横比を保ったまま収める(元が小さければ拡大しない)
    let small = if img.width() > THUMB_MAX || img.height() > THUMB_MAX {
        img.thumbnail(THUMB_MAX, THUMB_MAX)
    } else {
        img
    };
    let rgb = small.to_rgb8(); // JPEG はアルファ非対応
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(Cursor::new(&mut out), JPEG_QUALITY)
        .encode_image(&rgb)
        .ok()?;
    Some(out)
}

#[cfg(test)]
pub fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    use image::{ImageFormat, Rgba, RgbaImage};
    let img = RgbaImage::from_fn(width, height, |x, y| {
        Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
    });
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .unwrap();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dims(jpeg: &[u8]) -> (u32, u32) {
        let img = image::load_from_memory(jpeg).unwrap();
        (img.width(), img.height())
    }

    #[test]
    fn large_image_is_scaled_to_long_edge_keeping_aspect() {
        let thumb = make_thumbnail(&png_bytes(1024, 512)).unwrap();
        assert_eq!(&thumb[..2], &[0xFF, 0xD8], "JPEG のマジックナンバー");
        assert_eq!(dims(&thumb), (384, 192));

        let portrait = make_thumbnail(&png_bytes(512, 1024)).unwrap();
        assert_eq!(dims(&portrait), (192, 384));
    }

    #[test]
    fn small_image_is_not_enlarged() {
        let thumb = make_thumbnail(&png_bytes(100, 80)).unwrap();
        assert_eq!(dims(&thumb), (100, 80));
    }

    #[test]
    fn undecodable_data_returns_none() {
        assert!(make_thumbnail(b"not an image").is_none());
        assert!(make_thumbnail(&[]).is_none());
    }

    #[test]
    fn thumb_path_is_stable_and_relative() {
        assert_eq!(thumb_rel_path("abc"), "thumbs/abc.jpg");
    }
}
