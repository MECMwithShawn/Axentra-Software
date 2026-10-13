# Security scans: Axentra Horizon

Every release package is scanned on [VirusTotal](https://www.virustotal.com) (70+ antivirus engines) before it
is published. Each link opens the report for that exact file (matched by its SHA-256), so you can check the
file you downloaded is the one that was scanned.

| Version | File | Result | Scanned | Report |
|---|---|---|---|---|
| 0.3.6 | `AxentraHorizon-0.3.6.zip` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/0ea22039b65b10125a037ef163d9d17373e5901cf38edb3be64410371b51a4e2) |

SHA-256 of each file:

- `AxentraHorizon-0.3.6.zip`: `0ea22039b65b10125a037ef163d9d17373e5901cf38edb3be64410371b51a4e2`

Record each result as *detections / engines* (for example `0 / 66`) with the scan date. Screenshots of the
reports go in [`scans/`](scans/), named `<version>-<file>.png`.

## What's in the package

The zip contains only plain, unminified JavaScript, HTML, CSS, three PNG icons and a README. There is no
native code, no bundled third-party library, no remote code and no `eval`; the extension's content security
policy is the Manifest V3 default, which blocks remote scripts. You can read every file before loading it.

## Code signing

Browser extensions aren't Authenticode-signed. When Horizon is published on the Microsoft Edge Add-ons store,
the store signs the package and this section will link to the listing.
