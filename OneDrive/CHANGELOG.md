# Changelog

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
