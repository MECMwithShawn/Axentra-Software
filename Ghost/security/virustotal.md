# Security scans: Axentra Ghost

Every release file is scanned on [VirusTotal](https://www.virustotal.com) (70+ antivirus engines) before it is
published. Each link opens the report for that exact file (matched by its SHA-256), so you can check that the file
you downloaded is the one that was scanned.

| Version | File | Result | Scanned | Report | Screenshot |
|---|---|---|---|---|---|
| 0.1.4 | `AxentraGhost-0.1.4-Setup.exe` | *pending* | | [VirusTotal](https://www.virustotal.com/gui/file/6060095b76d6541af214750672c35e8345bc23dd89a7d20fcdc80b451260a01f/detection) | |

SHA-256: `6060095b76d6541af214750672c35e8345bc23dd89a7d20fcdc80b451260a01f`

Record each result as *detections / engines* (for example `0 / 72`) along with the scan date. Screenshots of the
reports go in [`scans/`](scans/), named `<version>-<file>.jpg`.

## About false positives

Ghost is a privacy tool, so it does what some antivirus heuristics look for: it changes Windows policies, disables
telemetry services and scheduled tasks, adds firewall rules and can edit the hosts file. Microsoft Defender in
particular flags hosts-file entries for Microsoft domains as `SettingsModifier:Win32/HostsFileHijack`; that is
expected when you choose hosts-file blocking, and Ghost keeps its entries in a clearly marked section. If a release
shows generic detections, they are reported to the vendor as false positives and noted here.

## Code signing

Not signed yet. Code signing is planned; this section will show the publisher name once releases are signed.
