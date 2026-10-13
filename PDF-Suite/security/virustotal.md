# Security scans: Axentra PDF Suite

Every release installer is scanned on [VirusTotal](https://www.virustotal.com) (70+ antivirus engines)
before it is published. Each link opens the report for that exact file (matched by its SHA-256), so you can
check the file you downloaded is the one that was scanned.

| Version | File | Result | Scanned | Report | Screenshot |
|---|---|---|---|---|---|
| 0.3.1 | `AxentraPDF-0.3.1-Setup.exe` | **0 / 68** detections | 2026-10-13 | [VirusTotal](https://www.virustotal.com/gui/file/50244e575db94640241df15655e0718cb68c50f2e5e192a03b8f96fd4ce4f8a2/detection) | [view](scans/0.3.1-Setup.exe.jpg) |
| 0.3.1 | `AxentraPDF-0.3.1.msi` | **0 / 54** detections | 2026-10-13 | [VirusTotal](https://www.virustotal.com/gui/file/f3f82c1a84bf2f0ab1bd2c896e848365462d69e6480036a4190b4f255f697b4f/detection) | [view](scans/0.3.1-msi.jpg) |
| 0.3.1 | `AxentraPDF-0.3.1-source.zip` | **0 / 65** detections | 2026-10-13 | [VirusTotal](https://www.virustotal.com/gui/file/04123696bd1f97bb8b8dcdb65ea91958d265e0ce6247a2409c65381b493400a5/detection) | [view](scans/0.3.1-source.zip.jpg) |
| 0.3.0 | `AxentraPDF-0.3.0-Setup.exe` | **0 / 68** detections | 2026-10-13 | [VirusTotal](https://www.virustotal.com/gui/file/b3b0e29c08e7a71d5b4e8ea8446ae4590e988f0f1f693a25c6618e358c6fe739/detection) | [view](scans/0.3.0-Setup.exe.jpg) |
| 0.3.0 | `AxentraPDF-0.3.0.msi` | **0 / 60** detections | 2026-10-13 | [VirusTotal](https://www.virustotal.com/gui/file/704ace20e151879e581875a34c3b619d33ab089b069c8c705b51c97133b9cf6a/detection) | [view](scans/0.3.0-msi.jpg) |
| 0.3.0 | `AxentraPDF-0.3.0-source.zip` | **0 / 66** detections | 2026-10-13 | [VirusTotal](https://www.virustotal.com/gui/file/ffe620c591d30c8f7960152a99238a1d4419854eddcd54a04d30cb3e961610a6/detection) | [view](scans/0.3.0-source.zip.jpg) |

The 0.3.0 download links on the release page were also checked as URLs: 0 / 93 vendors flag them.

SHA-256 of each file:

- `AxentraPDF-0.3.1-Setup.exe`: `50244e575db94640241df15655e0718cb68c50f2e5e192a03b8f96fd4ce4f8a2`
- `AxentraPDF-0.3.1.msi`: `f3f82c1a84bf2f0ab1bd2c896e848365462d69e6480036a4190b4f255f697b4f`
- `AxentraPDF-0.3.1-source.zip`: `04123696bd1f97bb8b8dcdb65ea91958d265e0ce6247a2409c65381b493400a5`
- `AxentraPDF-0.3.0-Setup.exe`: `b3b0e29c08e7a71d5b4e8ea8446ae4590e988f0f1f693a25c6618e358c6fe739`
- `AxentraPDF-0.3.0.msi`: `704ace20e151879e581875a34c3b619d33ab089b069c8c705b51c97133b9cf6a`
- `AxentraPDF-0.3.0-source.zip`: `ffe620c591d30c8f7960152a99238a1d4419854eddcd54a04d30cb3e961610a6`

Record each result as *detections / engines* (for example `0 / 72`) with the scan date. Screenshots of the
reports go in [`scans/`](scans/), named `<version>-<file>.jpg`.

## About false positives

Axentra is a Python application packaged with PyInstaller. A few antivirus engines flag PyInstaller apps
generically (names like `Trojan.Generic`, `Wacatac`, `ML.Attribute`), especially when they are unsigned and
new. If a release shows a low number of such detections, it is reported to the vendor as a false positive and
noted here.

## Code signing

The 0.3.0 installers are **not code-signed**. Windows SmartScreen therefore warns on first run, and PCs with
Smart App Control turned on block them. Code signing (an OV/EV certificate or Microsoft Trusted Signing) is
planned; this section will name the signer once releases are signed.
