# Changelog

## 0.1.2 (prepared 2026-10-14; unpublished)

- Source is now 0.1.2, ahead of the handoff's 0.1.0 installer. Fixed UTF-8 slicing crashes that could permanently stop sorting; per-message panic handling skips failures and records crashes in crash.log.
- Sorting status page shows progress, waiting/sorted counts, timing, recent decisions and category totals, with pause and resume.
- Resizable folder, message and reading panes remember their widths; double-click resets a divider.
- AI work no longer blocks opening, archiving or moving messages. Fixed native TLS for Microsoft HTTPS calls and made sign-in cancellation release the screen immediately.
- Per-user Inno installer includes optional desktop/startup entries and preserves the local mail copy on uninstall unless removal is selected.

Source: `e43dccaa360d7218e29af00205518a393b133e69`. See [release preparation](releases/0.1.2.md).

