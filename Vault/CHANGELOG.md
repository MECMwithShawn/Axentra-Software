# Changelog

## 0.1.0 (prepared 2026-10-14; unpublished)

- Prepared first-release packaging from audit/fixes; the manifests still identify the app as 0.1.0. No version was invented or bumped.
- Versioned backup store with retention, manual backup, restore, legacy-backup migration, configurable roots and read-only backup sessions; uninstall preserves backups.
- Credential changes re-encrypt reachable backups and adjacent old-key copies; cancelled changes remove the new key file and recovery handles locked originals.
- Bounded database/key/passkey reads and KDF work, compatible effective key-file handling, same-site icon fetching and reported background-job failures.
- Tray menu and single-instance commands integrate with Axentra Command; new suite app icon.

Source: `6995683265daf24e5d2d46d561f1c1d388365157`. See [release preparation](releases/0.1.0.md).

