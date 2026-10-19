# Axentra Outlook

A Linux desktop email, calendar, people and to-do client written in Rust
(Dioxus 0.7 on the system WebKitGTK webview), designed to look and work like
the new Outlook for Windows.

> "Outlook" is a Microsoft trademark. The working name lives in one constant
> (`axentra_ui::APP_NAME`); rename the product before distributing it.

## Install (Zorin OS 18 / Ubuntu 24.04, x86_64)

```bash
sudo apt install ./axentra-outlook_0.1.0-1_amd64.deb
```

This installs to `/opt/axentra/outlook`, adds **Axentra Outlook** to the
application menu and registers the app with Axentra Command. Remove it with
`sudo apt remove axentra-outlook`; your data in
`~/.local/share/axentra-outlook/` stays until you delete it.

To connect Microsoft 365 / Outlook.com, IMAP or Gmail accounts, see
[docs/ACCOUNTS.md](docs/ACCOUNTS.md).

## Run from source

```bash
cargo run --release            # desktop app (binary: axentra-outlook)
cargo run -p axentra-preview   # same UI in a browser at http://localhost:8090/app
cargo test --workspace
```

The desktop app keeps its data in `~/.local/share/axentra-outlook/axentra.db`
and creates a demo account on first launch (set `AXENTRA_NO_DEMO=1` to skip).

See `docs/ARCHITECTURE.md` for the design and `docs/spec/` for the behaviour
specification.

## Package

```bash
cargo build --release --locked -p axentra-desktop
python3 packaging/linux/package.py --output dist
```

Produces `dist/axentra-outlook_<version>-1_amd64.deb` and `dist/SHA256SUMS.txt`.

## Credits

App icon: Axentra Glass + Precision icon kit (app id `outlook`), in
`packaging/linux/icons/`.

Icons: [Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons),
MIT License, © Microsoft Corporation (`crates/axentra-ui/icons/LICENSE-fluentui-system-icons.txt`).
