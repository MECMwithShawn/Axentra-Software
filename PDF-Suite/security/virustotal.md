# Security scans: Axentra PDF Suite

Every release installer is scanned on [VirusTotal](https://www.virustotal.com) (70+ antivirus engines)
before it is published. Each link opens the report for that exact file (matched by its SHA-256), so you can
check the file you downloaded is the one that was scanned.

| Version | File | Result | Scanned | Report |
|---|---|---|---|---|
| 0.3.0 | `AxentraPDF-0.3.0-Setup.exe` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/2e818495d443281cce02960dc3586bfe06ceb09d11cc1d7df140f167fe91548d) |
| 0.3.0 | `AxentraPDF-0.3.0.msi` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/a2ae25cad6f398de7eb3afc5fff96c77abc53f2386b73562f4b958d5b2e64852) |

SHA-256 of each file:

- `AxentraPDF-0.3.0-Setup.exe`: `2e818495d443281cce02960dc3586bfe06ceb09d11cc1d7df140f167fe91548d`
- `AxentraPDF-0.3.0.msi`: `a2ae25cad6f398de7eb3afc5fff96c77abc53f2386b73562f4b958d5b2e64852`

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
