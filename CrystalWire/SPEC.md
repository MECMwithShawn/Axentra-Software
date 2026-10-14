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
| Not yet: tray icon + toasts, CSV export, idle shading, incognito, per-connection ports, publisher/signer grouping, and everything in M2 (firewall) and most of M3 | Open |

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
