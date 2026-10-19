//! Renders `assets/logo.svg` into the PNG icons the browser extension needs.
//! Run from the repository root:
//! `cargo run --release --example extension_icons [-- <icons folder>]`
//! (default: `../pincers-extension/src/icons`, the extension's repository
//! checked out next to this one).

use resvg::{tiny_skia, usvg};

fn main() {
    let svg = std::fs::read("assets/logo.svg").expect("assets/logo.svg");
    let tree = usvg::Tree::from_data(&svg, &usvg::Options::default()).expect("invalid SVG");
    let dir = std::env::args().nth(1).unwrap_or_else(|| "../pincers-extension/src/icons".into());
    let dir = std::path::Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    for side in [16u32, 32, 48, 96, 128] {
        let mut pix = tiny_skia::Pixmap::new(side, side).unwrap();
        let size = tree.size();
        let scale = side as f32 / size.width().max(size.height());
        let dx = (side as f32 - size.width() * scale) / 2.0;
        let dy = (side as f32 - size.height() * scale) / 2.0;
        resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy), &mut pix.as_mut());
        let path = dir.join(format!("icon{side}.png"));
        pix.save_png(&path).unwrap();
        println!("{}", path.display());
    }
}
