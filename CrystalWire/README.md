# Axentra CrystalWire

See what every app on your Windows PC is doing on the network, and later decide which apps get to connect at all.
CrystalWire records how much each app sends and receives, which servers it talks to, and the first time each app
ever goes online. Everything stays on your PC. There's no account, subscription, cloud console, ads or telemetry.

**Status: in development, no download yet.** Full functional spec and test results: [SPEC.md](SPEC.md). Version 0.1 is a monitor only: it records traffic and doesn't block
anything or change any firewall rules.

## What's planned

| Area | What you get | Status |
|---|---|---|
| **Per-app traffic** | Bytes sent and received by each app, every second, with per-minute and per-hour history by server | Working (0.1, command line) |
| **Server names** | Remote addresses shown as names from the PC's own DNS lookups, with a reverse-DNS fallback | Working (0.1) |
| **New-app alerts** | A record of the first time each app reaches the network | Working (0.1, command line) |
| **Graphs and app list** | A live traffic graph you can click to see which apps and servers caused a spike, with a timeline | Planned |
| **Country and owner** | The country and network owner of every server, from an offline database (no lookups sent anywhere) | Planned |
| **Per-app firewall** | Allow or block each app separately for incoming and outgoing traffic, using the Windows Filtering Platform. It doesn't change your Windows Firewall rules | Planned |
| **Ask to connect** | New apps are held back until you allow them; temporary rules (*allow for 1 hour*) | Planned |
| **Domain blocking** | Block ad, tracker and malware domains for every app, using the same blocklists as Pi-hole, with no HTTPS interception and no root certificate | Planned |
| **Pi-hole integration** | Hands blocking to your Pi-hole at home and does it locally elsewhere, and shows *which app* made each blocked lookup | Planned |
| **Security alerts** | Changes to DNS servers, proxy settings or the hosts file, ARP spoofing, new devices on your network, a changed program that keeps its old permissions | Planned |

## How it works

- A background Windows service (`Axentra CrystalWire`) reads the network, DNS and process events that Windows
  already produces (Event Tracing for Windows). It doesn't use a packet-capture driver and doesn't read packet contents.
- History goes to a local SQLite database: per-second totals for 24 hours, per-minute by server for 30 days, and
  per-hour by server indefinitely.
- In tests, its byte counts matched curl's own count within 0.2% on a 100 MB download, and came within 1% of the
  network adapter's counters once packet headers were included.

## Privacy

CrystalWire has no servers, accounts or analytics, and Axentra never receives any data from it. The only network
requests it makes itself are reverse-DNS lookups for unnamed servers, through your normal DNS resolver. The history
database stays on your PC.

## System requirements (planned)

- Windows 10 or 11, 64-bit
- Administrator rights to install, because the service runs as SYSTEM to read network events

## Built with

Rust (service, capture and command-line tool), SQLite, and the Windows ETW and WFP APIs. The planned interface uses
.NET 8 with Microsoft Edge WebView2, like [Axentra Spaces](../Spaces/).
