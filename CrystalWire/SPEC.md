# Axentra CrystalWire — Functional Spec (clean-room)

**Provenance.** This spec was written only from GlassWire's public marketing
pages and online user guide (glasswire.com, glasswire.com/features,
glasswire.com/userguide), read 2026-10-13. No GlassWire binaries were installed,
decompiled, or inspected. It describes *what* the product should do, in our own
words; the *how* uses documented Windows APIs. Keep it that way: don't add
anything learned from inspecting GlassWire's code.

Legend: **M1** = monitor milestone (no blocking), **M2** = firewall,
**M3** = security alerts, **M4** = extras. ⭐ = goes beyond the reference product.

## Build status — 0.2.0 (2026-10-13)

| Piece | State |
|---|---|
| `cw-svc` service (LocalSystem): ETW net + DNS + process capture → SQLite (schema v3) | Done. Idempotent `install`, `update`, `uninstall`; migrates the old "Axentra Firewall" install |
| Per-app traffic 1 s / 1 min / 1 h, hosts with forward-DNS, DNS-cache seeding and reverse-DNS names | Done (1.1, 1.2, 1.3, 1.5, 1.8, 1.16) |
| Owner + country for every internet host (1.6) | Done, using iptoasn.com `ip2asn-combined.tsv.gz` (public domain, PDDL) instead of DB-IP: one file gives ASN, owner and country. Refreshed every 30 days by the service with Windows' curl.exe |
| DNS lookups by app with outcome (ok / blocked 0.0.0.0 / NXDOMAIN / failed), 1 min (7 d) + 1 h (forever) | Done (feeds 5b.3) |
| Pi-hole link (5b.3): app password in the user's Credential Manager, v6 API query log for this PC joined to our per-app DNS records; "never reached Pi-hole" list | Done (CLI `cw pihole`, window Pi-hole tab) |
| First-network-activity alerts (3.1) with a 10-minute baseline | Done (recorded; shown in the window, no toast yet) |
| `CrystalWire.exe` window: live graph with 5 min–month ranges and click-to-drill, usage by app/server/country/owner, DNS, Pi-hole, alerts | Done |
| `cw.exe` CLI: status, top, app, dns, alerts, live, geo, pihole | Done |
| Inno Setup installer (per-machine, stops/reconfigures the service on upgrade, optional history removal on uninstall) | Done |
| 0.3: tray icon (live rates, close-to-tray, start with Windows), notifications, version + About, GlassWire-style layout, app icons, svchost service names | Done |
| 0.4 file reputation (5.1, 3.2): SHA-256 per app exe; signature via Get-AuthenticodeSignature (embedded + catalog); Microsoft Defender scan (MpCmdRun -ScanType 3 -DisableRemediation); opt-in VirusTotal v3 hash lookup (4/min, 480/day cap) and MalwareBazaar get_info; keys in Credential Manager; excluded folders (default %USERPROFILE%\code) never looked up; "program changed" alert when an app's hash changes; runs in the user's tray app, results in %LOCALAPPDATA%\Axentra\CrystalWire\scans.db | Done |
| 0.4 network identity (2.6, 5b.2): default-route adapter, gateway MAC as identity, Windows profile name, Pi-hole auto-detected on the network's DNS servers (unauthenticated /api/auth fingerprint), captive portal via msftconnecttest.com (redirect address kept); auto role Home / Work / Other with per-network override; joined/left/sign-in events, new-network and sign-in-needed alerts, Sign in button opens the portal | Done |
| 0.5 network profile manager (in the service): roles Home / Work / Public, unknown = Public; Pi-hole only *suggests* Home; device discovery needs explicit consent on a Home network and is gated by a Permit only the service can issue; Work/Public never discover, enumerate or probe (even the Pi-hole check only targets the user's known Pi-hole address); window -> service commands over a local named pipe (SYSTEM/Admins/interactive user only, remote clients rejected) | Done |
| 0.5 home discovery: neighbor cache, ARP sweep of the local /24 (32 concurrent), mDNS/DNS-SD (3 rounds incl. TXT/SRV), SSDP + UPnP descriptions, NetBIOS, reverse DNS via the router, Pi-hole's device list; classification incl. HomeKit category, Apple model codes, Google Cast, thermostat, air purifier; new-device alerts after a baseline; user names/types/trust per device | Done |
| 0.5 Network Map tab: Visio-style SVG (Internet -> router -> category containers, inferred vs known links), drag/zoom/pan, saved layout, details panel, list view, SVG export | Done |
| 0.5 Work/Public: isolate view, LAN-contact alerts (other devices exchanging traffic with this PC), one-click "make Windows treat it as Public" via UAC; CrystalWire-enforced inbound blocking arrives with M2 | Done (monitoring) |
| Not yet: CSV export, idle shading, incognito, per-connection ports, publisher/signer grouping, and everything in M2 (firewall) and most of M3 | Open |

---

## 1. Traffic monitoring

| # | Feature | Milestone | Implementation |
|---|---|---|---|
| 1.1 | Live graph of upload/download over time, with a scale label (Kb/s, Mb/s) | M1 | ETW `Microsoft-Windows-Kernel-Network` (TCP/UDP send/recv, PID, bytes), aggregated per second |
| 1.2 | Timeline ranges: 5 min, 3 h, 24 h, week, month; sliders to zoom to a period | M1 | SQLite rollups: 1 s (24 h kept), 1 min (30 d), 1 h (forever) |
| 1.3 | Click a point in time and see which apps and hosts made the traffic, sorted by bytes | M1 | Per-(app, remote IP) buckets in the rollups |
| 1.4 | Group by: all / app / traffic type (protocol or port) / publisher (signer) | M1 | Port→service map; Authenticode signer via `WinVerifyTrust`/`CryptQueryObject` |
| 1.5 | Remote IPs resolved to hostnames | M1 | ETW `Microsoft-Windows-DNS-Client` (query→answers per PID), with reverse DNS as a fallback |
| 1.6 | Country flag per host, "unknown" shown when there's no data | M1 | Offline DB-IP Lite country DB (CC-BY, attribution in About) |
| 1.7 | Usage table: day / week / month, by app, host, traffic type, country; CSV export | M1 | SQL over the rollups |
| 1.8 | Local vs. external traffic filter | M1 | RFC1918, link-local, loopback and ULA classification |
| 1.9 | Idle periods shaded on the graph | M1 | `GetLastInputInfo` sampled by the UI agent |
| 1.10 | Pause graph; snapshot to PNG | M1 | UI only (no social sharing) |
| 1.11 | Mini always-on-top graph | M4 | Small borderless WebView2 window |
| 1.12 | Live world map with connection lines | M4 | Country centroids + SVG map |
| 1.13 | Per-process CPU / RAM / disk / GPU view | M4 (maybe skip) | PDH counters; low priority, off-mission |
| 1.14 | Incognito: stop recording globally or per app | M1 | Recorder drops events for flagged apps |
| 1.15 | Data plan: monthly cap with warning thresholds | M3 | Rollup sum vs. configured cap, billing-day aware |
| 1.16 | ⭐ Full history with no paywall or retention limit, queryable | M1 | SQLite; documented schema |
| 1.17 | ⭐ Per-connection detail: local/remote port, start/end time, bytes | M1 | ETW connect/disconnect + `GetExtendedTcpTable` snapshot at start |

## 2. Firewall

| # | Feature | Milestone | Implementation |
|---|---|---|---|
| 2.1 | Separate inbound/outbound allow/block per app | M2 | WFP user-mode: own provider + sublayer, persistent filters on `ALE_AUTH_CONNECT_V4/V6` (out) and `ALE_AUTH_RECV_ACCEPT_V4/V6` (in), matched on `FWPM_CONDITION_ALE_APP_ID` |
| 2.2 | Mode "click to block": allow by default | M2 | Only block filters |
| 2.3 | Mode "ask to connect": new apps are blocked until the user decides | M2 | Default-block filter at low weight + per-app allow filters; `FWPM_NET_EVENT` drop events → prompt → add rule. (A true pend-and-release needs a callout driver: future option.) |
| 2.4 | Mode "block all" / lockdown; per-app rules paused | M2 | One high-weight block filter, with an exception for our own service |
| 2.5 | Firewall on/off switch; off = everything unblocked, on = rules reapplied | M2 | Enable/disable our sublayer's filters |
| 2.6 | Profiles (home, public WiFi, metered) | M2 | Named rule sets; ⭐ auto-switch on network change (NLM `INetworkListManager` events) |
| 2.7 | Apps list with columns: app, hosts, activity sparkline, in, out; filter by allowed/blocked | M2 | UI |
| 2.8 | Bulk select → allow/block/remove, previewed and applied with "Apply" | M2 | Transactional `FwpmTransactionBegin/Commit` |
| 2.9 | Our own components always allowed and locked | M2 | Hard-coded permit filters |
| 2.10 | Auto-cleanup: drop rules for uninstalled apps (on by default), or for apps idle N days (off, 30 d) | M2 | Path existence + last-seen check; logged |
| 2.11 | ⭐ Doesn't clash with Windows Firewall rules (the reference product edits Windows Firewall directly and asks users to reset it) | M2 | Own WFP sublayer; leave Windows Firewall untouched |
| 2.12 | ⭐ Host/domain rules ("block telemetry.example.com for all apps"), blocklist import | M3 | DNS-ETW resolves domain→IPs; dynamic remote-address filters with TTL |
| 2.13 | ⭐ Rule bound to the binary's hash/signer, so a changed exe doesn't inherit trust | M3 | Store SHA-256 + signer with each rule; mismatch → re-prompt |
| 2.14 | ⭐ Temporary rules ("allow for 1 hour", "until reboot") | M2 | WFP dynamic session or expiry timer |

## 3. Security alerts

| # | Alert | Milestone | Detection |
|---|---|---|---|
| 3.1 | First network activity by an app | M1 | First-seen app path in DB |
| 3.2 | App info changed (version, hash, signer) for an app that uses the network | M3 | Hash/version on each first-of-day sighting |
| 3.3 | Network-related system files changed (hosts file, etc.) | M3 | `ReadDirectoryChangesW` on a watch list |
| 3.4 | DNS server settings changed | M3 | `NotifyIpInterfaceChange` + adapter DNS snapshot diff |
| 3.5 | Proxy settings changed | M3 | WinINet/WinHTTP proxy registry watch |
| 3.6 | Internet connectivity lost/restored | M3 | NLM connectivity events |
| 3.7 | ARP spoofing (gateway MAC changes or duplicate IP) | M3 | `GetIpNetTable2` polling + `NotifyIpInterfaceChange` |
| 3.8 | New/removed devices on the LAN | M3 | Network scanner (section 4) diff |
| 3.9 | WiFi evil twin: same SSID from unknown BSSID; known network suddenly open | M3 | `WlanGetNetworkBssList` |
| 3.10 | Inbound RDP session | M3 | WTS session notifications / TermService event log |
| 3.11 | Contact with known-malicious hosts | M3 | ⭐ User-selectable open feeds (e.g. abuse.ch, Spamhaus DROP), updated locally |
| 3.12 | Bandwidth threshold / data-plan alerts | M3 | See 1.15 |
| 3.13 | "While you were away" summary after idle | M3 | Idle-span digest |
| 3.14 | Changes to our own settings | M3 | Audit log |
| 3.15 | Alert UX: toast that fades, 24 h snooze, alert log with unread badge, sort by date/app/type, mark all read, dots on timeline | M1 (log) / M3 (all) | Windows toast notifications |
| 3.16 | Optional: write alerts to the Windows Event Log | M3 | Own event source |
| 3.17 | ⭐ Persistence watch: new Run keys / services / scheduled tasks for apps that then hit the network | M4 | Registry/WMI watch |

## 4. Network scanner

| # | Feature | Milestone | Implementation |
|---|---|---|---|
| 4.1 | Devices on the LAN: IP, MAC, vendor, name | M3 | ARP sweep of local subnet + `GetIpNetTable2`; OUI vendor DB (IEEE public list); mDNS/NetBIOS/SSDP names (reuse LAN Explorer ideas) |
| 4.2 | Alert when an unknown device joins | M3 | Diff against known list; "trust" button |

## 5. Optional integrations

| # | Feature | Milestone | Notes |
|---|---|---|---|
| 5.1 | VirusTotal lookup per app, manual or automatic | M4 | Off by default; ⭐ **hash lookup only by default**, file upload needs explicit per-file consent; user's own API key |
| 5.2 | Remote monitoring of another PC running our service | M4 | ⭐ Authenticated TLS (pinned key), not a plain password on port 7010; or reuse SSH tunnels |

## 5b. Ad & tracker blocking (AdLock-style, without HTTPS interception)

Sources: adlock.com public pages, read 2026-10-13, plus general knowledge of
system-wide ad blockers. AdLock-class tools filter HTTPS by installing a local
root CA and decrypting traffic. **We won't do that:** a local CA breaks
cert-pinned apps and banking, weakens TLS for every app, and turns our service
into the most valuable target on the machine (see Superfish/PrivDog). Cosmetic
in-page blocking is left to a browser extension (uBlock Origin, or a future
Axentra Horizon module).

| # | Feature | Milestone | Implementation |
|---|---|---|---|
| 5b.1 | System-wide domain blocking for every app, using the same lists Pi-hole uses (adlists URLs, hosts format, AdGuard/ABP `||domain^` syntax) | M3 | Local DNS stub on 127.0.0.1:53, set as the adapter DNS while active; answers 0.0.0.0/NXDOMAIN for blocked names, forwards the rest over DoH/DoT ⭐ |
| 5b.2 | Pi-hole aware ⭐: on the home network, defer to Pi-hole (no double blocking); off-network, use the local blocklist | M3 | Detect Pi-hole by configured IP/API reachability + NLM network identity; profile auto-switch (2.6) |
| 5b.3 | Per-app attribution of blocks ⭐: "Pi-hole blocked 412 queries today; 300 came from `SomeApp.exe`". Pi-hole only sees the PC's IP, we see the process | M3 | DNS-client ETW (PID + query) joined with the Pi-hole API query log (`/api/queries`, v6 API) or our own stub's log |
| 5b.4 | Allow ads per app or per site (exceptions) | M3 | Per-app bypass in the stub (query PID via ETW correlation) + domain allowlist |
| 5b.5 | Bypass protection ⭐: apps that hard-code DNS (8.8.8.8:53) or use their own DoH | M3 | WFP: block outbound UDP/TCP 53 and 853 except to our stub/Pi-hole; optional block of known public-DoH endpoint IPs |
| 5b.6 | Malicious / phishing domain lists (separate from ad lists) | M3 | Same engine, separate list category and alert type (3.11) |
| 5b.7 | Encrypted-SNI-free hostname blocking for apps that skip DNS (hard-coded IPs) | M4 | WFP remote-IP filters from resolved list entries; ALE layer only, no payload inspection |
| 5b.8 | Stats: blocked queries over time, top blocked domains, top apps | M3 | Same graph/table UI as section 1 |
| 5b.9 | List auto-update on a schedule, with a diff view | M3 | Scheduled fetch; user-chosen lists only |

**Not doing:** TLS MITM / local root CA, in-page cosmetic filtering, and in-app
YouTube-style ad stripping. These need decryption or browser code.

### Home Pi-hole specifics (from MECMwithShawn/pi-hole, read 2026-10-13)

- Pi-hole v6 in Docker at `192.168.50.100` (DNS on :53, web/API on http :80,
  `/api`). Upstream is the ASUS router `192.168.50.1` → Quad9 over DoT (Safe mode).
- Lists: StevenBlack hosts + HaGeZi Multi PRO (Adblock syntax), plus an exact deny list
  (LG TV). **The local off-network blocker must parse both hosts and `||domain^`
  formats and should mirror Pi-hole's lists:** pull adlists (`GET /api/lists`) and
  deny/allow domains (`GET /api/domains`) so blocking is identical away from home.
- Off-network upstream should match home: Quad9 (`dns.quad9.net`) over DoH/DoT,
  so malware blocking also follows the laptop.
- API auth: use a Pi-hole v6 **app password** (Settings → Web interface/API),
  stored with Windows DPAPI / Credential Manager, never in a config file. The read
  scope (query log, stats, lists) is all we need.
- "Home network" = gateway `192.168.50.1` with the router's MAC + SSID `ShawnNet-6G`
  (NLM network identity), not the SSID alone (an evil twin can copy an SSID).
- DHCP gives two DNS servers (Pi-hole, then router fallback). Windows can send
  lookups to the fallback while Pi-hole is up, which bypasses blocking. ⭐ Alert
  "DNS fell back to the router" with the share of lookups and which apps, from
  DNS-client ETW (which server answered).
- ⭐ IPv6 leak check: alert if any active adapter has an IPv6 DNS server that
  isn't Pi-hole (the README's IPv6 caveat). Windows' `fec0:0:0:ffff::1-3`
  placeholders don't count.
- Hyper-V `vEthernet (External NAT)` uses the router's DNS directly, so VMs and
  containers behind it skip Pi-hole. Show this per adapter, not as an error.

### Observed on this PC (black-box, 2026-10-13)

AdLock's kernel driver `adlocknetdrv.sys` intercepts **UDP** port 53 to *any*
server and answers NXDOMAIN for names on its list. TCP DNS passes through. So
domains AdLock blocks never reach Pi-hole, and the Pi-hole dashboard undercounts
this PC. Consequences for us:
- Detect other DNS interceptors at startup (a query for a known-blocked
  test name over UDP vs TCP gives different answers) and warn; two blockers
  hide each other's stats.
- Our own design avoids a driver: a local stub plus WFP blocking of :53/:853 to
  other servers (5b.5) gets the same coverage.

## 6. Settings and admin

- Run on startup (UI agent; the service always runs). Tray icon. Bytes vs. bits units. 12/24 h time.
- Optional admin password / UAC prompt before changing rules or settings.
- Clear history. Move DB location (setting, not a hand-edited conf file).
- Export/import rules and settings (JSON) ⭐. Backup/restore built in ⭐.
- Clean uninstall removes all WFP filters (provider-keyed, so nothing is left behind).

## 7. Deliberately excluded

- Accounts, cloud management console, telemetry, ads, social sharing.
- Paid tiers gating ask-to-connect, block-all, history length or themes. All features ship in one build.

---

## Architecture (summary)

```
┌──────────────────────────┐   named pipe (ACL: admins + interactive user, read-only for non-admin)
│ axentra-crystalwire-svc     │◄───────────────────────────────┐
│ Rust, LocalSystem service│                                │
│  • ETW consumer (net+DNS)│                       ┌────────┴─────────┐
│  • WFP rule engine       │                       │ Axentra CrystalWire │
│  • alert detectors       │                       │ .NET 8 + WebView2│
│  • SQLite (rollups, log) │                       │ tray, toasts, UI │
└──────────────────────────┘                       └──────────────────┘
```

## Milestone M1 — monitor only (first build)

Done when:
1. The service captures per-PID TCP/UDP bytes via ETW, and totals agree within 5% of
   `Get-NetAdapterStatistics` deltas over 10 minutes of mixed traffic.
2. PID→exe path is resolved even for short-lived processes (capture at
   process-start ETW, not lazily).
3. IP→hostname comes from DNS-client ETW for ≥90% of browser connections.
4. 1 s / 1 min / 1 h rollups are in SQLite; 24 h of data is < 50 MB.
5. The UI shows a live graph, a click-to-drill app/host list, a usage table + CSV,
   first-network-activity alerts, and incognito.
6. Nothing in M1 creates WFP filters or touches Windows Firewall.

### M1 probe results (`cw-probe`, 2026-10-13, this PC, Wi-Fi)

| Criterion | Result | Status |
|---|---|---|
| 1. ETW bytes vs adapter counters, within 5% | 4 × 25 MB download: 95.6% payload only, **99.3%** with header estimate; curl's own byte count vs ETW for curl.exe: **100.2%** (the extra is TLS overhead) | ✅ |
| 2. Short-lived PIDs resolved | 0 unresolved apps (process-start ETW + snapshot fallback) | ✅ |
| 3. ≥90% of internet bytes get a hostname | **99.9%** with DNS-cache seeding at startup (57% without it) | ✅ (recheck under browser-heavy load) |
| parse errors | 0 of 70,141 events | ✅ |

Lessons for the service:
- Seed IP→name from the DNS client cache at startup (WMI `MSFT_DNSClientCache`).
  Long-lived connections otherwise never get names.
- Record process paths from Kernel-Process start events; lazy `OpenProcess` misses
  short-lived and protected processes.
- Mixed small traffic reads ~75% of adapter octets on payload alone (headers + bare
  ACKs). Show payload in the UI, but don't present it as "wire bytes".
- Browsers with their own secure DNS (Edge/Chrome DoH) bypass the Windows DNS client,
  so their IPs can stay unnamed (e.g. msedge → 23.220.103.8). Fallbacks: reverse DNS,
  TLS SNI isn't visible without inspection, and the 5b.5 DoH blocking pushes browsers
  back to system DNS.

## Known risks

- **Signing:** Smart App Control and Defender will block or flag an unsigned
  service that edits firewall rules (same issue as Axentra Vault). Budget a
  code-signing cert before M2 ships to anyone.
- **Lock-out:** a bad default-block rule can cut the PC off the network. M2 testing
  goes in Windows Sandbox/VM first, and the service gets a "failsafe: remove all
  our filters" command plus an uninstall path that works offline.
- **ETW attribution gaps:** traffic from svchost-hosted services shows up under
  svchost. Map to the service name via `QueryServiceStatusEx` / the service tag.


## Network profiles (0.5)

| Capability | Home (trusted, consented) | Work | Public / unknown |
|---|---|---|---|
| Device discovery, inventory, map | Yes | Never | Never |
| Subnet scan / ARP sweep / mDNS / SSDP / NetBIOS | Yes, local /24 only, ~every 15 min | Never | Never |
| Pi-hole check | Any of the network's DNS servers | Only the user's known Pi-hole address | Only the user's known Pi-hole address |
| Captive-portal check (msftconnecttest.com) | Yes | Yes | Yes |
| LAN-contact alerts | No (expected) | Yes | Yes |
| Inbound protection | Windows Firewall Private; CrystalWire rules in M2 | Strict: Windows Public profile now (one click), CrystalWire default-deny inbound in M2 | Same as Work |
| Organization-managed (DomainAuthenticated) policy | n/a | Never weakened | n/a |

| Router access (user's own login) | Yes, only the router connected on this network (gateway IP + MAC + pinned key) | Never | Never |
| Config health checks | Yes | No | No |

Topology honesty: a Windows endpoint can't see switches, mesh nodes or Wi-Fi associations, so device links are drawn as *inferred* unless the router reports them. With the router connected (0.6), links it reports are drawn solid with wired / band / RSSI; everything else stays dashed.

## Router access (0.6)

- ASUS stock ASUSWRT web API (the interface ASUS's own apps use), HTTPS port 8443 only.
- Trust: the router's self-signed certificate is not trusted; the SHA-256 of its public key (SPKI) is shown to the user, confirmed once, and pinned (`curl --pinnedpubkey`). A different key stops all access until the user reconnects.
- Read-only allowlist: `login.cgi`, `appGet.cgi` with `get_clientlist()`, `get_clientlist_from_json_database()` and `nvram_get()` of a fixed list of non-secret keys (identity, firmware, exposure, DNS, Wi-Fi auth mode), `Logout.asp`. Unit tests assert no apply/set/scan hooks and no secret-looking keys.
- Lockout safety: one login attempt per user action; a failed login (bad password, lockout, CAPTCHA, reset-required, pin mismatch) blocks further attempts until the user acts. An expired session gets at most one re-login.
- Secrets: the login is a generic credential `AxentraCrystalWire/router` in the user's Credential Manager. Password, token and request bodies go to curl on stdin (`-K -`), never on a command line, and are never logged.
- Split: the window (user context) holds the login and polls (clients every 60 s, settings every 30 min); the service accepts the normalized client list only for the current permitted Home network whose gateway MAC matches.

## Config health (0.6)

Pure, read-only rules (`cw-core/src/health.rs`) over this PC's DNS servers and recent DNS answers, the router's settings and clients, and Pi-hole single settings (`/api/config/dns/upstreams`, `/api/config/dns/revServers`, `/api/dns/blocking`, `/api/stats/summary`, `/api/stats/top_clients`, `/api/info/version`; never `/api/teleporter` or the full config). Each finding has a level (problem / warning / tip / good), a source, and where to fix it in the router's or Pi-hole's own UI. CrystalWire never changes those settings.

## AI agents (0.7)

CrystalWire finds and verifies; an AI agent makes the change; the user approves it.

- **Task briefs** (`cw-core/src/agents.rs`): for one finding, a markdown task with YAML front matter (finding id, level, source), what's wrong, the fix, where to make it (router admin page / Pi-hole repo or UI / this PC), the evidence CrystalWire read, how to verify (`cw health --recheck` or the MCP `recheck` tool) and ground rules: propose first and wait for the go-ahead, one change at a time, the user types router passwords, no secrets in files or logs, never `/api/teleporter`, stop if anything differs.
- **Hand-off** (window, `cw-app/src/agents.rs`): writes the brief to %LOCALAPPDATA%\Axentra\CrystalWire\tasks\, then on the user's click copies it, opens it in Antigravity, or starts Codex (optionally `--oss` with a local Ollama model) or Claude Code in a new console when their command lines exist. Pi-hole tasks run in the user's Pi-hole config folder when one is set.
- **Read-only MCP server** (`cw mcp`, stdio JSON-RPC): get_findings, get_fix_task, recheck, get_dns_summary, get_devices, get_fix_history. No tool can change anything; recheck only signals the running window (`Local\AxentraCrystalWire.Recheck`), which re-checks through its normal, lockout-safe router poll.
- **State for agents**: the window saves each Home-network check to health.json (findings + context, no secrets) and keeps health-history.json (first seen, hand-offs, fixed; a finding counts as fixed only when its source was read). Background checks every 10 minutes.
- **Privacy**: whatever an agent reads (IPs, MACs, SSIDs, device names) goes to its AI provider; Codex with a local model keeps it on the PC. The UI says so.

## Axentra Ghost and Axentra Command (0.8)

- **Ghost's network list -> Pi-hole** (`cw-core/src/ghost.rs`, contract in axentra-ghost/docs/INTEGRATION-CRYSTALWIRE.md). Off until the user turns it on in Settings; the window then checks the Pi-hole login and hands the service its own copy (SYSTEM's Credential Manager, `AxentraCrystalWire/ghost-pihole`). The service reads `%ProgramData%\Axentra\Ghost\network-blocklist.json` only if `%ProgramData%\Axentra` and `...\Ghost` pass the owner/DACL check (owner SYSTEM/Administrators/TrustedInstaller; no write rights for anyone else except inherit-only CREATOR OWNER; no reparse points), caps it at 1 MB / 5000 domains and validates every hostname. It makes Pi-hole's exact deny list match: adds listed domains not present (comment "Axentra Ghost"), removes only entries whose comment is exactly that. The credential's user field is JSON `{url, network, owner}`: the normalized Pi-hole URL (`http://<private IPv4>[:port]`), the network key where it was turned on (`mac:<router MAC>`, Home only) and the SID of the Windows user who turned it on (from the pipe caller's token). The service uses a login only while its current network (published by the aggregator, cleared while a new network is being recorded, republished on a role change) has that key and role Home, and `netid::current()` confirms the key live just before (the aggregator's view lags a network change by the watcher's probes). Only that SID or an elevated caller can replace, disable or cancel it. Roles are machine-wide: any user marking that network Work/Public pauses the sync (fails closed). Checks every 30 s for a new list (3 s debounce) and compares with Pi-hole every 10 minutes; Pi-hole replies are capped at 16 MiB / 30 s. Reports in `%ProgramData%\AxentraCrystalWire\ghost-sync.json` (applied_at kept across restarts; rewritten when anything shown changes or a new list was applied; `error` only for failures, and an optional `note` for information while the sync is off: a removal still waiting, or the 0.8.0 note). Turning it off moves the login to a removal slot (`ghost-pihole-remove`, `-2`, `-3`; one per Pi-hole + network) at once; the loop removes Ghost's entries when this PC is on that network and only then deletes the slot (only if unchanged meanwhile). Turning it on for the same Pi-hole + network cancels that removal (the sync takes over); for another one, the removal keeps waiting, and a sync switched from one target to another parks the old login for removal. One slot always stays free so the sync can be turned off; "Stop waiting" (`ghost_sync {enabled:false, forget_pending:true}`) forgets removals. All login changes are serialized by one lock. Failed removals retry every 10 minutes (at once after a network change or command). With the sync off, a new list is acknowledged (list_updated set, pihole null); a sync deferred because this PC is elsewhere is retried each pass and on arrival. applied_at is stamped when an apply finished. A 0.8.0 login (plain URL user field) is dropped with a note if the 0.8.0 status showed Ghost entries. Tagged entries that aren't plain domains are left alone.
- **Live monitor**: Ghost reads crystalwire.db read-only; `meta('monitor_contract') = '1'` declares the tables it relies on.
- **Shared tray** (axentra-command/docs/TRAY-HOST.md): every 2 s the window checks for `Local\Axentra.TrayHost`; while it exists the window hides its own tray icon. The hosted state is re-sent every 2 s while hosted (Explorer restarts re-add icons); unhiding redraws the badge. Command's menu runs `CrystalWire.exe --security | --check-network | --snooze | --resume | --exit` (listed in `axentra.app.json`), forwarded to the running window by named events (`Local\AxentraCrystalWire.Security / .CheckNetwork / .Snooze / .Resume / .Quit`; a second launch signals `.Show`, after AllowSetForegroundWindow). The watcher thread writes `%LOCALAPPDATA%\Axentra\CrystalWire\tray-status.json` (counts and a tooltip of at most 80 characters) when the counts change.
- **Command pipe** (`cw-core/src/ipc.rs`, 0.8.1): the service creates `\\.\pipe\AxentraCrystalWire` with FILE_FLAG_FIRST_PIPE_INSTANCE (retrying if the name is taken), owner SYSTEM, DACL SYSTEM/Administrators full and interactive users read + write data only (no create-instance right). Up to 16 overlapped instances; the next is listening before a connection is handled; each request must arrive within 5 s (256 KiB). The caller's SID and elevation come from its token (ImpersonateNamedPipeClient, identification level). The client opens with identification-only QoS, refuses a pipe whose owner isn't SYSTEM or Administrators, caps replies at 4 MiB, and (0.8.2) also requires the pipe's server process (GetNamedPipeServerProcessId) to be the service's own process (QueryServiceStatusEx). The client handle is overlapped and a call has 15 s in all. After writing a reply the server waits at most 5 s for the client to read and close (no FlushFileBuffers, which waits without limit), then disconnects and hands the instance back to the serving loop over a channel: instances are never closed while the service runs (so the name can't come free), the loop listens next on a handed-back instance, else a new one, and at the 16-instance limit waits for one to come back.

## Setup and console programs (0.8.2)

- Setup checks, stops and starts the service through the service API (OpenSCManager / QueryServiceStatus / ControlService / StartService, `installer/lib.iss`), not sc.exe. If the service won't stop within 60 s it ends `{app}\cw-svc.exe` only (by path).
- Before Windows' Restart Manager runs (CloseApplications), Setup ends `{app}\cw.exe` itself (by path; `cw mcp`, `cw live`). Restart Manager closes a console program by attaching to its console and leaves the installer attached; console programs the elevated installer starts afterwards inherit that console and fail with 0xC0000142. Every console program Setup still runs (`cw-svc install`) goes through `RunHidden`, which calls FreeConsole first and retries once on 0xC0000142. The uninstaller ends cw.exe and calls FreeConsole before `cw-svc uninstall`. Covered by installer/tests/lib-test.iss (it attaches to a test program's console and checks RunHidden lets go of it).

## App identity and attribution (0.8.2)

- **Connection owners** (`cw-core/src/owners.rs`): Kernel-Network send/receive events carry a PID that, for work done in the background (receive indications, ACK-driven sends), can be whichever process was running. The aggregator credits each packet to the owner Windows lists for the connection (GetExtendedTcpTable / GetExtendedUdpTable with owner PIDs). This PC's end is the source of a send; for a receive, the destination when it is one of this PC's addresses or a multicast/broadcast address, else the source. TCP is matched by local + remote endpoint; UDP by local endpoint, then the any-address binding of the family (dual-mode IPv6 sockets are listed in the IPv4 table too); packets to a multicast or broadcast address by every program bound to that port. On a UDP endpoint or port several programs share, the event's PID is used if it is one of them, else the lowest of them: never a program that holds no socket there. Ports are in network byte order (the manifest declares them win:Port). Tables are re-read at most every 250 ms, only for an endpoint they don't list at all and that wasn't looked for in vain in the last 2 s. When Windows doesn't list it (already closed), the event's PID is used. The service log reports the split once after 2000 packets, then hourly.
- **Identity** (`cw-core/src/appid.rs`, window and `cw` only; cached 10 minutes per path; a panic while working one out gives a lettered tile, never a failed list): `<pid N, exited>` is "Closed program (PID N)" with an unknown-program tile; the kernel's bare names (System, Registry, ...) get the Windows tile; a service suffix is read only on Windows' own System32/SysWOW64 svchost.exe. Network shares and removable drives aren't touched (lettered tile). Name = the exe's FileDescription (with the file name added unless the exe is a Windows file as above, an administrator-owned file in Program Files, or in a Microsoft Store package of Microsoft's own: "Claude Code (claude.exe)"), else its ProductName with the file name; svchost entries are named after their services (per-user `_xxxxx` suffix dropped). Icon = the exe's own; else a Windows tile for programs in %SystemRoot% (not its user-writable folders: Temp, Tasks, Tracing, spool, Crypto, ...) owned by TrustedInstaller, SYSTEM or Administrators; else the app it belongs to: a Store package folder this user has registered (HKCU AppModel\Repository, re-read every 5 minutes) or one under Program Files\WindowsApps, or its per-user data folder (%LOCALAPPDATA%\Packages\<Name>_<publisher>, newest main package, resource packs skipped), using its manifest's first Executable with an icon, else its Square44x44Logo/Logo PNG (manifest paths must be plain names inside the package; files size-checked before reading); a vendor and product folder directly inside %LOCALAPPDATA% or Program Files matching a two-part Store name (`%LOCALAPPDATA%\OpenAI\Codex\...` for `OpenAI.Codex`); or another exe with an icon at the top of the same installation folder (directly under Program Files, %LOCALAPPDATA%\Programs or %LOCALAPPDATA%; not shared folders such as Temp, Microsoft, Common Files, Packages; named like the folder first; not an uninstaller/updater; not one naming a different CompanyName). A borrowed identity is named "App (file.exe)" unless the exe has a name of its own. Otherwise console programs get a terminal tile and the rest a lettered tile (colour from the name). `/api/icon` serves PNG or the SVG tile.
- **Display names** (`query::app_names`): identity name or file name; names that collide get the file name, then the fewest parent folders (up to 3) that tell that group apart.
