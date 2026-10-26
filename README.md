# Axentra Notepad

Text and code editing for the Axentra suite. It's as quick as Windows Notepad for plain text, has Notepad++'s tabs, encodings and line tools, and VS Code's explorer, find in files, command palette and multiple cursors.

## Editor

- **Layouts**: *Notepad* is just the text, with word wrap on and no line numbers. *Notepad++* adds a toolbar, line numbers, a fold margin and bookmarks. *Code* adds the activity bar, explorer, search, breadcrumbs and minimap. Pick one from the welcome page, the title bar or View › Layout; every part can still be turned on or off on its own.
- **Tabs**: tabs open next to the current one and can be dragged to reorder. Middle-click closes a tab, Ctrl+Shift+T reopens one, and a file changed by another program reloads by itself (or asks, if you have unsaved changes). Open tabs survive closing the window. Closing the last tab goes back to the start page (*New Note*, *New Code File…*, Open, Recent), and untitled tabs reuse the lowest free number.
- **Highlighting**: 45+ languages are detected from the file name, extension or shebang. HTML highlights its `<script>` and `<style>` blocks, and Markdown highlights fenced code. CSV/TSV get rainbow columns and logs get coloured levels. Change the language from the status bar or with Ctrl+K M.
- **Cursors and selection**: Alt+click or Ctrl+click adds a cursor. Ctrl+D adds the next occurrence and Ctrl+Shift+L selects all of them. Ctrl+Alt+Up/Down adds cursors above or below. Alt+drag (or Shift+Alt+drag, or Column Selection Mode) makes a column selection. Paste spreads lines across cursors.
- **Editing**: brackets and quotes auto-close and auto-indent, including closing braces that outdent. There's smart Home, Tab/Shift+Tab indenting, toggle comment (Ctrl+/ or Ctrl+Q) and block comment. Line operations include move, copy, duplicate, delete, join, sort (plain, ignoring case, natural, by length), reverse, shuffle, remove duplicates or empty lines, and trim. Text transforms cover case changes, Base64, URL and HTML escaping, and JSON format/minify. You can insert the date and time (F5, as in Notepad), toggle overtype with Insert, and set a tab read-only.
- **Find**: VS Code's find widget handles match case, whole word, regex (with `$1`, `\U…\E` and `\n` in replacements), find in selection, Alt+Enter to select every match and *Find All* into the Search view. Ctrl+Shift+F searches the open folder (or the open tabs), with a replace preview.
- **Navigation**: Ctrl+P is quick open (tabs, recent files and the folder), Ctrl+Shift+P or F1 the command palette, Ctrl+G go to line:column. There are bookmarks (Ctrl+F2, F2, Shift+F2), folding by brackets, indentation, Markdown headings and `#region` (Ctrl+Shift+[ and ], Fold All, Fold Level N), and bracket matching with Ctrl+Shift+\.
- **View**: word wrap (Alt+Z), relative line numbers, whitespace and end-of-line markers (Notepad++'s *Show All Characters*), indent guides, rulers, zoom (Ctrl+wheel, Ctrl+= / Ctrl+- / Ctrl+0), full screen (F11), and dark and light themes.
- **Macros**: Ctrl+Shift+R starts and stops recording and Ctrl+Alt+R plays the macro back. You can also run it N times or to the end of the file, as in Notepad++.

Every command is in the menus and the command palette, and Help › Keyboard Shortcuts lists the keys. Where the three apps disagree, Axentra Notepad uses VS Code's key, except for Ctrl+U / Ctrl+Shift+U (lower/upper case) and Ctrl+F2 (bookmark), which come from Notepad++.

## Files and folders

- **Encodings**: files open in the encoding they use, detected from the byte order mark, valid UTF-8, BOM-less UTF-16 or (otherwise) Windows 1252. *Reopen with encoding* and *Save with encoding* cover UTF-8 (with or without a BOM), UTF-16 LE/BE and 21 legacy code pages (Western, Central European, Cyrillic, Greek, Turkish, Hebrew, Arabic, Baltic, Vietnamese, Thai, Japanese, Chinese and Korean). If a character can't be saved in the chosen encoding, the save stops and names the character instead of writing `?`.
- **Line endings**: CRLF, LF and CR are detected and kept on save. Files with mixed endings are flagged. The page always edits with `\n`.
- **Saving**: saves are atomic (a temp file next to the target, synced, then renamed). They keep the file's permissions, write through symlinks instead of replacing them, and refuse read-only files with a hint to use Save As. Files up to 100 MB open; files with NUL bytes are flagged as binary.
- **Folders**: *Open Folder* shows an explorer. The explorer can create files and folders, rename them and move them to the Trash, show a file in the file manager and open a terminal in a folder. Quick open lists the folder's files and honours `.gitignore` and `.ignore`. It never lists `.git`, `node_modules` or `__pycache__`.
- **Find in files** uses ripgrep's walker and regex engine. It supports match case, whole word, regular expressions, files to include and exclude (`*.rs`, `target`, `src/**/*.ts`) and a switch for ignore files. It skips binary files and files over 16 MB. *Replace in files* only touches the files listed in the results and keeps each file's encoding and line endings. Literal replacements stay literal, and regex replacements expand `$1` and `${name}`.
- **Session**: open tabs come back the next time you start Notepad. While you edit, unsaved text (untitled tabs, and changes to files you haven't saved yet) is written to `~/.local/share/axentra/notepad/backup/` about a second and a half after you stop typing, so it comes back after a crash or power cut. This happens in the default *Ask to save* mode too. Closing the window with unsaved changes asks to save them; *Don't Save* deletes those backups (untitled tabs go away and files reopen as they are on disk), and so does *Don't Save* when you close a single tab. With *Settings › Files › Unsaved changes when closing* set to *Keep for next time*, the window closes without asking and the backed-up changes come back next time, like Notepad++ and Windows 11 Notepad. Turning off *Settings › Files › Remember open tabs* stops the backups, and a *New window* doesn't make any.
- **Closing**: closing the window asks about unsaved changes in a native GTK dialog, so the question shows even if the page stops drawing. If the page doesn't finish closing within 3 seconds, the window exits anyway; unsaved tabs are backed up as you type, so they come back next time. The page renders without the GPU, because WebKit's GPU renderer can freeze on hybrid Intel/NVIDIA laptops. `AXN_GPU=1` turns it back on.
- **One window**: opening a file while Axentra Notepad is running adds a tab to the open window, over a socket in `$XDG_RUNTIME_DIR`. *New window* (`--new-window`) opens a separate window that doesn't restore the session.

## Build and package

```sh
cargo test                                    # Rust tests, plus tests/*_test.js under gjs
out=dist
python3 packaging/linux/package.py --output "$out"   # runs cargo build --release --locked first
sudo apt install "$out"/axentra-notepad_<version>-1_amd64.deb
```

Axentra Command looks for new packages in the folder named by `dist.path` in `axentra.app.json` (`dist/` next to the source), or in your user manifest (`~/.local/share/axentra/command/apps/notepad.json`) if that names another folder. After changing the version in `Cargo.toml`, build once without `--locked` so `Cargo.lock` takes the new version.

The package installs `/opt/axentra/notepad/AxentraNotepad`, `/usr/bin/axentra-notepad`, the launcher (registered for plain text, Markdown and common source files, with a *New window* action) and the suite icons. It also installs `/usr/share/axentra/command/apps/notepad.json`, which puts the app on Axentra Command's board. `package.py` doesn't trust an existing binary: it runs `cargo build --release --locked` itself, in `CARGO_TARGET_DIR` if that's set and `target/` otherwise, and packages the binary cargo reports. Cargo decides what to rebuild, for anything the binary is built from, including `ui/` (through `build.rs`) and `Cargo.toml` changes such as `[profile.release]` settings, so the package holds the binary `cargo build` makes from the current sources. Cargo judges freshness by file times, so a file restored with an older time (`cp -a`, `rsync -a`, `tar -x`) isn't picked up: `touch` it, or run `cargo clean`, before packaging. After copying the binary it asks cargo once more, and if a file was saved during the build (so cargo rebuilds), it packages the new binary instead. It refuses a checkout where `ui/` is incomplete, since `build.rs` only warns about that. Because of `--locked`, it also stops when `Cargo.lock` doesn't match `Cargo.toml`, as it won't right after a version change. The icons are packaged straight from `assets/`. `Installed-Size` is counted the way `dpkg-gencontrol` counts it. `package.py` also writes `SHA256SUMS.txt`, with a line for every `.deb` in the output folder.

## Develop

```sh
cargo run -- notes.txt src/                   # open a file and a folder
cargo run -- --new-window                     # a second, independent window
cargo run -- --serve 8797 -- notes.txt        # page + API on http://127.0.0.1:8797 (no dialogs; in-memory clipboard)
```

In `--serve` mode the file dialogs return nothing, and the clipboard is a string in memory. Set `XDG_DATA_HOME` to keep test sessions and recent files out of your real data.

## How it fits together

The page in `ui/` owns the open documents: text, undo, cursors, highlighting and layout all live in the webview. The Rust side does everything that touches the system:

- `src/files.rs`: encodings, line endings, open, save, stat, recent lists and session backup.
- `src/search.rs`: folder listing, find in files and replace in files.
- `src/fsops.rs`: explorer operations, file manager, terminal and monospace fonts.
- `src/api.rs`: answers the page's `/api` requests.
- `src/instance.rs`: one window per session.
- `src/main.rs`: the window, GTK dialogs and clipboard, `/state` (title, and whether closing must ask the page first) and the dev server.

`build.rs` copies `ui/` into the build, so the page is compiled into the binary. User data lives in `~/.local/share/axentra/notepad`: WebKit storage, `recent.json`, `recent-folders.json`, `session.json` and `backup/`.

The icon comes from the suite renderer in Axentra Command (`tools/suite-icons/src/suite.rs`, app id `notepad`, accent `#C8D4E8`, silver). `src/suite.rs` is a copy of it.
