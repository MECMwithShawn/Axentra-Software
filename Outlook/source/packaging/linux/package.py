#!/usr/bin/env python3
"""Build the Axentra Outlook .deb for Zorin OS 18 / Ubuntu 24.04 x86_64.

Layout follows the rest of the Axentra suite, so Axentra Command finds it:
  /opt/axentra/outlook/axentra-outlook   program
  /opt/axentra/outlook/VERSION           version Command reads
  /usr/bin/axentra-outlook               symlink
  /usr/share/applications/axentra-outlook.desktop
  /usr/share/icons/hicolor/<n>x<n>/apps/axentra-outlook.png
  /usr/share/axentra/command/apps/outlook.json   Command app manifest
  /opt/axentra/outlook/models/bge-small-en-v1.5/  search model (meaning search)

The search model (BAAI/bge-small-en-v1.5, MIT licence) is downloaded once at a
pinned revision, checked against its SHA-256, and halved to 16-bit floats
(crates/axentra-embed/examples/to_f16.rs) into target/models/.

Usage: cargo build --release --locked -p axentra-desktop
       python3 packaging/linux/package.py --output dist
"""
import argparse, hashlib, json, pathlib, shutil, subprocess, tempfile, tomllib, urllib.request

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
ICON_SIZES = [16, 24, 32, 48, 64, 128, 256, 512]
DEPENDS = ", ".join([
    # Direct NEEDED libraries of the release binary (readelf -d), by package.
    "libc6 (>= 2.39)",
    "libgcc-s1",
    "libglib2.0-0t64",
    "libgtk-3-0t64",
    "libgdk-pixbuf-2.0-0",
    "libcairo2",
    "libwebkit2gtk-4.1-0",
    "libjavascriptcoregtk-4.1-0",
    "libsoup-3.0-0",
    "libssl3t64",
    "libxdo3",
    "xdg-utils",
])
RECOMMENDS = "gnome-keyring | kwalletmanager"
MODEL = "bge-small-en-v1.5"
MODEL_URL = "https://huggingface.co/BAAI/bge-small-en-v1.5/resolve/5c38ec7c405ec4b44b94cc5a9bb96e735b38267a/"
MODEL_FILES = {
    "config.json": "094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750",
    "vocab.txt": "07eced375cec144d27c900241f3e339478dec958f92fddbc551f295c992038a3",
    "model.safetensors": "3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad",
}
# A copy still running from before the upgrade keeps the old program; the new
# one won't start beside it (it tells the user so), so say it here too.
POSTINST = """#!/bin/sh
set -e
if command -v gtk-update-icon-cache >/dev/null 2>&1; then gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true; fi
if command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database -q /usr/share/applications || true; fi
if [ "$1" = configure ]; then
  for p in /proc/[0-9]*; do
    case "$(readlink "$p/exe" 2>/dev/null || true)" in
      /opt/axentra/outlook/axentra-outlook*)
        echo "Axentra Outlook is still running (the version from before this update). Close it, then open it again."
        break ;;
    esac
  done
fi
exit 0
"""


def sha256(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def search_model() -> pathlib.Path:
    """The shipped model directory, fetched and converted on first use."""
    out = ROOT / "target/models" / MODEL
    if (out / "model.safetensors").is_file():
        return out
    raw = ROOT / "target/models" / f"{MODEL}-download"
    raw.mkdir(parents=True, exist_ok=True)
    for name, digest in MODEL_FILES.items():
        dest = raw / name
        if not dest.is_file() or sha256(dest) != digest:
            print(f"Downloading {MODEL}/{name}")
            urllib.request.urlretrieve(MODEL_URL + name, dest)
        if sha256(dest) != digest:
            raise SystemExit(f"{name}: checksum mismatch, not packaging it")
    tmp = ROOT / "target/models" / f"{MODEL}-tmp"
    shutil.rmtree(tmp, ignore_errors=True)
    tmp.mkdir()
    for name in ("config.json", "vocab.txt"):
        shutil.copy2(raw / name, tmp / name)
    subprocess.run(["cargo", "run", "--release", "--locked", "-q", "-p", "axentra-embed", "--example", "to_f16", "--",
                    str(raw / "model.safetensors"), str(tmp / "model.safetensors")], check=True, cwd=ROOT)
    tmp.rename(out)
    return out


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--output", type=pathlib.Path, required=True)
    p.add_argument("--binary", type=pathlib.Path, default=ROOT / "target/release/axentra-outlook")
    a = p.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if not a.binary.is_file():
        raise SystemExit("Build first: cargo build --release --locked -p axentra-desktop")
    out = a.output.resolve()
    out.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="axentra-outlook-package-") as td:
        stage = pathlib.Path(td)
        app = stage / "opt/axentra/outlook"
        app.mkdir(parents=True)
        shutil.copy2(a.binary, app / "axentra-outlook")
        subprocess.run(["strip", "--strip-unneeded", str(app / "axentra-outlook")], check=True)
        (app / "VERSION").write_text(version + "\n")
        shutil.copytree(search_model(), app / "models" / MODEL)
        shutil.copy2(HERE / f"LICENSE-{MODEL}.txt", app / "models" / MODEL / "LICENSE.txt")

        bin_dir = stage / "usr/bin"
        bin_dir.mkdir(parents=True)
        (bin_dir / "axentra-outlook").symlink_to("/opt/axentra/outlook/axentra-outlook")

        apps = stage / "usr/share/applications"
        apps.mkdir(parents=True)
        shutil.copy2(HERE / "axentra-outlook.desktop", apps)
        subprocess.run(["desktop-file-validate", str(apps / "axentra-outlook.desktop")], check=True)

        for n in ICON_SIZES:
            dest = stage / f"usr/share/icons/hicolor/{n}x{n}/apps/axentra-outlook.png"
            dest.parent.mkdir(parents=True)
            shutil.copy2(HERE / f"icons/axentra-outlook-{n}.png", dest)

        manifest = stage / "usr/share/axentra/command/apps/outlook.json"
        manifest.parent.mkdir(parents=True)
        json.loads((HERE / "command-app.json").read_text())  # must be valid JSON
        shutil.copy2(HERE / "command-app.json", manifest)

        doc = stage / "usr/share/doc/axentra-outlook"
        doc.mkdir(parents=True)
        (doc / "copyright").write_text(
            "Axentra Outlook\nCopyright Axentra contributors\n\n"
            "Icons: Fluent UI System Icons, MIT License, (c) Microsoft Corporation.\n"
            "Search model: BAAI/bge-small-en-v1.5, MIT License, (c) 2022 staoxiao (see\n"
            "/opt/axentra/outlook/models/bge-small-en-v1.5/LICENSE.txt); stored as\n"
            "16-bit floats.\n"
            "\"Outlook\" is a trademark of Microsoft Corporation. Axentra Outlook is an\n"
            "independent client and is not affiliated with or endorsed by Microsoft.\n"
        )

        control = stage / "DEBIAN"
        control.mkdir()
        size_kb = sum(f.stat().st_size for f in stage.rglob("*") if f.is_file() and not f.is_symlink()) // 1024
        (control / "control").write_text(f"""Package: axentra-outlook
Version: {version}-1
Section: mail
Priority: optional
Architecture: amd64
Maintainer: Axentra
Installed-Size: {size_kb}
Depends: {DEPENDS}
Recommends: {RECOMMENDS}
Homepage: https://github.com/MECMwithShawn/Axentra-Software/tree/main/Outlook
Description: Mail, calendar, people and to-do with the Outlook layout
 Axentra Outlook is a native Rust desktop client for Linux that mirrors the
 layout and keyboard of the new Outlook for Windows. It works offline with a
 local demo mailbox and connects to Microsoft 365 / Outlook.com (Microsoft
 Graph) and IMAP/SMTP accounts. Built for Zorin OS 18 and Ubuntu 24.04.
""")
        (control / "postinst").write_text(POSTINST)
        (control / "postrm").write_text("#!/bin/sh\nset -e\nif command -v gtk-update-icon-cache >/dev/null 2>&1; then gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true; fi\nif command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database -q /usr/share/applications || true; fi\n")

        stage.chmod(0o755)  # the temporary directory is created 0700
        for path in stage.rglob("*"):
            if path.is_symlink():
                continue
            executable = path.is_dir() or path == app / "axentra-outlook" or path.parent == control and path.name in ("postinst", "postrm")
            path.chmod(0o755 if executable else 0o644)

        deb = out / f"axentra-outlook_{version}-1_amd64.deb"
        subprocess.run(["dpkg-deb", "--root-owner-group", "-Zxz", "--build", str(stage), str(deb)], check=True)
    digest = hashlib.sha256(deb.read_bytes()).hexdigest()
    (out / "SHA256SUMS.txt").write_text(f"{digest}  {deb.name}\n")
    print(deb)
    print(f"sha256 {digest}")


if __name__ == "__main__":
    main()
