# Axentra Outlook

A Linux desktop email, calendar, people and to-do client written in Rust
(Dioxus 0.7 on the system WebKitGTK webview), designed to look and work like
the new Outlook for Windows.

> "Outlook" is a Microsoft trademark. The working name lives in one constant
> (`axentra_ui::APP_NAME`); rename the product before distributing it.

## Install (Zorin OS 18 / Ubuntu 24.04, x86_64)

```bash
sha256sum -c SHA256SUMS.txt
sudo apt install ./axentra-outlook_0.1.4-1_amd64.deb
```

This installs to `/opt/axentra/outlook`, adds **Axentra Outlook** to the
application menu and registers the app with Axentra Command.

**Upgrading from 0.1.3 or earlier:** quit Axentra Outlook first (or right
after installing). Those versions don't take the one-window lock, so while one
is still open the new version won't start beside it; it tells you to close the
old window. Once you have closed it, opening Axentra Outlook again works
straight away, even while that message is still showing.

Opening it again while it runs (from Command, the menu or a terminal) shows
the open window instead of starting a second copy. On X11 the window comes to
the front. On Wayland it comes to the front only when the launcher passes an
activation token (GNOME's app grid and dash do); a launch without one, such as
from Axentra Command or a terminal, makes GNOME show an "Axentra Outlook is
ready" notification instead (click it to switch), and a minimized window stays
minimized until then. If the open copy has stopped responding, a new launch
says so after about 3 seconds (in a message you close) rather than open a
second copy.
Changes per version are in [CHANGELOG.md](CHANGELOG.md). Remove it with
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
One copy runs per database (opening it again shows the open window); it keeps
a lock file next to the database, `axentra.db.instance`. `AXENTRA_DB=/path/to/test.db`
gives a separate profile that runs alongside your own. `axentra-outlook --version` prints the
version and `--help` the options; other arguments (files, `mailto:` links) are
ignored with a note.
Its header doubles as the window's title bar; Settings › General › Appearance ›
"Use the system title bar" (or `AXENTRA_SYSTEM_TITLEBAR=1`) restores the
desktop's own title bar.

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
