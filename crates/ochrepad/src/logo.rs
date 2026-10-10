//! The ochrepad logo: an ochre pad with three lines of text. Used for the
//! window icon and, through `build.rs` (which includes this file by path),
//! for the icon embedded in the Windows executable. So it uses no crates.

pub const OCHRE: [u8; 3] = [0xCC, 0x77, 0x22];
/// The lines of text.
pub const PAPER: [u8; 3] = [0xFF, 0xF3, 0xE2];

/// A rounded rectangle in the unit square: min x, min y, max x, max y,
/// corner radius.
type RoundRect = (f32, f32, f32, f32, f32);

const PAD: RoundRect = (0.03, 0.03, 0.97, 0.97, 0.24);
const LINE_H: f32 = 0.1;
const LINES: [RoundRect; 3] = [line(0.34, 0.52), line(0.5, 0.36), line(0.66, 0.46)];

const fn line(y: f32, len: f32) -> RoundRect {
    (
        0.24,
        y - LINE_H / 2.0,
        0.24 + len,
        y + LINE_H / 2.0,
        LINE_H / 2.0,
    )
}

fn inside((x0, y0, x1, y1, r): RoundRect, x: f32, y: f32) -> bool {
    let dx = (x0 + r - x).max(x - (x1 - r)).max(0.0);
    let dy = (y0 + r - y).max(y - (y1 - r)).max(0.0);
    dx * dx + dy * dy <= r * r
}

/// The logo as `size` × `size` straight (not premultiplied) RGBA pixels,
/// antialiased, rows top to bottom.
pub fn rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4; // samples per axis
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (mut on_pad, mut on_line) = (0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = (x as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let py = (y as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    if inside(PAD, px, py) {
                        on_pad += 1;
                        if LINES.iter().any(|&l| inside(l, px, py)) {
                            on_line += 1;
                        }
                    }
                }
            }
            let t = if on_pad == 0 {
                0.0
            } else {
                on_line as f32 / on_pad as f32
            };
            for c in 0..3 {
                let v = f32::from(OCHRE[c]) * (1.0 - t) + f32::from(PAPER[c]) * t;
                out.push(v.round() as u8);
            }
            out.push((on_pad as f32 / (SS * SS) as f32 * 255.0).round() as u8);
        }
    }
    out
}

/// A Windows `.ico` file with the logo at each of `sizes` (at most 256),
/// as uncompressed 32-bit images.
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| ico_image(s)).collect();
    let mut out = Vec::new();
    let u16le = |out: &mut Vec<u8>, v: u16| out.extend_from_slice(&v.to_le_bytes());
    let u32le = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    u16le(&mut out, 0); // reserved
    u16le(&mut out, 1); // icon
    u16le(&mut out, sizes.len() as u16);
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&size, image) in sizes.iter().zip(&images) {
        let dim = if size >= 256 { 0 } else { size as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        u16le(&mut out, 1); // planes
        u16le(&mut out, 32); // bits per pixel
        u32le(&mut out, image.len() as u32);
        u32le(&mut out, offset);
        offset += image.len() as u32;
    }
    for image in images {
        out.extend_from_slice(&image);
    }
    out
}

/// One icon image: a BITMAPINFOHEADER, BGRA rows bottom to top, and an
/// all-clear AND mask (transparency comes from the alpha channel).
fn ico_image(size: u32) -> Vec<u8> {
    let pixels = rgba(size);
    let mask_row = size.div_ceil(32) * 4;
    let mut out = Vec::new();
    for v in [40, size, size * 2] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    for _ in 0..6 {
        out.extend_from_slice(&0u32.to_le_bytes()); // no compression; defaults
    }
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            out.extend_from_slice(&[pixels[i + 2], pixels[i + 1], pixels[i], pixels[i + 3]]);
        }
    }
    out.resize(out.len() + (mask_row * size) as usize, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_is_ochre_with_transparent_corners_and_cream_lines() {
        let px = rgba(64);
        assert_eq!(px.len(), 64 * 64 * 4);
        let at = |x: usize, y: usize| &px[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        assert_eq!(at(0, 0)[3], 0, "rounded corner is transparent");
        assert_eq!(at(58, 40), &[OCHRE[0], OCHRE[1], OCHRE[2], 255]);
        assert_eq!(at(20, 22), &[PAPER[0], PAPER[1], PAPER[2], 255]);
    }

    #[test]
    fn ico_has_a_directory_entry_per_size() {
        let file = ico(&[16, 256]);
        assert_eq!(&file[..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(file[6], 16, "first entry is 16 px");
        assert_eq!(file[6 + 16], 0, "256 px is written as 0");
        let len16 = 40 + 16 * 16 * 4 + 16 * 4;
        let len256 = 40 + 256 * 256 * 4 + 256 * 32;
        assert_eq!(file.len(), 6 + 32 + len16 + len256);
        let offset = u32::from_le_bytes([file[18], file[19], file[20], file[21]]) as usize;
        assert_eq!(
            &file[offset..offset + 4],
            &40u32.to_le_bytes(),
            "header size"
        );
    }
}
