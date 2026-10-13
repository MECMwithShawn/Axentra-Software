# Security scans: Axentra Spaces

Every release file is scanned on [VirusTotal](https://www.virustotal.com) (70+ antivirus engines) before it is
published. Each link opens the report for that exact file (matched by its SHA-256), so you can check that the file
you downloaded is the one that was scanned.

| Version | File | Result | Scanned | Report | Screenshot |
|---|---|---|---|---|---|
| 0.1.0 | `AxentraSpaces-0.1.0-Setup.exe` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/6704fb673134f2d8dd53ab08ff207cf699fdbb11e7e2cef7ef99deb6789d6d35/detection) | |
| 0.1.0 | `AxentraSpaces-0.1.0-portable.exe` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/10b9ed3c7d02b46133c599e23474b2a355b6ceffd1f19ef035d8a6f01ddc7745/detection) | |

SHA-256 of each file:

- `AxentraSpaces-0.1.0-Setup.exe`: `6704fb673134f2d8dd53ab08ff207cf699fdbb11e7e2cef7ef99deb6789d6d35`
- `AxentraSpaces-0.1.0-portable.exe`: `10b9ed3c7d02b46133c599e23474b2a355b6ceffd1f19ef035d8a6f01ddc7745`

Record each result as *detections / engines* (for example `0 / 72`) along with the scan date. Screenshots of the
reports go in [`scans/`](scans/), named `<version>-<file>.jpg`.

## About false positives

Spaces is a .NET 8 app published as a single self-extracting file. The installer is made with Inno Setup. It sets
a global hotkey, registers a link handler and can start with Windows, all for the current user only. A few
antivirus engines flag new, unsigned apps like this generically, with names such as `ML.Attribute` or
`Wacatac`. If a release shows a small number of such detections, it is reported to the vendor as a false
positive and noted here.

## Code signing

Not signed yet. Code signing is planned; this section will show the publisher name once releases are signed.
