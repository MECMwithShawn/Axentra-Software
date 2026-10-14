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
