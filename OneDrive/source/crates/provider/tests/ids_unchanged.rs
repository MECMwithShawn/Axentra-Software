//! The CloudDrive rename is display-only: every identifier that installs, the
//! daemon, file managers and the desktop depend on (AUDIT-AND-PLAN §8) must
//! still be in the source tree, word for word.

use std::path::{Path, PathBuf};

/// The workspace root (`source/`).
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    let path = root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Files that are installed or looked up by name.
const FILES: &[&str] = &[
    "extras/systemd/axentra-onedrive.service",
    "extras/systemd/axentra-onedrive-tray.service",
    "extras/desktop/axentra-onedrive.desktop",
    "extras/command/onedrive.json",
    "extras/gnome-shell/axentra-onedrive@mecmwithshawn.github.io/extension.js",
    "extras/gnome-shell/axentra-onedrive@mecmwithshawn.github.io/metadata.json",
    "extras/nautilus/axentra_onedrive_nautilus.py",
    "extras/icons/emblems/axentra-onedrive-online.svg",
    "extras/icons/emblems/axentra-onedrive-local.svg",
    "extras/icons/emblems/axentra-onedrive-pinned.svg",
    "extras/icons/emblems/axentra-onedrive-syncing.svg",
    "extras/icons/emblems/axentra-onedrive-error.svg",
    "extras/icons/emblems/axentra-onedrive-shared.svg",
];

/// (file, text that must still appear in it).
const IDS: &[(&str, &str)] = &[
    // D-Bus: daemon and tray names, paths and interfaces.
    (
        "crates/core/src/config.rs",
        r#"DBUS_NAME: &str = "io.github.mecmwithshawn.AxentraOneDrive";"#,
    ),
    (
        "crates/core/src/config.rs",
        r#"DBUS_PATH: &str = "/io/github/mecmwithshawn/AxentraOneDrive";"#,
    ),
    (
        "crates/core/src/config.rs",
        r#"DBUS_IFACE: &str = "io.github.mecmwithshawn.AxentraOneDrive1";"#,
    ),
    (
        "crates/core/src/ipc.rs",
        r#"#[interface(name = "io.github.mecmwithshawn.AxentraOneDrive1")]"#,
    ),
    (
        "crates/core/src/ipc.rs",
        r#"default_service = "io.github.mecmwithshawn.AxentraOneDrive","#,
    ),
    (
        "crates/core/src/ipc.rs",
        r#"default_path = "/io/github/mecmwithshawn/AxentraOneDrive","#,
    ),
    (
        "crates/ui/src/app.rs",
        r#"INSTANCE_NAME: &str = "io.github.mecmwithshawn.AxentraOneDrive.Tray";"#,
    ),
    (
        "crates/ui/src/app.rs",
        r#"INSTANCE_PATH: &str = "/io/github/mecmwithshawn/AxentraOneDrive/Tray";"#,
    ),
    (
        "crates/ui/src/app.rs",
        r#"INSTANCE_IFACE: &str = "io.github.mecmwithshawn.AxentraOneDrive.Tray1";"#,
    ),
    (
        "extras/nautilus/axentra_onedrive_nautilus.py",
        r#"BUS_NAME = "io.github.mecmwithshawn.AxentraOneDrive""#,
    ),
    (
        "extras/nautilus/axentra_onedrive_nautilus.py",
        r#"OBJECT_PATH = "/io/github/mecmwithshawn/AxentraOneDrive""#,
    ),
    // xattrs.
    (
        "crates/core/src/fs.rs",
        r#"PINNED_XATTR: &str = "user.axentra.onedrive.pinned";"#,
    ),
    (
        "crates/core/src/fs.rs",
        r#""user.axentra.onedrive.status","#,
    ),
    (
        "crates/core/src/fs.rs",
        r#""user.axentra.onedrive.shared","#,
    ),
    ("crates/core/src/fs.rs", r#""user.axentra.onedrive.url","#),
    ("crates/core/src/fs.rs", r#""user.axentra.onedrive.error","#),
    (
        "crates/core/src/thumbs.rs",
        r#"NUDGE_XATTR: &str = "user.axentra.onedrive.thumbnail";"#,
    ),
    (
        "extras/nautilus/axentra_onedrive_nautilus.py",
        r#""xattr::axentra.onedrive.status"#,
    ),
    // FUSE.
    (
        "crates/core/src/fs.rs",
        r#"MountOption::FSName("axentra-onedrive".into()),"#,
    ),
    (
        "crates/core/src/fs.rs",
        r#"MountOption::Subtype("axentra-onedrive".into()),"#,
    ),
    // On-disk paths and the pending-upload prefix.
    (
        "crates/core/src/config.rs",
        r#"config.join("axentra/onedrive/config.toml")"#,
    ),
    (
        "crates/core/src/config.rs",
        r#"data.join("axentra/onedrive/tokens.json")"#,
    ),
    (
        "crates/core/src/config.rs",
        r#"data.join("axentra/onedrive/state.sqlite")"#,
    ),
    (
        "crates/core/src/config.rs",
        r#"cache.join("axentra/onedrive/files")"#,
    ),
    (
        "crates/core/src/engine.rs",
        r#"PENDING_PREFIX: &str = ".axentra-onedrive-pending-";"#,
    ),
    // Binaries and crates.
    ("crates/cli/Cargo.toml", r#"name = "axentra-onedrive""#),
    ("crates/ui/Cargo.toml", r#"name = "AxentraOneDrive""#),
    (
        "crates/core/Cargo.toml",
        r#"name = "axentra-onedrive-core""#,
    ),
    (
        "crates/graph/Cargo.toml",
        r#"name = "axentra-onedrive-graph""#,
    ),
    (
        "crates/mock/Cargo.toml",
        r#"name = "axentra-onedrive-mock""#,
    ),
    // Units.
    (
        "extras/systemd/axentra-onedrive.service",
        "ExecStart=@APP@/axentra-onedrive daemon",
    ),
    (
        "extras/systemd/axentra-onedrive-tray.service",
        "ExecStart=@APP@/AxentraOneDrive",
    ),
    // Desktop, shell and icon.
    (
        "extras/desktop/axentra-onedrive.desktop",
        "\nIcon=axentra-onedrive\n",
    ),
    (
        "extras/desktop/axentra-onedrive.desktop",
        "\nStartupWMClass=axentra-onedrive\n",
    ),
    (
        "crates/ui/src/main.rs",
        r#"set_xdg_app_id("axentra-onedrive")"#,
    ),
    (
        "crates/ui/src/tray.rs",
        "fn id(&self) -> String {\n        \"axentra-onedrive\".into()",
    ),
    (
        "crates/core/src/notify.rs",
        "0u32,\n                    \"axentra-onedrive\",",
    ),
    (
        "extras/gnome-shell/axentra-onedrive@mecmwithshawn.github.io/metadata.json",
        r#""uuid": "axentra-onedrive@mecmwithshawn.github.io""#,
    ),
    (
        "extras/gnome-shell/axentra-onedrive@mecmwithshawn.github.io/extension.js",
        "const APP_ID = 'axentra-onedrive';",
    ),
    ("extras/command/onedrive.json", r#""id": "onedrive","#),
    // HTTP User-Agent.
    (
        "crates/graph/src/client.rs",
        "\"AxentraOneDrive/\",\n                    env!(\"CARGO_PKG_VERSION\"),",
    ),
];

#[test]
fn installed_files_keep_their_names() {
    let missing: Vec<&str> = FILES
        .iter()
        .copied()
        .filter(|f| !root().join(f).is_file())
        .collect();
    assert!(missing.is_empty(), "missing: {missing:#?}");
}

#[test]
fn section_8_identifiers_are_unchanged() {
    let missing: Vec<String> = IDS
        .iter()
        .filter(|(file, id)| !read(file).contains(id))
        .map(|(file, id)| format!("{file}: {id:?}"))
        .collect();
    assert!(
        missing.is_empty(),
        "identifiers gone:\n{}",
        missing.join("\n")
    );
}
