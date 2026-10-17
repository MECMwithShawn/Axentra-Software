# Axentra Command on Linux

Verified 2026-10-17 on Zorin OS 18.1 (Ubuntu 24.04), x86_64. Source version **0.1.11**; **development preview**, not a published stable Linux release.

## Run

Open **Axentra Command** from the application menu, or run:

```bash
~/.local/share/axentra/apps/command/AxentraCommand
```

The preview is installed for the desktop user, with a separate folder for each app. Data and profiles live outside the application folder. Installation does not enable startup or change default applications.

## Available

Nine-app catalog, including Archive; per-user and /opt/axentra discovery, VERSION files, process monitoring, launching, custom manifests, native tray and optional sign-in startup. POSIX paths and argument quoting are preserved.

## Verification

20 Rust tests passed; 1 interactive test skipped. Native GET/POST roundtrip passed. Horizon queried all nine installed applications successfully.

## Remaining work and requirements

Linux apps keep separate tray icons. Windows installers are not offered on Linux. Building projects from the board and the Windows terminal action are not Linux build workflows; use the documented CLI. Horizon Mail summaries currently cover the primary Outlook profile only.

## Screenshot

![Axentra Command Linux preview](screenshots/linux-2026-10-17.png)

Native Linux catalog. Screenshots show the test environment at capture time.
