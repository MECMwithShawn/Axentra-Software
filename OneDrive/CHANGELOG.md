# Changelog

## 0.3.0-dev (unreleased) — mirror to Proton Drive (preview, offline-tested only)

The package version stays 0.2.0 until a release.

- **New `axentra-onedrive mirror plan|run|status`:** copies OneDrive to Proton Drive one way.
  - Only adds folders and new files. Nothing is ever deleted, moved, renamed, trashed or replaced on either side.
  - Content that changed, or a same-named file with other content, is recorded as diverged and left alone.
  - `run` needs `--only <folder>` for now: a whole-drive run is refused until it is approved and given its own `--all` flag.
  - `run` asks before starting unless `--yes` is given.
  - `--round-trip` downloads each file back to check it (L4).
- **New `axentra-onedrive proton status|login`:**
  - Checks Proton's official `proton-drive` CLI (pinned to 0.9.0 by SHA-512 and version).
  - Reports only whether it is signed in.
  - Starts its browser sign-in.
- **Optional `[mirror]` table in config.toml:**
  - Keys: `enabled`, `dest_root`, `staging_dir`, `staging_cap_bytes`, `proton_cli` and `changed_content`.
  - Off by default. Existing settings files load unchanged, and the table isn't written while it holds only defaults.
- **The OneDrive sign-in is only read:** the mirror commands never refresh, save or remove `tokens.json`, so they can't race the running sync service.
- **New crates:**
  - `axentra-clouddrive-mirror`: manifest, planner, executor and readiness report.
  - The Proton CLI adapter in `axentra-clouddrive-provider`.
- **Docs:** `docs/clouddrive/PROTON_INTEGRATION.md` and `docs/clouddrive/MIGRATION_READINESS.md`. No production measurement has been made yet.
- **Unchanged:** all existing commands, identifiers and sync behaviour.

## 0.2.0 — Axentra CloudDrive (display rename; provider groundwork)

- Renamed to Axentra CloudDrive, an independent client that syncs with Microsoft OneDrive. Only the name people see changed: the tray, panel, Settings, notifications, app menu, Axentra Command card, GNOME Files menu, service descriptions and the sign-in page now say Axentra CloudDrive. The icon is unchanged.
- Everything an install depends on keeps its old id, so upgrading keeps sign-in, sync state and settings: the `axentra-onedrive` and `AxentraOneDrive` commands, `~/OneDrive`, the `~/.config/axentra/onedrive` and `~/.local/share/axentra/onedrive` folders, the systemd units, D-Bus names, extended attributes, the desktop file, the icon theme name and the Command id `onedrive`.
- The GNOME Shell extension places the panel whether its window is titled Axentra CloudDrive or, from an older build, Axentra OneDrive.

## 0.1.0 (development preview, 2026-10-19; unpublished)

- First version: Files On-Demand OneDrive folder over FUSE, two-way sync through Microsoft Graph, Windows-style conflict copies, pin and free up space, sharing links, version history and restore, pause, bandwidth limits and notifications.
- Tray icon, activity panel, Settings and Version history windows in Fluent light and dark; GNOME Files status badges and OneDrive menu.
- Installs per user into `~/.local/share/axentra/apps/onedrive` with a systemd user service. Upgrades the earlier working build, Cirrus, in place, keeping sign-in and sync state.
- Fast, safe shutdown during long first scans; on-demand downloads log the program that requested them.
- The first scan shows how many items it has listed and resumes after a restart instead of starting over.
- Ice glass app icon from the Axentra Icon Kit, with periwinkle and azure in Settings > Appearance.
- Starting before the network is up shows "You're offline" instead of a sync error.
- The tray panel follows the Windows 11 activity center: "OneDrive - Personal" with Help & Settings, a one-line status, recent files with file-type icons and their folder as a link, and Open folder / View online / Recycle bin along the bottom; storage moved into the Help & Settings menu. On X11 it opens above the tray icon, stays out of the taskbar and closes when you click elsewhere.
- Listed in Axentra Command through a manifest the installer adds; the card follows the icon colour.
- Starting the app again (from the app menu or Command) shows the running one's panel or Settings instead of adding a second tray icon.
- Windows now match `axentra-onedrive.desktop`, so the taskbar shows the app icon instead of a generic one.
- Stopping no longer waits for programs that still have a folder in OneDrive open; it closes the folder after 5 seconds and logs which programs had it open.
- Cloud thumbnails: online-only photos and videos get OneDrive's previews in the freedesktop thumbnail cache, so GNOME Files, GTK file choosers and Eye of GNOME show them without downloading. Adds the `GetThumbnail` D-Bus method and `axentra-onedrive thumbnail`.
- GNOME's own thumbnailers (whose names the kernel shortens) and the RAW helper `dcraw` are now recognised and kept from downloading online-only files.
- A second sync service can no longer take the D-Bus name from the running one.
- The tray icon runs as the user service `axentra-onedrive-tray.service` instead of an autostart entry, so it comes back if it crashes or is closed by something else; the installer restarts it through systemd.
- On Wayland, the panel and Settings no longer sit in the taskbar as empty windows before they are first opened.
- Settings polish: Network rows describe each setting instead of repeating the dropdown, show "Limit to ___ KB/s" in order, and line up; About has a "Source code and help" link and a compact Slint credit.
- When the desktop session ends, the tray app exits cleanly instead of crashing during shutdown.
- A full multi-agent review confirmed 96 problems (57, then 29 and 10 in follow-up rounds), now fixed with tests (46 to 172). The most serious: files moved out of a folder that was then deleted were deleted online; a save-by-rename during an upload could delete the file online; overwriting or truncating a file while it downloaded could lose the new content; writes only counted at close; a folder 409 could loop forever; long uploads hit the 120 s timeout and never resumed; signing out or pausing left uploads running. Others covered upload/download backoff instead of repeated toasts, case-only renames, big-folder listing, Download all files confirmation, config mistakes no longer overwritten, keyboard and screen-reader access, Nautilus badges, logout and different-account safety, and the service no longer restarting forever on problems only the user can fix.
- On Wayland the panel opens above the tray icon with the focus, through a small GNOME Shell extension the installer adds (active from the next sign-in), and has a close button. Long folder names in the activity list are shortened instead of the action.
- Folder listings are attributed to the program that made them before the reply, so cloud thumbnails no longer miss folders listed by short-lived threads.

No binary release, installer or checksums are published for this version.
