// Renders assets/vault-mark.svg to PNGs for previews: `cargo run --example render_mark -- <out dir>`.
fn main() {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: render_mark <out dir>"),
    );
    let data = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/vault-mark.svg"
    ))
    .unwrap();
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).unwrap();
    for size in [256u32, 64, 32, 16] {
        let mut pm = resvg::tiny_skia::Pixmap::new(size, size).unwrap();
        let s = size as f32 / 256.0;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(s, s),
            &mut pm.as_mut(),
        );
        pm.save_png(out.join(format!("vault-mark-{size}.png")))
            .unwrap();
    }
    println!("rendered to {}", out.display());
}
