// Embeds the Glass + Precision icon (16-256 px optical sizes) and version info in the Windows exe,
// so Explorer, Start and pinned taskbar shortcuts show the brand mark.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app/vault.ico")
            .set("FileDescription", "Axentra Vault")
            .set("ProductName", "Axentra Vault")
            .set("CompanyName", "Axentra");
        res.compile().expect("embed Windows icon resource");
    }
    println!("cargo:rerun-if-changed=assets/app/vault.ico");
}
