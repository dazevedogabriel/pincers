//! Turns `assets/logo.svg` into the executable's icon (what Explorer, the
//! desktop and shortcuts show). To change the icon, replace the SVG and run
//! `cargo build --release` again.

use resvg::{tiny_skia, usvg};
use std::path::PathBuf;

/// Sizes Windows picks from for Explorer views, the taskbar and HiDPI.
const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

fn main() {
    println!("cargo:rerun-if-changed=assets/logo.svg");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let svg = std::fs::read("assets/logo.svg").expect("assets/logo.svg");
    let tree = usvg::Tree::from_data(&svg, &usvg::Options::default()).expect("invalid assets/logo.svg");
    let pngs: Vec<(u32, Vec<u8>)> = SIZES.iter().map(|&s| (s, render_png(&tree, s))).collect();

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("pincers.ico");
    std::fs::write(&ico, ico_bytes(&pngs)).expect("write icon");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().unwrap());
    res.set("FileDescription", "Pincers — gerenciador de downloads");
    res.set("ProductName", "Pincers");
    res.compile().expect("embedding the icon needs rc.exe from the Windows SDK");
}

/// Renders the (non-square) logo centred in a transparent square.
fn render_png(tree: &usvg::Tree, side: u32) -> Vec<u8> {
    let mut pix = tiny_skia::Pixmap::new(side, side).unwrap();
    let size = tree.size();
    let scale = side as f32 / size.width().max(size.height());
    let dx = (side as f32 - size.width() * scale) / 2.0;
    let dy = (side as f32 - size.height() * scale) / 2.0;
    resvg::render(tree, tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy), &mut pix.as_mut());
    pix.encode_png().unwrap()
}

/// ICO container with PNG-compressed images (supported since Windows Vista).
fn ico_bytes(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (side, png) in images {
        // 256 is stored as 0 in the one-byte width/height fields.
        let dim = if *side >= 256 { 0 } else { *side as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        out.extend_from_slice(png);
    }
    out
}
