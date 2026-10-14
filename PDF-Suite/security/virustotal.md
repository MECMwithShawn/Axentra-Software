# Security scans: Axentra PDF Suite

Scan results and pending checks for the current release are recorded below. Each
[VirusTotal](https://www.virustotal.com) link identifies the exact file by its SHA-256.
The engine count varies by report; a pending entry has no recorded scan result yet.

| Version | File | Result | Scanned | Report | Screenshot |
|---|---|---|---|---|---|
| 0.3.4 | `AxentraPDF-0.3.4-Setup.exe` | **1 / 68**: Deep Instinct only, see below | 2026-10-14 | [VirusTotal](https://www.virustotal.com/gui/file/afab0bccadf6df95df770976f7225da7f95f5c43ccdbb9f648b6d8a15f11c413/detection) | [view](scans/0.3.4-Setup.exe.jpg) |
| 0.3.4 | `AxentraPDF-0.3.4.msi` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/fdd13b18dd224c029ec19597402a19a69cea855e87399dd816a90731c2130e2d/detection) | |
| 0.3.4 | `AxentraPDF-0.3.4-source.zip` | **0 / 66** detections | 2026-10-14 | [VirusTotal](https://www.virustotal.com/gui/file/e60049b9331e1eb3c6f5850804f21abe12b196b16f9068fc0cee8ff6b200938b/detection) | [view](scans/0.3.4-source.zip.jpg) |

SHA-256 of each file:

- `AxentraPDF-0.3.4-Setup.exe`: `afab0bccadf6df95df770976f7225da7f95f5c43ccdbb9f648b6d8a15f11c413`
- `AxentraPDF-0.3.4.msi`: `fdd13b18dd224c029ec19597402a19a69cea855e87399dd816a90731c2130e2d`
- `AxentraPDF-0.3.4-source.zip`: `e60049b9331e1eb3c6f5850804f21abe12b196b16f9068fc0cee8ff6b200938b`

Only the current release is listed; older versions have been withdrawn.

Record each result as *detections / engines* (for example `0 / 72`) with the scan date. Screenshots of the
reports go in [`scans/`](scans/), named `<version>-<file>.jpg`.

**0.3.4 installer (.exe):** one engine, Deep Instinct (a machine-learning scanner), flags it; the other 67
are clean. The 0.3.1 installer scanned 0 / 68. Machine-learning engines often flag new, unsigned installers,
and this one will be submitted to Deep Instinct for review as a likely false positive. The source scanned
0 / 66.

## About false positives

Axentra is a Python application packaged with PyInstaller. A few antivirus engines flag PyInstaller apps
generically (names like `Trojan.Generic`, `Wacatac`, `ML.Attribute`), especially when they are unsigned and
new. If a release shows a low number of such detections, it is reported to the vendor as a false positive and
noted here.

## Code signing

The installers are **not code-signed**. Windows SmartScreen therefore warns on first run, and PCs with
Smart App Control turned on block them. Code signing (an OV/EV certificate or Microsoft Trusted Signing) is
planned; this section will name the signer once releases are signed.
