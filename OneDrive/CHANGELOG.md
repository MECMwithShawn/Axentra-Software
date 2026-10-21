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

No binary release, installer or checksums are published for this version.
