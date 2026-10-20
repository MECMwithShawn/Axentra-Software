# Changelog

## 0.1.2 (pre-release, 2026-10-20)

- **Merged title bar:** the blue header is now the window's title bar, like the new Outlook for Windows. Minimize,
  maximize/restore and close follow your desktop's button layout; drag empty header space to move the window,
  double-click to maximize, resize from the edges and corners. Rounded corners when floating, square when
  maximized or tiled. **Settings › General › Appearance › Use the system title bar** brings the normal title bar
  back.
- Reviewed independently for window behaviour and layout; fixed before release: a click right after a drag
  being ignored, re-grabbing just after a drag maximizing the window, menus and dialogs covering the window
  buttons, and too little drag space at narrow widths.

Source: [`source/`](source/) (snapshot of commit `0f6f30e`). See [release notes](releases/0.1.2.md).

## 0.1.1 (pre-release, 2026-10-19)

- **Backup:** Settings › Accounts › Backup saves an encrypted backup (AES-256-GCM, Argon2id password key) of
  account setups, settings and device-only data to the OneDrive folder every month and on demand; restore
  previews the backup and merges without duplicates. Passwords and sign-in tokens are never included.
- The Microsoft client ID is remembered after the first sign-in, so it isn't needed again.
- Cross-module actions: Add to contacts from a message, Schedule meeting from a contact with the attendee
  filled in, open a task from My Day or reminders; toast actions and an Undo send countdown.
- Account status in the folder pane, Add account from the folder pane and account manager, RSVP responses sent
  to Microsoft 365, new Mail settings pages (attachment reminder, blocked and safe senders, sweep rules).
- Fixed: the message read window now refreshes when you open another message in release builds.

Source: [`source/`](source/) (snapshot of commit `da0c289`). See [release notes](releases/0.1.1.md).

## 0.1.0 (pre-release, 2026-10-19)

First release. Linux only (Zorin OS 18 / Ubuntu 24.04, x86_64).

- Outlook-style shell: header with search, app rail, Simplified and Classic ribbon with Customize, My Day,
  notifications, help, settings with search, Fluent 2 light/dark themes, colour themes, three densities.
- Mail: folder tree and favorites, Focused/Other, conversations, columns layout, multi-select, context menus,
  drag to folders, flags, categories, pin, snooze, junk and block, sweep, search keywords, Undo.
- Reading pane with sandboxed HTML, blocked remote images, attachments and meeting responses.
- Compose with recipient autocomplete, rich text, attachments, autosaved drafts, Schedule send and Undo send.
- Calendar with day / work week / week / month views, drag to create and move, recurrence, reminders.
- People with contact lists and vCard/CSV import/export; To Do with smart lists, steps and repeating tasks.
- Microsoft 365 / Outlook.com sync over Microsoft Graph; IMAP/SMTP accounts; Secret Service keyring storage.
- `.deb` package with menu entry, icons and an Axentra Command manifest.

Source: [`source/`](source/) (snapshot of commit `6926c2f`). See [release notes](releases/0.1.0.md).
