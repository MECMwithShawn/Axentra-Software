//! The Axentra Glass + Precision suite icon set:
//! dark glass tiles, individual product accents, crisp glyphs and restrained halos.
//!
//!   cargo run --release -- [OUT_DIR]     writes <app>-<size>.png, <app>.ico, preview.png
//!
//! `suite.rs` owns APPS, accent(), glyph(), sample() and rgba(). Coordinates
//! run from -0.5 to 0.5, with y down. This file only encodes PNG/ICO and previews.
//! No image libraries, fonts, network services or generated artwork are needed.

use std::path::PathBuf;

include!("suite.rs");

// ---------------------------------------------------------------- PNG / ICO writers

fn crc(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

pub fn png(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc(&c).to_be_bytes());
    }
    let mut raw = Vec::with_capacity(h * (w * 4 + 1));
    for y in 0..h {
        raw.push(0);
        raw.extend_from_slice(&rgba[y * w * 4..(y + 1) * w * 4]);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push((i + 1 == blocks.len()) as u8);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A 32-bit BMP (DIB) icon entry: BITMAPINFOHEADER, bottom-up BGRA, empty AND mask.
fn bmp_entry(rgba: &[u8], n: usize) -> Vec<u8> {
    let mask_row = ((n + 31) / 32) * 4;
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(n as i32).to_le_bytes());
    out.extend_from_slice(&((n * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&((n * n * 4 + mask_row * n) as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    for y in (0..n).rev() {
        for x in 0..n {
            let i = (y * n + x) * 4;
            out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    out.extend(std::iter::repeat(0u8).take(mask_row * n));
    out
}

/// A .ico in the classic layout every toolchain accepts (rc.exe, Inno Setup, .NET, PyInstaller):
/// BMP entries below 256 px, a PNG entry at 256.
pub fn ico(app: &str, sizes: &[usize]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&n| if n >= 256 { png(n, n, &rgba(app, n)) } else { bmp_entry(&rgba(app, n), n) }).collect();
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len();
    for (n, img) in sizes.iter().zip(&images) {
        let b = if *n >= 256 { 0 } else { *n as u8 };
        out.extend_from_slice(&[b, b, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += img.len();
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

/// One sheet: every app at 160 px, then 32 px and 16 px blown up 4× (nearest) to judge pixels.
fn preview() -> (usize, usize, Vec<u8>) {
    let (cell, pad) = (200usize, 20usize);
    let w = APPS.len() * cell + pad;
    let h = 160 + 128 + 64 + pad * 5;
    let mut img = vec![0u8; w * h * 4];
    for p in img.chunks_mut(4) {
        p.copy_from_slice(&[11, 14, 30, 255]);
    }
    let mut blit = |src: &[u8], n: usize, scale: usize, ox: usize, oy: usize| {
        for y in 0..n * scale {
            for x in 0..n * scale {
                let s = ((y / scale) * n + x / scale) * 4;
                let d = ((oy + y) * w + ox + x) * 4;
                let a = src[s + 3] as f32 / 255.0;
                for k in 0..3 {
                    img[d + k] = (src[s + k] as f32 * a + img[d + k] as f32 * (1.0 - a)).round() as u8;
                }
            }
        }
    };
    for (i, app) in APPS.iter().enumerate() {
        let ox = pad + i * cell;
        blit(&rgba(app, 160), 160, 1, ox, pad);
        blit(&rgba(app, 32), 32, 4, ox, pad * 2 + 160);
        blit(&rgba(app, 16), 16, 4, ox, pad * 3 + 160 + 128);
    }
    (w, h, img)
}

fn main() {
    let out = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("out"));
    std::fs::create_dir_all(&out).unwrap();
    for app in APPS {
        for n in [16usize, 24, 32, 48, 64, 128, 256, 512] {
            std::fs::write(out.join(format!("{app}-{n}.png")), png(n, n, &rgba(app, n))).unwrap();
        }
        std::fs::write(out.join(format!("{app}.ico")), ico(app, &[16, 20, 24, 32, 40, 48, 64, 256])).unwrap();
    }
    let (w, h, img) = preview();
    std::fs::write(out.join("preview.png"), png(w, h, &img)).unwrap();
    println!("wrote {} icons to {}", APPS.len(), out.display());
}
