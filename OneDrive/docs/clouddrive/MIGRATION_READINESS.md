# Migration readiness: OneDrive → Proton Drive

> **No production measurement yet.** Every figure on this page is a placeholder. Nothing has been mirrored from the real OneDrive account. All results so far come from offline tests: the OneDrive mock and a fake `proton-drive`.
>
> Fill this in from `axentra-onedrive mirror status --json` after each approved step. Copy counts and percentages only, never file names or account details.

This is a third-party application not officially supported by Proton.

## 1. Scope and policy

| Item | Value |
|---|---|
| Direction | One way, OneDrive → Proton Drive. Non-destructive. |
| Destination root | `/my-files/OneDrive` (`[mirror] dest_root`) |
| New files | `-f skip`. A name that is already taken is adopted if the SHA-1 and size match; otherwise it is marked `diverged`. |
| Changed source content | Marked `diverged`. No new revision is uploaded. |
| Source deletes and moves | Recorded only, never propagated. |
| Staging | `~/.cache/axentra/onedrive/mirror-staging`, mode 0700, cap 2 GiB |
| Manifest | `~/.local/share/axentra/onedrive/mirror.sqlite`, `user_version=1` |
| Level counted as "migrated" | _to decide (L3 or L4), checkpoint §10.4_ |

## 2. Tooling

| Item | Value |
|---|---|
| Proton CLI | `proton-drive` 0.9.0 (`cli-drive@0.9.0+75612299`, SDK js@0.22.2) |
| Binary SHA-512 | pinned in `PINNED_SHA512` (crates/provider/src/proton_cli.rs) |
| Binary signature | **None published.** This is a known gap: only the SHA-512 is checked. |
| Axentra CloudDrive build | _version / commit_ |
| `proton status` | _date: ok / not signed in_ |

## 3. Checkpoints (AUDIT-AND-PLAN.md §10)

| # | Checkpoint | Status | Date | Notes |
|---|---|---|---|---|
| 1 | Mechanism sign-off (official CLI) | approved | | |
| 2 | CLI acquisition and pinned SHA-512 | approved | | |
| 3 | Changed-content policy (`diverged`) | approved | | |
| 4 | Destination layout, staging cap, migrated level | partly decided | | level still open |
| 6 | First Proton sign-in | done before M3 | | The session is in libsecret. |
| 7 | Live contract test (read-only) | done (M3) | | `version`, `list`, `info` only |
| 8 | First real dry run (`mirror plan`) | **not run** | | |
| 9 | First controlled folder (`mirror run --only … --round-trip`) | **not run** | | |
| 10 | Production-wide migration | **not run** | | needs `--all` (not built) and approval |

## 4. Dry run (plan)

_Not run yet._ After `mirror plan --only <folder>` (or a whole-drive plan), record:

| Measure | Items | Bytes |
|---|---|---|
| Folders to create | | |
| Files to upload (`upload_new`) | | |
| Adopted (same content already there) | | |
| Check existing (no source SHA-1) | | |
| Diverged | | |
| Skipped: too large | | |
| Skipped: unsupported (names, OneNote, shortcuts) | | |
| Case collisions | | |
| Peak staging needed | | |

## 5. Run results

_Not run yet._ One row per approved run:

| Run id | Date | Scope | Outcome | Uploaded | Adopted | Diverged | Skipped | Failed | Bytes | Staging peak |
|---|---|---|---|---|---|---|---|---|---|---|

## 6. Verification coverage

From `mirror status`. Each level is reported as reached, never rounded up (see PROTON_INTEGRATION.md):

| Level | Files | % of eligible | Bytes |
|---|---|---|---|
| L4 round trip (measured) | | | |
| L3 remote-claimed only | | | |
| Not verified (L2 or below) | | | |
| **Eligible total** | | 100% | |

## 7. Exceptions

| Kind | Count | Handling |
|---|---|---|
| Diverged | | Left untouched on both sides. Review by hand. |
| Failed (with retry time) | | Persisted backoff, 60 s doubling to 6 h. |
| Skipped too large | | Raise `staging_cap_bytes` or handle by hand. |
| Skipped unsupported name | | Rename on OneDrive, or copy by hand. |
| Source deleted after mirroring | | Kept on Proton. |
| Moved on OneDrive | | The old copy is kept on Proton. |

## 8. Safety evidence

| Check | Offline result (tests) | Production result |
|---|---|---|
| No Graph writes (no DELETE, PATCH, PUT or POST) | pass: `plan_and_run_add_only_and_never_write_to_onedrive`, `onedrive_mock_to_memory_destination` | _not measured_ |
| Only allowlisted `proton-drive` argv, uploads with `skip` | pass: the same test and `crates/provider/tests/proton_cli.rs` | _not measured_ |
| Destination unchanged after a source edit, rename or delete | pass | _not measured_ |
| `tokens.json` never written by the mirror | pass: `the_onedrive_sign_in_is_only_read` | _not measured_ |
| Staging never above the cap; nothing left staged | pass (crates/mirror tests) | _not measured_ |

## 9. Known gaps and open decisions

- **Binary signature.** The CLI binary is unsigned; only the SHA-512 pin protects it.
- **Proton's crypto migration** (late 2026 or early 2027) will need a CLI upgrade, which is a checkpoint.
- **Telemetry.** The CLI identifies itself as `cli-drive` and its telemetry follows the account preference.
- **Migrated level.** Decide whether L3 or L4 counts as "migrated".
- **Whole-drive runs** need a `--all` flag (not built) and their own approval. Batch size and schedule are still to decide.
- **No GUI or daemon integration** yet (M5).
- **Cutover** (§10.13) is never automatic.

## 10. Sign-off

| Role | Name | Date | Decision |
|---|---|---|---|
| Owner | | | |

## Measured: controlled test migration (2026-10-29)

These are real results against the live accounts, not simulated. Scope was one synthetic folder only: no personal data, and no whole-drive run.

| Item | Result |
|---|---|
| Source | OneDrive `/CloudDrive Mirror Test`: 11 synthetic files in 5 folders, 25.0 MiB |
| Test cases covered | Unicode, emoji and punctuation names; 3 levels of nesting; an empty file; a 25 MiB file |
| Destination | Proton Drive `/my-files/OneDrive/CloudDrive Mirror Test` |
| Tool | `proton-drive` CLI 0.9.0 (checksum pinned), Axentra CloudDrive build `c72c8f6` |
| Dry run | 10 s, read-only: 5 folders to create, 11 uploads, 25.0 MiB |
| Run | 4 min 28 s: 5 folders created, 11 files uploaded, 0 diverged/skipped/failed |
| Verification | 11/11 files (100 %) at **L4**: downloaded back through the CLI's own integrity checks and byte-compared |
| Idempotency | A second dry run found all 16 items already mirrored, with 0 to upload |
| Staging | Empty after the run; folder mode 0700; manifest mode 0600 |
| Writes to OneDrive | None: GraphSource only reads |
| Deletes / moves / replaces on Proton | None: these are not constructible in the adapter |

Whole-drive inventory: several hundred thousand OneDrive items, measured from the local sync database. The full migration has **not** been planned or run and needs separate approval.

Observed throughput was about 0.1 MiB/s including the L4 round trip. Most of that time is per-file CLI overhead (process start and SDK session), not bandwidth. At this rate a full migration needs batching and a faster mode; see the cutover blockers.

### Cutover blockers (current)
- No full-drive dry run yet. It requires a whole-drive enumeration of several hundred thousand items.
- Per-file CLI overhead makes a whole-drive migration slow. This needs batched uploads (the CLI accepts several local paths per call) and possibly L3-only for bulk, with L4 sampling.
- The mirror runs from the CLI only. Background, scheduled runs and the Cloud Mirror GUI page are not built yet (M5).
- Proton plans a crypto-model change in late 2026 or early 2027 that will force a CLI upgrade. Each upgrade is a checkpoint.
