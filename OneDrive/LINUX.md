# Axentra OneDrive on Linux

Verified 2026-10-20 on Zorin OS 18.1 (Ubuntu 24.04), x86_64, GNOME on Wayland. Source version **0.1.0**; **development preview**, not a published release.

## Run

Open **Axentra OneDrive** from the application menu, or run:

```bash
~/.local/share/axentra/apps/onedrive/AxentraOneDrive
```

The sync service runs as the systemd user unit `axentra-onedrive.service`, and the `axentra-onedrive` command controls it from a terminal. Settings are in `~/.config/axentra/onedrive`, sign-in and sync state in `~/.local/share/axentra/onedrive`, and downloaded copies in `~/.cache/axentra/onedrive`. Unlike the other previews, installation enables the service and the tray at sign-in, as the Windows client does.

## Available

Files On-Demand through FUSE, two-way sync with Microsoft Graph's delta feed, chunked uploads, hash-checked downloads, Windows conflict naming, pin and free up space, sharing links, version history and restore, pause, bandwidth limits, desktop notifications, a StatusNotifierItem tray icon, the activity panel, Settings with a choice of three app icon colours, and GNOME Files badges and menu. The first scan of a large drive shows how many items it has listed and resumes after a restart instead of starting over.

## Verification

31 Rust tests passed: 18 end-to-end tests on a real FUSE mount against a Graph mock (on-demand download, uploads including chunked large files, rename identity, conflicts, deletes in both directions, pin and free, save-by-rename history, delta paging and expiry, token refresh, throttling, invalid names, sharing, version restore, resuming an interrupted first scan, starting before the network is up, and stopping while a folder is still open), plus unit tests including QuickXorHash against a bit-level model. Clippy reported no warnings.

On the desktop: signed in to a real personal account with its own app registration; the mount listed the account's files and downloaded on demand. The tray registered with the AppIndicator host, its menu paused and resumed the service over D-Bus, and the panel opened with the GPU renderer. The service restarts in under a second, and after a boot without a network it shows "You're offline" until the network is up. Switching the icon colour replaced the installed desktop icon, and switching back restored it. The installer upgraded an earlier test install without re-downloading.

## Remaining work and requirements

Needs your own Microsoft Entra app registration (free); none is bundled. Uploads, conflicts and deletes are verified against the Graph mock, not yet against a real account. GNOME Files badges were tested in a harness rather than a restarted Files window. During testing a desktop program read every file in two folders, downloading them; downloads now log the requesting program so it can be blocked like thumbnailers are. Wayland doesn't let the panel position itself above the tray icon. Not done: cloud thumbnails, folder backup, Choose folders, Personal Vault, multiple accounts and push notifications.

## Screenshot

![Axentra OneDrive Linux preview](screenshots/linux-2026-10-19.png)

Rendered from the app's UI with demo data; no real account appears.
