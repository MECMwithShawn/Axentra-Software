# Security scans: Axentra PDF Suite

Every release installer is scanned on [VirusTotal](https://www.virustotal.com) (70+ antivirus engines)
before it is published. Each link opens the report for that exact file (matched by its SHA-256), so you can
check the file you downloaded is the one that was scanned.

| Version | File | Result | Scanned | Report |
|---|---|---|---|---|
| 0.3.0 | `AxentraPDF-0.3.0-Setup.exe` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/b3b0e29c08e7a71d5b4e8ea8446ae4590e988f0f1f693a25c6618e358c6fe739) |
| 0.3.0 | `AxentraPDF-0.3.0.msi` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/704ace20e151879e581875a34c3b619d33ab089b069c8c705b51c97133b9cf6a) |

SHA-256 of each file:

- `AxentraPDF-0.3.0-Setup.exe`: `b3b0e29c08e7a71d5b4e8ea8446ae4590e988f0f1f693a25c6618e358c6fe739`
- `AxentraPDF-0.3.0.msi`: `704ace20e151879e581875a34c3b619d33ab089b069c8c705b51c97133b9cf6a`

Record each result as *detections / engines* (for example `0 / 72`) with the scan date. Screenshots of the
reports go in [`scans/`](scans/), named `<version>-<file>.png`.

## About false positives

Axentra is a Python application packaged with PyInstaller. A few antivirus engines flag PyInstaller apps
generically (names like `Trojan.Generic`, `Wacatac`, `ML.Attribute`), especially when they are unsigned and
new. If a release shows a low number of such detections, it is reported to the vendor as a false positive and
noted here.

## Code signing

The 0.3.0 installers are **not code-signed**. Windows SmartScreen therefore warns on first run, and PCs with
Smart App Control turned on block them. Code signing (an OV/EV certificate or Microsoft Trusted Signing) is
planned; this section will name the signer once releases are signed.
