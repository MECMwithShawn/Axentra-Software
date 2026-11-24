# Changelog

## 0.1.5 (preview, 2026-10-30)

- **Lines no longer overlap after changing the font size:** rows stayed where they were first drawn, so a bigger font
  or zoom piled lines on top of each other until the file was reopened.
- **Ctrl+scroll wheel zooms** the text anywhere in the window, in 10% steps from 10% to 500% like Windows Notepad
  (Ctrl+= and Ctrl+- use the same steps). Touchpad scrolling adds up to a step instead of racing.
- **Each layout has its usual font size and line spacing:** Notepad 15 px with tight lines, Notepad++ 14 px, Code
  14 px with VS Code's spacing. Upgrading resets the font settings once to the current layout's.
- The editor font list names only installed fonts. WebKitGTK replaced a missing font with a proportional one instead
  of trying the next, which broke the character grid.

## 0.1.4 (preview, 2026-10-26)

- **Highlights line up with the text:** if the editor font finished loading after the editor measured it,
  selections, find matches and the cursor drifted to the right of the words. The editor now measures again once
  fonts are ready and checks a drawn line against the width it expects.
- The About box says 25 encodings (it said 26).

## 0.1.3 (preview, 2026-10-26)

- **Closing always works:** the "save changes?" question when closing the window is a native GTK dialog, so it
  shows even if the page stops drawing. If the page doesn't finish closing within 3 seconds the window exits
  anyway, and if it doesn't respond to *Save* you're offered *Close Anyway*. Unsaved tabs are backed up as you
  type, so they come back next time.
- **No GPU rendering:** WebKit's GPU renderer could stop repainting the window on a hybrid Intel/NVIDIA laptop, which
  left the save prompt invisible and the window stuck. `AXN_GPU=1` turns GPU rendering back on.
- A request that fails inside the app always gets an answer, so nothing waits on it forever.

## 0.1.2 (preview, 2026-10-21)

- Closing the window with unsaved changes asks Save / Don't Save / Cancel. Settings › Files › *Unsaved changes when
  closing* can keep them for next time instead, like Windows 11 Notepad.

## 0.1.1 (preview, 2026-10-21)

- Closing the last tab returns to the start page, which now has *New Note* and *New Code File…*.
- Untitled tabs reuse the lowest free number (Untitled-1 again, not Untitled-34).

## 0.1.0 (preview, 2026-10-21)

- First version: Notepad, Notepad++ and Code layouts, tabs and sessions, highlighting for 46 languages, multiple
  cursors and column selection, find and replace in files, encodings, folding, bookmarks, macros, explorer, quick
  open and command palette, a silver suite icon, and an Axentra Command manifest.
