# Security scans: Axentra PDF Suite

Every release installer is scanned on [VirusTotal](https://www.virustotal.com) (70+ antivirus engines)
before it is published. Each link opens the report for that exact file (matched by its SHA-256), so you can
check the file you downloaded is the one that was scanned.

| Version | File | Result | Scanned | Report | Screenshot |
|---|---|---|---|---|---|
| 0.3.2 | `AxentraPDF-0.3.2-Setup.exe` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/a9b5e91a666dc847e08ee940aeca696a665bc024f38c771ac82c5bc1f88b46c7/detection) | |
| 0.3.2 | `AxentraPDF-0.3.2.msi` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/f13fd03a0fd8d4f691bf800ba715493f88fec8445c5f2c65c6a77ef58dcd4658/detection) | |
| 0.3.2 | `AxentraPDF-0.3.2-source.zip` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/e78bf6df5f007e182a2be55cad7b0453d8c19a90298f6e09151f81ce8d2aac23/detection) | |

SHA-256 of each file:

- `AxentraPDF-0.3.2-Setup.exe`: `a9b5e91a666dc847e08ee940aeca696a665bc024f38c771ac82c5bc1f88b46c7`
- `AxentraPDF-0.3.2.msi`: `f13fd03a0fd8d4f691bf800ba715493f88fec8445c5f2c65c6a77ef58dcd4658`
- `AxentraPDF-0.3.2-source.zip`: `e78bf6df5f007e182a2be55cad7b0453d8c19a90298f6e09151f81ce8d2aac23`

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
