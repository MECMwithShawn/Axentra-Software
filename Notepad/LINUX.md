# Axentra Notepad on Linux

Built and tested 2026-10-26 on Zorin OS 18.1 (Ubuntu 24.04), x86_64. Version **0.1.4**; a **preview**, not a stable
release. There's no published package yet: build it from [source](source/).

## Install and run

```bash
cd source
python3 packaging/linux/package.py --output dist
sudo apt install ./dist/axentra-notepad_0.1.4-1_amd64.deb
```

Open **Axentra Notepad** from the application menu, from Axentra Command, or run `axentra-notepad` (with files, or
a folder to open it in the explorer). The package installs to `/opt/axentra/notepad` with a `VERSION` file,
`/usr/bin/axentra-notepad`, a menu entry with a *New window* action, icons in the hicolor theme and an Axentra
Command manifest, so Command lists it like the other suite apps. The menu entry is registered for plain text,
Markdown and common source files, but installation doesn't make it the default editor.

## Available

Native WebKitGTK window with an Axentra title bar, three layouts (Notepad, Notepad++, Code), tabs and a session
under `~/.local/share/axentra/notepad`, syntax highlighting for 46 languages, multiple cursors and column
selection, find and replace in the file and across a folder, 25 encodings with line-ending conversion, folding,
bookmarks, macros, a file explorer, quick open and a command palette. One window per session: opening a file while
Axentra Notepad is running adds a tab to the open window (`--new-window` opens a separate one).

Closing the window with unsaved changes asks Save / Don't Save / Cancel in a native GTK dialog, so the question
shows even if the page stops drawing, and the window exits within 3 seconds even if the page doesn't answer.
Unsaved tabs are backed up as you type. The page renders without the GPU, because WebKit's GPU renderer froze
(stopped repainting) on a hybrid Intel/NVIDIA laptop; set `AXN_GPU=1` to turn it back on.

## Verification

- 21 Rust tests passed (`cargo test`): encodings and round trips, line endings, atomic saves, sessions, folder
  listing, find and replace in files, window commands.
- 109 editor-core tests and 606 highlighter tests passed under gjs (run by `cargo test`): editing, undo, multiple
  cursors, wrapping, folding, search and replace, incremental highlighting, and detection, tokens, states and
  speed for every language (100,000 lines of JavaScript, Python, Rust, HTML or Markdown tokenize in 50–160 ms).
- In a real window under Xvfb: typing, multi-cursor editing, find/replace, find in files, the palette, folding,
  session restore, all three layouts, the light theme, the clipboard, handing a file to the running window, and
  every close path (Cancel, Don't Save, Save through the GTK save dialog, and Don't Save or Save with the page
  process paused).
- A 13 MB, 200,000-line file opens in under half a second and types at about 2 ms per keystroke.
- 0.1.3 was installed and used on this Zorin machine; the window-freeze it fixes was diagnosed on the same machine.
  0.1.4 was built and its packaging checked, but not installed here yet.

## Not yet

No split editors, IntelliSense or language servers, extensions, integrated terminal, Git integration or debugger.
Printing uses WebKit's print dialog and wasn't tested. Right-to-left text isn't laid out correctly (positions assume
a monospace grid). Files over 100 MB don't open. Find in the editor uses JavaScript regular expressions and *Find in
Files* uses Rust's regex syntax, so look-around works in the first and not the second. There's no Windows build.
