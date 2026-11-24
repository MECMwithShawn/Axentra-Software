//! Runs the page's script tests (tests/*_test.js) with gjs, so `cargo test` covers the editor too.
use std::path::Path;
use std::process::Command;

#[test]
fn page_scripts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut tests: Vec<_> = std::fs::read_dir(root.join("tests"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with("_test.js")))
        .collect();
    tests.sort();
    if tests.is_empty() {
        eprintln!("no tests/*_test.js yet");
        return;
    }
    if Command::new("gjs").arg("--version").output().is_err() {
        eprintln!("gjs isn't installed; skipping {} script tests", tests.len());
        return;
    }
    let mut failed = Vec::new();
    for t in &tests {
        let out = Command::new("gjs").arg(t).current_dir(root).output().expect("run gjs");
        let name = t.file_name().unwrap().to_string_lossy().into_owned();
        if out.status.success() {
            eprintln!("{name}: ok");
        } else {
            eprintln!("{name} failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "script tests failed: {failed:?}");
}
