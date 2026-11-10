//! The KHGIS logo, drawn in code (no image files): a graticule globe on a deep teal rounded
//! square, matching the app icon and the mark on the web page.

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

const C: f32 = 256.0;
/// The globe's radius on the 512-unit grid.
const R: f32 = 184.0;

fn solid(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
    let mut p = Paint { anti_alias: true, ..Paint::default() };
    p.set_color_rgba8(r, g, b, a);
    p
}

/// A circle's quarter as a cubic: control points at 0.5523 of the radius.
const K: f32 = 0.552_284_8;

/// An ellipse centred on the globe, as four cubic segments. `rx` of `R` is the globe's edge;
/// narrower values are the meridians drawn across it.
fn ellipse(rx: f32, ry: f32) -> Option<tiny_skia::Path> {
    let (kx, ky) = (rx * K, ry * K);
    let mut pb = PathBuilder::new();
    pb.move_to(C, C - ry);
    pb.cubic_to(C + kx, C - ry, C + rx, C - ky, C + rx, C);
    pb.cubic_to(C + rx, C + ky, C + kx, C + ry, C, C + ry);
    pb.cubic_to(C - kx, C + ry, C - rx, C + ky, C - rx, C);
    pb.cubic_to(C - rx, C - ky, C - kx, C - ry, C, C - ry);
    pb.close();
    pb.finish()
}

/// The KHGIS logo as a premultiplied RGBA pixmap of `size`×`size` pixels (16..=1024): a globe
/// with its graticule on a deep teal rounded square. No lettering: the mark is drawn as small as
/// a title bar, where letters over the graticule stop being readable.
pub fn pixmap(size: u32) -> Option<Pixmap> {
    let size = size.clamp(16, 1024);
    let mut pm = Pixmap::new(size, size)?;
    let t = Transform::from_scale(size as f32 / 512.0, size as f32 / 512.0);
    // The tile.
    let tile = {
        let (x0, y0, x1, y1, r) = (16.0f32, 16.0f32, 496.0f32, 496.0f32, 104.0f32);
        let k = r * K;
        let mut pb = PathBuilder::new();
        pb.move_to(x0 + r, y0);
        pb.line_to(x1 - r, y0);
        pb.cubic_to(x1 - r + k, y0, x1, y0 + r - k, x1, y0 + r);
        pb.line_to(x1, y1 - r);
        pb.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
        pb.line_to(x0 + r, y1);
        pb.cubic_to(x0 + r - k, y1, x0, y1 - r + k, x0, y1 - r);
        pb.line_to(x0, y0 + r);
        pb.cubic_to(x0, y0 + r - k, x0 + r - k, y0, x0 + r, y0);
        pb.close();
        pb.finish()?
    };
    pm.fill_path(&tile, &solid(0x0E, 0x3F, 0x44, 255), FillRule::Winding, t, None);
    // The globe, then its graticule.
    pm.fill_path(&ellipse(R, R)?, &solid(0x10, 0x56, 0x5C, 255), FillRule::Winding, t, None);
    let grid = solid(0x9F, 0xD8, 0xDF, 235);
    let stroke = Stroke { width: 9.0, ..Stroke::default() };
    // Parallels: chords of the globe, so each stops at its edge.
    for dy in [-98.0f32, 0.0, 98.0] {
        let half = (R * R - dy * dy).max(0.0).sqrt();
        let mut pb = PathBuilder::new();
        pb.move_to(C - half, C + dy);
        pb.line_to(C + half, C + dy);
        if let Some(path) = pb.finish() {
            pm.stroke_path(&path, &grid, &stroke, t, None);
        }
    }
    // Meridians, and the globe's edge.
    for rx in [68.0f32, R] {
        pm.stroke_path(&ellipse(rx, R)?, &grid, &stroke, t, None);
    }
    let mut pb = PathBuilder::new();
    pb.move_to(C, C - R);
    pb.line_to(C, C + R);
    if let Some(path) = pb.finish() {
        pm.stroke_path(&path, &grid, &stroke, t, None);
    }
    Some(pm)
}

/// The logo as straight (unpremultiplied) RGBA bytes, for window icons.
pub fn rgba(size: u32) -> Option<(u32, Vec<u8>)> {
    let pm = pixmap(size)?;
    let mut out = Vec::with_capacity(pm.data().len());
    for p in pm.pixels() {
        let c = p.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Some((pm.width(), out))
}

/// The logo as an ICO file holding PNG images from 16 to 256 px (Windows Vista and later read
/// these), for the exe icon and the Windows installers.
pub fn ico() -> Option<Vec<u8>> {
    const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
    let mut images = Vec::new();
    for size in SIZES {
        images.push((size, pixmap(size)?.encode_png().ok()?));
    }
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&u16::try_from(images.len()).ok()?.to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (size, png) in &images {
        // A dimension of 0 means 256.
        let dim = u8::try_from(*size).unwrap_or(0);
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&u32::try_from(png.len()).ok()?.to_le_bytes());
        ico.extend_from_slice(&u32::try_from(offset).ok()?.to_le_bytes());
        offset += png.len();
    }
    for (_, png) in &images {
        ico.extend_from_slice(png);
    }
    Some(ico)
}

/// A 24-bit BMP of `width`×`height` white pixels with the logo (`size` px) drawn at `x`, `y`,
/// for installer wizard images. Sizes are clamped to 1..=2048.
pub fn bmp(size: u32, width: u32, height: u32, x: i32, y: i32) -> Option<Vec<u8>> {
    let (w, h) = (width.clamp(1, 2048), height.clamp(1, 2048));
    let (side, px) = rgba(size)?;
    let row = (w as usize * 3).div_ceil(4) * 4;
    let len = 54 + row * h as usize;
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&u32::try_from(len).ok()?.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&u32::try_from(row * h as usize).ok()?.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&[0; 8]);
    // Rows run bottom to top, pixels are BGR, each row padded to four bytes.
    for r in (0..h as i64).rev() {
        let start = out.len();
        for c in 0..w as i64 {
            let (lx, ly) = (c - i64::from(x), r - i64::from(y));
            let mut bgr = [255u8; 3];
            if (0..i64::from(side)).contains(&lx) && (0..i64::from(side)).contains(&ly) {
                let i = usize::try_from((ly * i64::from(side) + lx) * 4).ok()?;
                if let Some(p) = px.get(i..i + 4) {
                    let a = u32::from(p[3]);
                    let over = |v: u8| ((u32::from(v) * a + 255 * (255 - a)) / 255) as u8;
                    bgr = [over(p[2]), over(p[1]), over(p[0])];
                }
            }
            out.extend_from_slice(&bgr);
        }
        out.resize(start + row, 0);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn draws_at_every_size() {
        for s in [0, 16, 32, 256, 5000] {
            let (w, px) = super::rgba(s).unwrap();
            assert_eq!(px.len() as u32, w * w * 4);
            assert!(px.chunks(4).any(|c| c[3] == 255), "logo has opaque pixels");
        }
    }

    #[test]
    fn bmp_has_padded_rows_and_the_logo() {
        let bmp = super::bmp(32, 50, 40, 9, 4).unwrap();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(bmp.len(), 54 + 152 * 40);
        assert!(bmp[54..].chunks(3).any(|c| c != [255, 255, 255]), "logo pixels");
        // A logo placed off the canvas leaves it white.
        let blank = super::bmp(32, 10, 10, 500, -500).unwrap();
        assert!(blank[54..].chunks(32).all(|r| r[..30].iter().all(|&b| b == 255)));
    }

    #[test]
    fn ico_lists_seven_png_images() {
        let ico = super::ico().unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 7, 0]);
        // The first image starts right after the directory, with the PNG signature.
        let offset = u32::from_le_bytes(ico[18..22].try_into().unwrap()) as usize;
        assert_eq!(offset, 6 + 16 * 7);
        assert_eq!(&ico[offset..offset + 4], b"\x89PNG");
    }
}
