# Security scans: Axentra PDF Suite

Every release installer is scanned on [VirusTotal](https://www.virustotal.com) (70+ antivirus engines)
before it is published. Each link opens the report for that exact file (matched by its SHA-256), so you can
check the file you downloaded is the one that was scanned.

| Version | File | Result | Scanned | Report | Screenshot |
|---|---|---|---|---|---|
| 0.3.3 | `AxentraPDF-0.3.3-Setup.exe` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/f0f8e18a19f721a5b613a8b424f09fa0e1cbceec63933a899ecd443a8897a6f3/detection) | |
| 0.3.3 | `AxentraPDF-0.3.3.msi` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/8c84611a568213d691a7ac1b5c5f2f303a7ca296c8bf685854628ea03fe5deba/detection) | |
| 0.3.3 | `AxentraPDF-0.3.3-source.zip` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/2ad8b3734360017b22a7dba237e7574cb15c0f92a0d70ebfd897859a28a96389/detection) | |

SHA-256 of each file:

- `AxentraPDF-0.3.3-Setup.exe`: `f0f8e18a19f721a5b613a8b424f09fa0e1cbceec63933a899ecd443a8897a6f3`
- `AxentraPDF-0.3.3.msi`: `8c84611a568213d691a7ac1b5c5f2f303a7ca296c8bf685854628ea03fe5deba`
- `AxentraPDF-0.3.3-source.zip`: `2ad8b3734360017b22a7dba237e7574cb15c0f92a0d70ebfd897859a28a96389`

Only the current release is listed; older versions have been withdrawn.

Record each result as *detections / engines* (for example `0 / 72`) with the scan date. Screenshots of the
reports go in [`scans/`](scans/), named `<version>-<file>.jpg`.

## About false positives

Axentra is a Python application packaged with PyInstaller. A few antivirus engines flag PyInstaller apps
generically (names like `Trojan.Generic`, `Wacatac`, `ML.Attribute`), especially when they are unsigned and
new. If a release shows a low number of such detections, it is reported to the vendor as a false positive and
noted here.

## Code signing

The installers are **not code-signed**. Windows SmartScreen therefore warns on first run, and PCs with
Smart App Control turned on block them. Code signing (an OV/EV certificate or Microsoft Trusted Signing) is
planned; this section will name the signer once releases are signed.
