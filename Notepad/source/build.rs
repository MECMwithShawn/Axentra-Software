// Stages the page (ui/) for include_str!, so a checkout that's missing a page file still
// builds the backend and its tests; the build warns about anything missing.
fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=ui");
    for name in ["index.html", "app.css", "app.js", "core.js", "editor.js", "lang.js"] {
        let src = std::path::Path::new("ui").join(name);
        println!("cargo:rerun-if-changed={}", src.display());
        let body = std::fs::read(&src).unwrap_or_else(|_| {
            println!("cargo:warning=ui/{name} is missing; the page will be incomplete");
            Vec::new()
        });
        std::fs::write(out.join(name), body).unwrap();
    }
}
