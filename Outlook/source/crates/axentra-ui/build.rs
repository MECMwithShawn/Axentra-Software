//! Generates `icon_svg(name)` from every `icons/*.svg` file, so adding an
//! icon is just dropping a file into `icons/` (no shared list to edit).

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("icons");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<(String, String)> = std::fs::read_dir(&dir)
        .expect("icons dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "svg"))
        .map(|p| {
            println!("cargo:rerun-if-changed={}", p.display());
            (p.file_stem().unwrap().to_string_lossy().into_owned(), p.display().to_string())
        })
        .collect();
    names.sort();
    let mut out = String::from("/// SVG markup for a Fluent icon name, e.g. `\"mail\"` or `\"star_filled\"`.\npub fn icon_svg(name: &str) -> Option<&'static str> {\n    match name {\n");
    for (name, path) in &names {
        writeln!(out, "        {name:?} => Some(include_str!({path:?})),").unwrap();
    }
    out.push_str("        _ => None,\n    }\n}\n\n/// Every available icon name.\npub const ICON_NAMES: &[&str] = &[\n");
    for (name, _) in &names {
        writeln!(out, "    {name:?},").unwrap();
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("icons.rs");
    std::fs::write(dest, out).unwrap();
}
