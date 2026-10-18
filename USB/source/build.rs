fn main() {
    println!("cargo:rerun-if-changed=assets/axentra-usb.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = root
        .join("assets/axentra-usb.ico")
        .display()
        .to_string()
        .replace('\\', "/");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let parts: Vec<_> = version.split('.').collect();
    let (a, b, c) = (parts[0], parts[1], parts[2]);
    let rc = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Axentra\0"
      VALUE "FileDescription", "Axentra USB\0"
      VALUE "FileVersion", "{version}\0"
      VALUE "InternalName", "AxentraUSB.exe\0"
      VALUE "OriginalFilename", "AxentraUSB.exe\0"
      VALUE "ProductName", "Axentra USB\0"
      VALUE "ProductVersion", "{version}\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("app.rc");
    std::fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE);
}
