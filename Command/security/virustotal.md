# Security scans: Axentra Command

Every release file is scanned on [VirusTotal](https://www.virustotal.com) (70+ antivirus engines) before it is
published. Each link opens the report for that exact file (matched by its SHA-256), so you can check that the file
you downloaded is the one that was scanned.

| Version | File | Result | Scanned | Report | Screenshot |
|---|---|---|---|---|---|
| 0.1.9 | `AxentraCommand-0.1.9-Setup.exe` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/09fe65292f9acf11fc2a38500c30ee55934e04caab86bcd7c378fb7c83d21a82/detection) | |

SHA-256 of each file:

- `AxentraCommand-0.1.9-Setup.exe`: `09fe65292f9acf11fc2a38500c30ee55934e04caab86bcd7c378fb7c83d21a82`

Record each result as *detections / engines* (for example `0 / 72`) along with the scan date. Screenshots of the
reports go in [`scans/`](scans/), named `<version>-<file>.jpg`.

## About false positives

Command is a small Rust program with an Inno Setup installer. It downloads installers from GitHub (only when you
click Get or Update), starts other programs, and can start with Windows, all for the current user only. A few
antivirus engines flag new, unsigned apps that download and run files with generic names such as `ML.Attribute`
or `Wacatac`. If a release shows a small number of such detections, it is reported to the vendor as a false
positive and noted here.

## Code signing

Not signed yet. Code signing is planned; this section will show the publisher name once releases are signed.

## Prepared 0.1.9 (unpublished)

This is a local rebuild of an already published version; the published assets and checksums remain unchanged.

- [AxentraCommand-0.1.9-Setup.exe](https://www.virustotal.com/gui/file/09fe65292f9acf11fc2a38500c30ee55934e04caab86bcd7c378fb7c83d21a82/detection): scan pending; this is a hash lookup, not a recorded result.

See [release preparation](../releases/0.1.9-rebuild.md) for build validation and pending signing/review work.
