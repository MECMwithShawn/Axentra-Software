# Security scans: Axentra Horizon

Every release package is scanned on [VirusTotal](https://www.virustotal.com) (70+ antivirus engines) before it
is published. Each link opens the report for that exact file (matched by its SHA-256), so you can check the
file you downloaded is the one that was scanned.

| Version | File | Result | Scanned | Report |
|---|---|---|---|---|
| 0.4.2 | `AxentraHorizon-0.4.2.zip` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/2300f331094aa56d1f814fd7aab609e501458047cd2d5a48073ca4dc04f6951b) |

SHA-256 of each file:

- `AxentraHorizon-0.4.2.zip`: `2300f331094aa56d1f814fd7aab609e501458047cd2d5a48073ca4dc04f6951b`

Record each result as *detections / engines* (for example `0 / 66`) with the scan date. Screenshots of the
reports go in [`scans/`](scans/), named `<version>-<file>.png`.

## What's in the package

The zip contains only plain, unminified JavaScript, HTML and CSS, the JSON manifest, four PNG and two SVG icons
(static images, no scripts) and a README. There is no
native code, no bundled third-party library, no remote code and no `eval`; the extension's content security
policy is the Manifest V3 default, which blocks remote scripts. You can read every file before loading it.

## Code signing

Browser extensions aren't Authenticode-signed. When Horizon is published on the Microsoft Edge Add-ons store,
the store signs the package and this section will link to the listing.
