# Security: Axentra Vault

## Security review

Before its first release, Axentra Vault is getting an independent review of its encryption, its passkey handling,
how it keeps secrets out of memory and off the clipboard, how it saves files, and its optional site-icon
downloads. Problems found in the review are fixed and retested before release. This page will summarize the outcome
when the first version is published.

## Virus scans

Every release file will be scanned on [VirusTotal](https://www.virustotal.com) (70+ antivirus engines) before it is
published. Each link will open the report for that exact file, matched by its SHA-256.

| Version | File | Result | Scanned | Report | Screenshot |
|---|---|---|---|---|---|
| | *No releases yet* | | | | |

Record each result as *detections / engines* (for example `0 / 72`), with the scan date. Screenshots of the reports
go in [`scans/`](scans/), named `<version>-<file>.jpg`.

## Code signing

Not signed yet. Code signing is planned; this section will show the publisher name once releases are signed.

## Prepared 0.1.0 (unpublished)

- [AxentraVault-0.1.0-Setup.exe](https://www.virustotal.com/gui/file/82f7c7d3d5a8d1e14b1f13e0da81e7355a13f82a2b1eebc52239cb6aebfd4087/detection): scan pending; this is a hash lookup, not a recorded result.
- [AxentraVault-0.1.0.msi](https://www.virustotal.com/gui/file/d66dd9873e356a86b960e7d2973ee9bf0d2f433d49630f71e450781e5d95966c/detection): scan pending; this is a hash lookup, not a recorded result.
- [AxentraVault-0.1.0-source.zip](https://www.virustotal.com/gui/file/0f45879aa662543d926f168f50add9d81f3130863f6108c368b243d30a512c55/detection): scan pending; this is a hash lookup, not a recorded result.

See [release preparation](../releases/0.1.0.md) for build validation and pending signing/review work.
