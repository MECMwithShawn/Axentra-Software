# Changelog

## 0.1.0 (development preview, 2026-10-19; unpublished)

- First version: Files On-Demand OneDrive folder over FUSE, two-way sync through Microsoft Graph, Windows-style conflict copies, pin and free up space, sharing links, version history and restore, pause, bandwidth limits and notifications.
- Tray icon, activity panel, Settings and Version history windows in Fluent light and dark; GNOME Files status badges and OneDrive menu.
- Installs per user into `~/.local/share/axentra/apps/onedrive` with a systemd user service. Upgrades the earlier working build, Cirrus, in place, keeping sign-in and sync state.
- Fast, safe shutdown during long first scans; on-demand downloads log the program that requested them.
- The first scan shows how many items it has listed and resumes after a restart instead of starting over.
- Ice glass app icon from the Axentra Icon Kit, with periwinkle and azure in Settings > Appearance.
- Starting before the network is up shows "You're offline" instead of a sync error.
- Stopping no longer waits for programs that still have a folder in OneDrive open; it closes the folder after 5 seconds and logs which programs had it open.

No binary release, installer or checksums are published for this version.
