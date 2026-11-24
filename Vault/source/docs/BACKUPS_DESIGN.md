# Axentra Vault: central versioned backups (reviewed design)

Implemented on `audit/fixes` (`fa56113`..`83e5a1b`). Where the code deviates, the commit messages explain why; the code is authoritative.

Base: `audit/fixes` @ 174acaf. Draft: not kept.

## Code facts checked

- **Ring copies.** `write_ring` (session.rs L2414–2425) copies the previous ring to `.axvault.old`. On a revocation it runs after the database is already on K_new (L2391 → L2398), so `.old` holds the revoked envelope that wraps K_old. Security #1 is confirmed.
- **The `.old` test.** floating.rs L2565 sits in the add-key test (`added_keys_join_the_ring_without_rekeying`), where the ring keeps the same key.
- **Key-file bytes differ by unlock path.** `Credentials.key_file` holds the XML `.keyx` bytes after a key-file unlock (jobs.rs L81–85). It holds 32 raw bytes after a passkey unlock (L127) or a key change (session.rs `finish_key_change`). An id hashed over the file bytes is therefore not canonical.
- **The writer only saves KDBX 4.1.** keepass 0.15.2 `dump_kdbx4` refuses anything except `KDB4(1)` (format/kdbx4/dump.rs L31), and `save.rs` rejects KDB3. So KDBX 3.1 and 4.0 files cannot be re-keyed with the writer at all: the save fails, it does not silently upgrade. This corrects the durability review's "3.1 comes out as 4".
- **Fresh seeds are automatic.** Every save regenerates the master seed, IV, inner stream key and KDF seed (dump.rs L36–46). Only `kdf_config` needs copying for security #11.
- **Both new identity schemes are feasible.**
  - `Meta.custom_data` is a public `HashMap` (types/meta.rs L82), and `Group::uuid()` exists.
  - `DatabaseKey::get_key_elements` is `pub(crate)`, so the composite key has to be computed in our code, which depends on the M5 fix.
- **Lossy writer.** `fidelity::check` exists and can be split into a two-input compare. `verify_db` compares counts only.
- **Plugin.** The KeePass plugin still writes `.axvault.old` (AxentraVaultExt.cs L221–233).
- **Existing tests that assert `.bak` or `.old`:**
  - vault.rs L304 and L326
  - jobs.rs L345
  - ui_tests.rs L436
  - floating.rs L2565

## Decisions

### Accepted changes to the draft

1. **Folder identity: from a path hash to a vault id taken from the root-group UUID.** The folder is named `<stem>` (or `<stem> (n)`), and settings map the path to the folder. A path hash orphans history on a rename, a move or a drive-letter change, and a hash of a guessable path leaks it (compat #8, sec #9, dur #16).
2. **File names carry a sequence number.** The name time is the source file's mtime, followed by `#seq`, plus `~rev` for re-keyed copies. The " (2)" collision suffix is dropped and names are parsed from the right. Local-time names collide and sort wrongly around DST changes and clock jumps, and the content time is what users search for (dur #2, compat #13).
3. **Order by `seq`, never by wall clock.** This prevents clock, DST and forged-date ordering bugs (dur #2, sec #6).
4. **Retention uses the last D distinct days and M distinct months, not windows measured from now.** Some kinds are pinned, and abort guards apply. `prune()` takes no clock input. Windows from `now` mass-delete after a clock jump or a long absence (dur #2, compat #11).
5. **The store never replaces a file.** Files are published with a no-replace `MoveFileExW(…, 0)`, temp files are named `.~axv-<hex>.tmp`, and backups are marked read-only. `std::fs::rename` silently replaces, and read-only stops KeePass from saving over a backup (dur #4/#15, sec #12, compat #16).
6. **Skip writing `auto` and `opened` copies whose SHA-256 equals the newest entry's.** Retries and repeated locks would otherwise push real versions out of the last 20 (dur #9).
7. **Key id: replaced by a random credential generation stored inside the KDBX together with a salted check value.** Labels are assigned by content hash. The draft's id was non-canonical, allowed a KDF-free key-file test, ignored password changes and mislabelled external bytes (sec #4, dur #12, compat #3).
8. **`index.json` is a rebuildable cache.** Automatic deletion of `.kdbx` files never depends on it. It is forgeable and can lose updates (dur #5, sec #6).
9. **Save ordering.**
   - Read the previous bytes once.
   - Write exactly those bytes to the store.
   - Re-check the main file's stamp and hash immediately before the main rename.
   - Use a unique `create_new` main temp.

   This closes the copy/rename window, the wider form of M3 (dur #3), and folds in M3 spec 6.
10. **Fallback chain.** Configured root, then the default local root, then `<db>.bak` (ordinary saves only), then fail the save. One notice per session. An app-written fallback `.bak` is imported into the store automatically once the store works again. A `.bak` in OneDrive undoes the goal, and credential changes must never publish a weaker copy (all three reviews).
11. **"Save anyway" (overwrite) parses the on-disk bytes with the session's credentials first.** If they don't open, it refuses. The pre-overwrite copy is labelled `replaced-external`. This stops a second device from writing a revoked key back over a rotated database (sec #5, compat #5).
12. **Re-key is staged before the commit, inside the key change, behind a modal progress dialog.**
    - Cancel, or a lock, aborts the whole key change.
    - Each file passes a fidelity gate, a KDF budget preflight and a format check, and is re-keyed with the current `kdf_config`.
    - Publishing writes the new copy first and deletes the original after. A manifest makes a crash recoverable.
    - The key change also imports `<db>.bak` into the store first.

    After the commit, K_old is gone, so a background job could never resume a revocation. Staging also removes all resume state. The writer is lossy for files written by other apps (sec #1/#2/#3/#11, dur #1/#8, compat #9).
13. **Sidecar.**
    - No more `.axvault.old`.
    - The new ring is staged beside the database before the database commit.
    - The store keeps snapshots of the current ring only.
    - At a key-change commit, every other-generation ring snapshot and the legacy `<db>.axvault.old` are deleted.

    Old rings hold revoked envelopes (H4). M2 restart recovery must work while the store is offline (sec #1, dur #11, compat #14). This folds in the ring part of M2 spec 7.
14. **The first-open snapshot becomes an `opened` snapshot at every unlock, whenever the on-disk hash differs from the newest entry.**
    - It uses the exact bytes that were parsed, on the unlock worker.
    - It is skipped for `open_sealed`, but that path still gets the store.

    This captures versions saved by other apps and avoids a second read (dur #13, compat #5).
15. **Restore is defined in phase 1.** Open the backup with the current credentials, re-save it under the current credentials, `kdf_config` and generation through the normal save, which writes a `before-restore` copy first. Never byte-copy. Copying a backup over the main file breaks passkey unlock and can bring back revoked or weaker keys (compat #1, sec #7).
16. **Opening a file that lies under any backup root gives a read-only session.** It isn't remembered, gets no store of its own, and shows a banner. Otherwise the app makes a "backup of the backup" and saves edits into a backup (compat #2).
17. **Root availability and safety.**
    - A custom root needs a marker file and is never created by the app.
    - A 2 s probe runs on a worker thread.
    - A free-space check runs before writing.
    - Warnings for FAT/exFAT, UNC paths and sync roots.
    - The database's own folder is rejected.
    - New folders under a custom root get a protected DACL.

    This covers wrong-device writes, NAS hangs, a full C: and other local users (dur #6/#10, sec #8, compat #7).
18. **Known roots: default, configured and any previous root.** All are listed, re-keyed and pruned. A root change or a fallback must not orphan backups (compat #7, dur #16).
19. **Migration copies, verifies the hash, then deletes the source.** It also covers `.axvault.old`, `<db>.tmp` and `.bak.tmp`. Imported copies are pinned, and there is a "Don't ask again" option. A plain rename fails across volumes and gives up the original before anything is verified (dur #14, compat #15).
20. **Retention is fixed and stated as text, with no number fields.** That avoids user-entered zeros (compat #11).
21. **"Back up now" is disabled while there are unsaved changes, with the hint "Save first".** Manual copies skip the duplicate check. Users expect a pinned copy of what they see (compat #12).
22. **New actions: "Delete backups with older credentials (N)" and "Delete all backups".** A user-driven way out of residual old-key copies (sec #13).
23. **Default `.keyx` name `{stem}-backup YYYY-MM-DD.keyx`, with a warning if it is inside a backup root or on the same removable volume.** `create_new` forces ad-hoc names, and a key file on the same USB stick as the backups defeats the second factor (compat #3, sec #8).
24. **Outside-change notice.** When the stored check value doesn't match at unlock, tell the user their key changed outside Axentra. Covers KeePass/KeePassXC key changes and plugin enrollments (compat #4).
25. **UI strings, README and installer text updated.** Several current claims become false (compat #10).
26. **One `backups_section` used by both the Classic and Floating settings, with settings in `%APPDATA%\Axentra Vault\backups.json`.** Both layouts need it (compat #17).
27. **Prerequisites: L1 (a dead `Job` reports failure), M5 (effective key-file element) and M4 (capped reads, KDF budget) land first.** The staging job, the credential check and the preflight depend on them.

### Rejected

- **HMAC-authenticated index (sec #6).** Write access to the store already allows deleting every backup. What would be bought is covered by `seq` ordering, the prune guards, the KDF preflight, restore re-saving under current credentials, and the DACL.
- **Post-commit background re-key with resume (sec #3, dur #8), including keeping a K1 ring snapshot to resume from.** The snapshot reintroduces H4. Pre-commit staging needs no resume state at all.
- **Cross-process `.lock` file (dur #5).** The app runs as a single instance per profile. No-replace names with `seq` retry, a cache-only index and a prune that tolerates NotFound and sharing violations make races harmless. An in-process mutex is enough.
- **Byte budget with tier thinning (dur #10).** Deferred. A free-space check covers the real failure, saves failing on a full C:.
- **Fidelity gate on the save-time H3 copy (dur #1).** The main save of the same content goes through the same writer, so the gate would add 2 KDF runs on the UI thread without changing the outcome.
- **Key-class keepers in retention (dur).** Generation is orthogonal to version history, and the re-key exists to eliminate old generations.
- **Deletion protection for files written in the last 10 minutes or dated in the future (dur).** `seq` ordering already puts new files in the newest slot or the recent tier.
- **`±HHMM` offset in names (dur).** Ordering uses `seq`, so the offset is only noise for users.
- **Key id over `raw_key` (compat #3).** It is still non-canonical for hashed key files and gives a KDF-free test.
- **Estimated time remaining (compat #9).** An N/M progress count is enough.
- **Confirm before closing while a job runs.** Accepted only for the modal key change, where closing means cancelling it.
- **Rejecting OneDrive or sync folders outright (compat #7).** Changed to a warning with confirmation. Only the database's own folder is rejected.
- **"Back up the new bytes too, for up to 2 rounds" on overwrite (dur #3).** A failed re-check simply returns `ChangedOnDisk` again.
- **Reading the DACL back to verify it (sec #8).** Checking that the API call succeeded is enough for v0.1.

## Final spec

### Data layout

- **Default root:** `%LOCALAPPDATA%\Axentra Vault\Backups\`. The app may create it. Per-user ACL, not synced.
- **Custom root:**
  - Chosen by the user and never created by the app.
  - Must contain `.axentra-backups` (JSON `{"marker":"<uuid>"}`) whose marker matches settings.
  - On NTFS, each new per-vault folder gets the protected DACL `D:P(A;OICI;FA;;;<user SID>)(A;OICI;FA;;;SY)`.
- **Per-vault folder** `<root>\<stem>[ (n)]\` contains:
  - `vault.json`: `{"v":1,"id":"<vault id>","stem":"…"}`. No path is stored in the store.
  - `index.json`: the cache (below).
  - `rekey.json`: the staging manifest, present only during or after an interrupted key change.
  - backups and ring snapshots.
- **Vault id:** `hex(SHA-256("axentra-vault/store-id/v1" ‖ root_group_uuid))[..32]`.
- **Folder lookup** (settings `backups.json` maps `path → {id, root, folder}`):
  1. The settings entry for this path exists and its folder's `vault.json` id matches: use it.
  2. A settings entry with this id exists and its recorded path no longer exists (moved or renamed): reuse it and update the path.
  3. Otherwise scan the root's `vault.json` files (at most 256). An id match that no live settings entry claims is reused (covers a reinstall).
  4. Otherwise create `<stem>` or `<stem> (n)`. A copy of the database (same id, both paths exist) gets its own folder.
- **`index.json`** (written atomically under the mutex):
  ```
  { "v":1,
    "gens":    { "<gen>": { "created":"<UTC>", "committed":true, "keyx":"Personal-backup 2026-10-14.keyx"|null } },
    "entries": { "<file name>": { "sha256":"…", "size":n, "gen":"<gen>"|null, "taken":"<UTC>",
                                  "rekey":null|"failed: lossy|format|kdf|open" } },
    "fallback_bak_sha256": "…"|null }
  ```
  - A missing or corrupt index is rebuilt from the names, with hashes recomputed and `gen=null`.
  - Entries for missing files are dropped after each prune.

### File naming

- **KDBX:** `<stem> YYYY-MM-DD HH-MM-SS #<seq ≥6 digits>[~<rev>][ <kind>].kdbx`
- **Ring:** `<stem> YYYY-MM-DD HH-MM-SS #<seq> passkeys.axvault`
- **Kinds:**

  | Kind | Meaning | Pinned |
  |---|---|---|
  | none (`auto`) | previous version, written before a save | no |
  | `opened` | snapshot taken at unlock | no |
  | `replaced-external` | copy written before "Save anyway" overwrites another device's version | no |
  | `manual` | "Back up now" | yes |
  | `before-restore` | copy written before a restore | yes |
  | `before-key-change` | the H3 copy written during a key change | yes |
  | `imported` | migrated or recovered `.bak` / `.tmp` files | yes |
- **Parsing:**
  - Right-anchored: `^(.+) (\d{4}-\d{2}-\d{2}) (\d{2}-\d{2}-\d{2}) #(\d{6,})(?:~(\d+))?(?: (kind|passkeys))?\.(kdbx|axvault)$`.
  - Any non-empty stem is accepted (the database may have been renamed).
  - Hand-written parser, so no regex dependency.
  - A file that doesn't parse is foreign: never touched or listed as a backup.
- **Name time:** local time of the source version's mtime (falling back to now if it is missing or in the future). The backup's own mtime is set to the same value. Ring snapshots use now.
- **seq:** under the mutex, 1 + the maximum `seq` over all parsed `.kdbx` and `.axvault` files in the folder. A no-replace collision retries with seq+1, up to 16 times.
- **Sanitized stem:** replace `<>:"/\|?*` and control characters with `_`, trim trailing dots and spaces, cap at 64 characters.

### Write primitive (store)

1. `create_new` `.~axv-<16 hex>.tmp` in the target folder, write, then `sync_all`.
2. `MoveFileExW(tmp, final, 0)` (no replace). On `ERROR_ALREADY_EXISTS`, bump `seq` and retry.
3. Reopen the final file and `sync_all`. Set mtime, then the read-only attribute.
4. Update the index.

Never claim power-loss safety without a power-cut test. Every delete clears read-only first and ignores NotFound and sharing violations.

**Temp sweep** at folder open: delete `.~axv-*.tmp` files older than 1 h that are not listed in `rekey.json`.

### Credential generation (replaces the key id)

- **`gen`:** 128-bit random value in hex.
- **In-database record:** Meta CustomData item `AxentraVault.Credentials` = `v1;<gen>;<salt 32B hex>;<check 32B hex>`.
  - `check = HMAC-SHA256(salt, "axentra-vault/credential-check/v1" ‖ composite)`.
  - `composite = SHA-256(SHA-256(password)? ‖ keyfile_element?)`, computed exactly as KeePass does.
  - `keyfile_element` comes from M5's `keyfile::effective_key`: XML data → 32 raw bytes → 64 hex characters → otherwise SHA-256 of the file.
  - The same key therefore yields the same check whether it arrives as raw bytes or as `.keyx` XML.
  - Nothing derived from key material exists outside the encrypted KDBX.
- **At open:**

  | Record | Session generation | Committed | Other effect |
  |---|---|---|---|
  | Present and check matches | the recorded gen | yes | — |
  | Missing | new random gen and salt | no (provisional) | — |
  | Check mismatch | new random gen and salt | no (provisional) | `outside_change = true`, which shows the notice |
- **At every save:** if the record differs from (session gen, salt, check), write it into `db.meta.custom_data` before serializing. After the main rename commits, set `gens[gen].committed = true`.
- **Labels:** an entry gets the session gen only if its SHA-256 equals bytes this session opened or saved, or bytes it parsed with session credentials on the overwrite path. H3 copies and re-keyed copies get the new gen. Everything else is `null`.
- **Display:**
  - session gen: "Current key"
  - other committed gen: "Older key – needs your backup key file from <date> (<keyx name>)"
  - otherwise: "Unknown"
- **Labels are hints only.** The re-key tries every file whose gen ≠ the target gen, including `null`.

### Retention (`prune(folder) -> PruneReport`; no clock input)

```
reps = parsed .kdbx regular files (not reparse points), grouped by seq; representative = highest rev
       (lower revs exist only during or after an interrupted re-key: the manifest owns them, prune ignores them)
S    = reps sorted by seq desc
REG  = {auto, opened, replaced-external}; PIN = {manual, before-restore, before-key-change, imported}
K  = {S[0]}
K += every e in S with kind ∈ PIN
K += first 20 of [e in S | kind ∈ REG]
days = {}:   for e in S with kind ∈ REG: d = e.name_date;     if d ∉ days and |days| < 30: days += d; K += e
months = {}: for e in S with kind ∈ REG: m = e.name_date[..7]; if m ∉ months and |months| < 12: months += m; K += e
C = S \ K
ABORT (delete nothing, record warning) if any:
  - a re-key or staging job is active on this folder (skip, not a warning)
  - S[0] < 512 bytes, or lacks the KDBX signature 03 D9 A2 9A 67 FB 4B B5
  - |C| > max(5, |S| / 4)
  - vault.json is missing or its id ≠ this vault's id
  - duplicate (seq, rev)
for e in C (oldest seq first): clear read-only; remove
Ring snapshots: keep the newest 3 whose index gen == session gen; delete every other *.axvault snapshot
  (unknown gen included: app metadata that may hold revoked envelopes)
Drop index entries for missing files.
```

- Prune runs after a successful save, from every known root that is available, and never fails the save.
- At most 1 + 20 + 30 + 12 regular backups per folder, plus the pinned ones.
- A wildly wrong clock costs at most one daily and one monthly slot.

### Save ordering (`Vault::save_overwriting`, shared by `save`)

1. Refresh the CustomData record. Serialize to `buf` under the current credentials and run `verify_db`.
2. Read `prev` from the database path once into a `Zeroizing` buffer (capped at 512 MiB), then stamp it and hash it (`FileStamp {modified, len, sha256}`).
   - Normal save: if `prev.sha256 ≠ on_disk.sha256`, return `ChangedOnDisk`.
   - Overwrite: parse `prev` with the on-disk credentials (`previous_credentials` if set, otherwise the current ones). If that fails, return `VaultError::CredentialsChangedElsewhere` ("Lock and reopen; saving would put back credentials changed on another device"). The kind is `replaced-external`, labelled with the session gen.
3. Backup bytes:
   - Credential change pending (H3): parse `prev` with the old credentials, set `config.kdf_config` to the current one, save under the new credentials and verify. Kind `before-key-change`, gen = new gen.
   - Otherwise: `prev` as-is. Kind `auto`, or `replaced-external`, or `before-restore` when a restore is pending.
4. Write a unique main temp `<db>.~axv-<hex>.tmp` (`create_new`, `sync_all`).
5. Write the backup through the fallback chain (below).
   - Skip `auto` if its SHA-256 equals the newest entry's.
   - If every option fails: delete the main temp and return an error. The main file is untouched.
6. Re-stamp and re-hash the database path. If it differs from `prev`: delete the main temp and return `ChangedOnDisk`. The backup written in step 5 stays.
7. `MoveFileExW(main temp, db, REPLACE_EXISTING | WRITE_THROUGH)`.
8. Set `on_disk` = the stamp of `buf`, record the hash of `buf` as session-saved, set `previous_credentials = None`, `dirty = false`, and mark the gen committed.
9. Prune (errors are reported, never fatal).

**Open** stamps before and after the read and retries once on a mismatch (M3).

### Fallback

- **Order:**
  1. Configured root. It must be available: marker matches, the probe answers within 2 s on a worker thread, enough free space.
  2. Default root.
  3. `<db>.bak` (temp + `sync_all` + `MoveFileExW(REPLACE | WRITE_THROUGH)`). Its SHA-256 is recorded in the default index as `fallback_bak_sha256`.
  4. Fail the save.
- **Credential-change saves stop at step 2.** On failure, `finish_key_change` rolls back.
- **Free space** (`GetDiskFreeSpaceExW`): at least backup size + 64 MiB, plus 2 × database size + 256 MiB when the root is on the database's volume.
- **Notices:** one per session ("Backup folder unavailable; using <default>" or "…kept as <db>.bak"), plus a status line that stays until a store write succeeds.
- After a successful store write, an existing `<db>.bak` whose hash matches `fallback_bak_sha256` is imported automatically (copy, verify, delete).

### Re-key during a key change (first passkey setup and revocation)

- **Add-key:** the database key is unchanged, so there is no re-key. Write the ring as described under Sidecar, then snapshot it.
- **Key change:**
  1. **Worker:** registration or rotation, as today, produces `K_new` and the ring.
     - The confirmation shown at click time now also says:
       - "Your N backups will be re-encrypted to the new key; afterwards they need the new backup key file."
       - when a `.bak` exists: "<db>.bak will be moved into the backup folder."
       - when some roots are unreachable: "Backups in <X> aren't reachable and will keep the old credentials."
       - "Copies outside the backup folder (OneDrive version history, other apps' backups, phones) keep their old credentials."
  2. **UI:** the `.keyx` save dialog, named `{stem}-backup YYYY-MM-DD.keyx`. It keeps the existing OneDrive warning and adds a warning when the path is inside a backup root or on the configured root's removable volume. Then `write_new_backup`.
  3. **Worker, staging.** An `egui::Modal` shows "Re-encrypting backups i/N…" with Cancel. It counts as `key_job_pending`, with an idle-lock ceiling of 60 s + 30 s × N, capped at 30 min.
     - Generate the new gen G'.
     - Import `<db>.bak`, if present, into the store (kind `imported`, verified copy). The original is deleted only after the commit.
     - For each KDBX representative in every reachable known folder whose gen ≠ G':
       1. **Preflight on bytes read with a cap:** the header KDF must be within the M4 budget and at most 2 × the current database's KDF cost, and the version must be KDBX 4.1. Otherwise mark it `failed: kdf|format`.
       2. Parse with the old credentials. If that fails and the old credentials had a password plus a key file, try password-only. Otherwise mark `failed: open`.
       3. Set `config.kdf_config` to the current one and save under the new credentials to a temp file.
       4. Gate: `fidelity::compare(get_xml(orig, old), get_xml(temp, new))` must be clean (dropped fallback icons are tolerated). Otherwise delete the temp and mark `failed: lossy`.
       5. Add `{tmp, original, original_sha256, tmp_sha256}` to `rekey.json` (`{"to_gen": G', items}`).
     - About 4 KDF runs per file.
     - **Cancel or lock:** set the cancel flag, which is checked before every KDF run. The worker deletes its temps and the manifest. Lock waits up to 5 s with the UI showing "Locking…". Result: "Key change cancelled; nothing changed."
  4. **UI, commit:**
     - `set_credentials(new, G')`.
     - Stage the ring as `<db>.axvault.~new` (`create_new` + `sync_all`).
     - `vault.save()`. H3 writes `before-key-change` to the store, and the store is required here.
     - On failure: roll back the credentials and delete the staged ring, the temps and the manifest.
  5. `MoveFileExW(<db>.axvault.~new → <db>.axvault, REPLACE | WRITE_THROUGH)`.
  6. **Publish each manifest item:**
     - no-replace move of the temp to `<same name>~<rev+1>`;
     - set mtime to the original's, set read-only;
     - set the index gen to G';
     - clear read-only on the original and delete it.

     Then delete `rekey.json`.
  7. **Cleanup:**
     - Delete every ring snapshot whose gen ≠ G'.
     - Delete `<db>.axvault.old`.
     - Delete the original `.bak` if its hash still matches the imported copy.
     - Snapshot the new ring into the store.
     - Set `gens[G'] = {created, committed:true, keyx: <file name>}`.
  8. **Notice:** "Unlock keys updated. Use the NEW backup key file: <path>. N backups re-encrypted; M kept their old credentials (Settings → Backups)." Failed entries keep `rekey: failed:*` and appear under "Delete backups with older credentials".
- **Crash recovery at the next unlock:**
  - If `rekey.json.to_gen` equals the database's committed gen: finish step 6 using the manifest hashes. No KDF is needed.
  - Otherwise: delete the manifest's temps and the manifest.
  - If passkey unlock fails: try `<db>.axvault.~new`, then the current-generation store ring snapshots, newest first. Accept a ring only if its key opens the database, then repair the sidecar (M2 spec 7, adapted).

### Sidecar snapshots

- Axentra never writes `.axvault.old` again.
- `write_ring` uses the staged `create_new` file plus `MoveFileExW(REPLACE | WRITE_THROUGH)`.
- After every committed ring write, and at unlock when the sidecar's SHA-256 differs from the newest ring snapshot, write `… passkeys.axvault` to the store, labelled with the session gen.
- Retention: newest 3 of the session gen. Other generations are deleted at commit and by prune.
- M2 backup-path collision checks also reject any path inside a known backup root.

### Unlock snapshot (`opened`)

- On the `jobs::unlock` worker, after a successful `Vault::open`, which now returns its parsed `data` buffer:
  - resolve the folder;
  - write `data` as `opened` if its SHA-256 ≠ the newest entry's;
  - label it with the session gen (provisional if uncommitted).
- Also on unlock:
  - the ring snapshot rule above;
  - import any leftover `<db>.~axv-*.tmp` that has a KDBX signature and is at least 512 bytes, as `imported` (verified copy, then delete); delete smaller leftovers older than 1 h;
  - run manifest recovery;
  - show the `outside_change` notice.
- `jobs::unlock` takes a `BackupConfig` and attaches the store to the vault on both the `open` and the `open_sealed` path. The snapshot runs only on `open`.

### Restore

- Settings → Backups → "Restore…" opens a picker limited to the known folders.
- **Steps** (`Vault::restore(path)`):
  1. KDF preflight, capped read, then parse with the session credentials. If that fails: "This backup needs older credentials; restoring those comes in a later version."
  2. Copy `self.db.config` (version 4.1, ciphers, KDF) and the CustomData record onto the restored database.
  3. Replace `self.db`, mark dirty, set `pending_kind = before-restore`, then call `save()`. A `ChangedOnDisk` result leads to the overwrite dialog.
- **Texts:**
  - Restore: "Close this vault on your other devices and apps first; restoring replaces it everywhere it syncs."
  - Next to "Open folder": "To recover a backup, use Restore. Copying a file over your vault can lock you out."

### Opening a backup as a vault

- If the path is under any known root:
  - no `remember_database` and no store;
  - `read_only = true`, so Save and Ctrl+S are disabled and lock never saves;
  - banner: "Backup from <name date>. Changes here aren't saved. To restore it, open your vault and use Settings → Backups → Restore."
- Unlock hint: "Backups open with your master password + backup key file; passkeys unlock only the main vault."

### Migration (prompt once per vault after unlock; "Don't ask again" stored per vault id)

- **Offered when any of these exist:** `<db>.bak`, `<db>.tmp`, `<db>.bak.tmp` (KDBX signature required), `<db>.axvault.old`.
- **KDBX files:** copied through the write primitive (kind `imported`, name time = file mtime, gen `null`), read back and hash-compared, then read-only cleared and the source deleted. If the delete fails, keep the source and say so.
- **`.axvault.old`:** deleted. It is app metadata and may hold revoked envelopes.
- **Prompt also says:** OneDrive keeps moved files in its recycle bin and version history.
- **Automatic, no prompt:** the fallback-`.bak` import, the `.bak` import during a key change, and the `.axvault.old` delete at commit.

### Settings and UI

- **`%APPDATA%\Axentra Vault\backups.json`:**
  ```
  {"root":{"path","marker"}|null, "previous_roots":[{"path","marker"}],
   "vaults":{"<lowercased path>":{"id","root","folder"}}, "migration_declined":["<id>"]}
  ```
  - Known roots = default + `root` + `previous_roots`.
  - Changing the root moves the old one into `previous_roots`. Its backups stay where they are, and are still listed, re-keyed and pruned.
- **Choosing a root:**
  - Reject the database's own folder.
  - Warn and require confirmation for: inside OneDrive/Dropbox/Google Drive (environment variables plus known folder names), FAT/FAT32/exFAT (`GetVolumeInformationW`), UNC paths.
  - Write the marker.
- **Retention is shown as text:** "Keeps the newest 20 saves, one per day for the last 30 days with saves, one per month for the last 12 months with saves, and every manual, restore, key-change and imported backup."
- **Shared `backups_section(ui)`:** used as a modal from the Classic settings dropdown and in the Floating `settings_page`. It contains:
  - location and "Change…";
  - count and newest backup;
  - fallback or stale status, plus "N backups use older credentials";
  - "Back up now" (disabled when dirty: "Save first");
  - "Restore…", "Open folder", "Move old .bak" (only when present);
  - "Delete backups with older credentials (N)" and "Delete all backups", each confirmed.
- **The Backups section explains:**
  - backups are on this PC only, and phone apps can't see them;
  - they need the same password and key and don't replace the backup key file;
  - copies outside the store keep their old credentials.
- **Strings to change:**
  - session.rs L1925: "Saved. The previous version is in your backup folder."
  - L1945 and L1996: "kept as a backup"
  - L2249: revoke dialog now says backups are re-encrypted
  - L2406: success notice above
  - README L72 and L102–104
  - vault.rs module doc L1–3 and doc comment L181
- **Installer:** the Inno `[Messages]` text and the WiX comment state that `%LOCALAPPDATA%\Axentra Vault\Backups` is kept on uninstall.

### Concurrency

- One `Arc<Mutex<()>>` per folder, covering seq allocation, publish, index writes and prune. No cross-process lock.
- Prune skips any folder that has an active staging job.

## Tests to write

### vault-core `backups.rs`

| Test | Asserts |
|---|---|
| `name_parse_roundtrip_any_stem` | Stems containing dates, spaces, `#` or `~`; all kinds, `~rev` and `passkeys` round-trip. `X 12-00-00 (2).kdbx` and unknown kinds parse as foreign. |
| `foreign_files_are_never_touched` | Random files, young `.~axv` temps and `.txt` files survive prune and the sweep. |
| `publish_never_replaces` | With the target name pre-created, write lands at seq+1 and the existing bytes are unchanged. |
| `seq_spans_kdbx_and_axvault` | seq is allocated across both extensions. |
| `dedupe_skips_identical_newest_but_not_manual` | Identical bytes are skipped for `auto`/`opened` but still written for `manual`. |
| `prune_keep_set_exact` | 400 synthetic entries over 2 years give exactly newest + 20 recent + 30 distinct days + 12 distinct months + pinned. |
| `prune_has_no_clock_dependence` | Shifting the newest 5 names by +80 years changes at most one day slot and one month slot; S[0] and pinned entries are kept. |
| `prune_after_13_month_gap_keeps_monthly_history` | Monthly entries from before the gap survive. |
| `prune_orders_by_seq_not_name_time` | Duplicate and DST-repeated times: the highest seq is the one kept as newest. |
| `prune_never_deletes_pinned` | Pinned kinds survive any amount of pruning. |
| `prune_aborts_on_mass_delete` | Nothing is deleted and the report says aborted. |
| `prune_aborts_on_corrupt_newest` | Newest truncated to 100 bytes: nothing is deleted. |
| `prune_aborts_on_foreign_vault_json` | Nothing is deleted. |
| `prune_clears_readonly_and_tolerates_missing` | Read-only files are deleted and missing files cause no error. |
| `ring_snapshots_keep_three_of_session_gen` | Other-generation and unknown-generation `.axvault` snapshots are deleted. |
| `index_rebuilt_when_missing_or_corrupt` | Every entry is listed, with gen `null`. |
| `sweep_respects_age_and_manifest` | Temps older than 1 h are deleted; temps listed in the manifest are kept. |
| `folder_identity_move_vs_copy` | Same id with the old path gone reuses the folder; with the old path present, a `Stem (2)` folder is created. |
| `custom_root_without_marker_is_unavailable_and_not_created` | No directories are created under it. |

### vault-core `vault.rs`

| Test | Asserts |
|---|---|
| `save_writes_exact_previous_bytes_to_store` | Store copy hash equals the previous on-disk hash; no `.bak`; the backup's mtime equals the source mtime. Replaces `save_round_trip_keeps_backup`. |
| `fallback_order_configured_default_bak` | Configured root unavailable → default root; default unwritable → `.bak` with the warning set; then the next good save imports that `.bak`. |
| `credential_change_never_falls_back_to_bak` | All roots failing makes the save fail; the main file still opens with the old credentials and no `.bak` exists. Replaces the `.bak` parts of `credential_change_protects_previous_contents_and_can_fail_safely`; the store-side H3 asserts are kept. |
| `save_fails_without_durable_previous_copy` | Every target failing leaves the main file untouched. |
| `external_write_between_backup_and_rename_is_detected` | A test hook rewrites the main file with the same length and restored mtime: `ChangedOnDisk`, outside bytes intact, no temp left behind. |
| `overwrite_refuses_other_credentials` | On-disk file rotated to K2 by a second Vault; a K1 session's `save_overwriting` returns `CredentialsChangedElsewhere` and the file is still K2. |
| `low_free_space_skips_root` | With an injected free-space function, the write goes to the next root in the chain. |
| `credential_check_detects_outside_change` | After a password change made directly through keepass, reopening gives a new provisional gen and `outside_change == true`. |
| `credential_check_is_form_independent` | Raw 32 bytes and `.keyx` XML give the same check and the same gen. |
| `no_key_material_outside_kdbx` | Hex of SHA-256 over the password, raw key, `.keyx` bytes or composite appears in neither `index.json` nor any file name. |
| `labels_follow_content_hash` | External bytes are `null`; opened and saved bytes get the session gen. |
| `restore_resaves_under_current_credentials_and_kdf` | Restoring a weak-AES-KDF backup: the main file opens with the current credentials, its header KDF equals the current one, and `before-restore` exists. |
| `restore_rejects_backup_needing_older_credentials` | The restore fails cleanly. |

### Re-key (vault-core, injected credentials)

| Test | Asserts |
|---|---|
| `rekey_stage_then_publish_new_then_delete` | Each original is replaced by a `~1` copy that opens only with the new credentials, keeps its mtime and is read-only; originals are gone. |
| `rekey_cancel_leaves_store_identical` | Every file's hash is unchanged; no temps or manifest remain. |
| `rekey_skips_lossy_unsupported_and_over_budget` | A KeePass-written fixture with plugin CustomData, a KDBX 4.0 file and a 4 GiB Argon2 header are untouched and marked `failed:*`; the over-budget one fails fast without a KDF run. |
| `rekey_covers_unknown_and_password_only` | With the index deleted, all files are tried; password-only backups are re-keyed during a revocation. |
| `manifest_recovery_publishes_or_discards` | Database on the new gen: the manifest is published; database not on the new gen: temps are deleted. |
| `key_change_imports_bak_and_rekeys_it` | `.bak` is imported and re-keyed. |
| `after_key_change_nothing_reachable_opens_with_old_credentials` | For both first setup and revocation: no KDBX under any known root, at `<db>.bak` or at `<db>.*.tmp` opens with the old credentials; no `.axvault*` beside the database or in the store contains the revoked credential id. Extends the jobs.rs L345 test. |

### App

| Test | Asserts |
|---|---|
| `added_keys_join_the_ring_without_rekeying` (updated, floating.rs L2565) | A current-ring snapshot is in the store and no `.axvault.old` exists. |
| `revocation_deletes_old_ring_snapshots_and_legacy_old` | Both are gone after the commit. |
| `key_change_staging_is_modal_and_lock_cancels_it` | `expire_idle_timer` past the ceiling locks; the database and store are unchanged; staged temps are gone. |
| `staged_ring_recovers_passkey_unlock_after_crash` | Database on K2, sidecar on K1, `.~new` holds K2: unlock succeeds and the sidecar is repaired. |
| `backup_opened_as_vault_is_read_only_and_not_remembered` | Save is disabled and the path is not remembered. |
| `back_up_now_disabled_when_dirty` | The button is disabled while there are unsaved changes. |
| `migration_copies_verifies_then_deletes` | A locked `.bak` stays and the message says so. |
| `unlock_snapshot_dedupes_by_hash` | Two unchanged unlocks give one `opened`; an external change gives a second. |
| `recovered_session_still_uses_store` | A save on the `open_sealed` path goes to the store. |
| ui_tests.rs L436 (updated) | Asserts a store entry instead of `.bak`. |

**Test isolation:** every test passes an explicit `BackupConfig`. None touches the real `%LOCALAPPDATA%` or `%APPDATA%`.

## Deferred (not in this phase)

- **Lossless KDBX4 outer re-wrap.** Re-encrypt the decrypted payload under the new key without going through XML. This also handles KDBX 4.0 files.
- **Older credentials:**
  - restoring backups that need older credentials;
  - "Re-encrypt with previous key file…";
  - automatic password-only re-key of folders that were unreachable during a key change.
- **Folders and roots:**
  - a "Move existing backups" job on root change;
  - listing orphaned folders;
  - mirroring the local store to a removable or network root;
  - importing default-root fallback copies into the configured root.
- **Space and process:** byte-budget thinning, and running the re-key in a child process killed on lock.
- **Other apps:**
  - KeePass plugin: stop writing `.axvault.old` and tell Axentra when it enrolls;
  - importing KeePassXC `.old.kdbx` files.
- **Later UI:**
  - showing KeePass's key-file check value per generation;
  - letting a passkey open a backup;
  - "Save a copy…" export;
  - the full Recovery & Keys page;
  - per-vault settings and retention number fields.
- **Power-cut testing.** Required before claiming durability.
- **Pre-existing finding (spawn a separate task).** keepass 0.15.2 saves only KDBX 4.1 (dump.rs L31). A vault last saved as KDBX 4.0 likely can't be saved by Axentra at all, and its H3 key change would fail. KDBX 4.0 is written by older KeePass and possibly KeePassXC; verify which. Decide between upgrading to 4.1 on save and refusing with a clear message.