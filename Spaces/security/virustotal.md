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

## Prepared 0.1.2 (unpublished)

This version is prepared locally and is awaiting approval to upload a GitHub Release draft.

- [AxentraSpaces-0.1.2-Setup.exe](https://www.virustotal.com/gui/file/9293f0f4fb6abd5ad69fe870b0c918da9e2e3f9ab8f062be00e719183475b0f9/detection): scan pending; this is a hash lookup, not a recorded result.
- [AxentraSpaces-0.1.2-portable.exe](https://www.virustotal.com/gui/file/a6a0a3e1892ae4d7cf824e8ff793757424f4c8fb1a57d5d4511f345aaa89a217/detection): scan pending; this is a hash lookup, not a recorded result.

See [release preparation](../releases/0.1.2.md) for build validation and pending signing/review work.
