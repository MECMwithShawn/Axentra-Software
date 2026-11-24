# Proton Drive integration

Axentra CloudDrive can copy OneDrive to Proton Drive. The copy goes one way only and only adds things. Nothing is ever deleted, moved, renamed, trashed or replaced on either side.

This is a third-party application not officially supported by Proton.

Status: development (0.3.0-dev). Everything below is built and tested offline against a fake `proton-drive` and the OneDrive mock. **No production mirror has been run.** See [MIGRATION_READINESS.md](MIGRATION_READINESS.md).

## Mechanism

```
axentra-onedrive mirror plan|run|status          (crates/cli/src/mirror.rs)
 ├─ GraphSource            OneDrive, read-only: GET requests only
 │    └─ Graph + TokenStore::open_read_only(tokens.json)
 ├─ axentra-clouddrive-mirror                    (crates/mirror)
 │    ├─ planner   dry run: lists both sides, changes nothing
 │    ├─ executor  L1→L4 per file, through private staging
 │    ├─ manifest  ~/.local/share/axentra/onedrive/mirror.sqlite
 │    └─ report    readiness: measured vs claimed vs unverified
 └─ ProtonCliDestination   Proton Drive                     (crates/provider/src/proton_cli.rs)
      └─ ProcessRunner → proton-drive 0.9.0 (official CLI, MIT; argv only, no shell)
```

- **Source.** `GraphSource` wraps the existing `Graph` client and uses only `me`, `drive`, `delta_page`, `item` and `download`. A test confirms that a full plan and run send no `DELETE`, `PATCH`, `PUT` or `POST` to Graph.
- **Destination.** `ProtonCliDestination` implements `MirrorDestination` by running the official `proton-drive` CLI. The `MirrorDestination` trait has no delete, move, rename or trash method, so the mirror has no way to express those calls.
- **Layout.** OneDrive's paths are mirrored beneath `[mirror] dest_root`, which defaults to `/my-files/OneDrive`. The OneDrive file `/Documents/a.txt` becomes `/my-files/OneDrive/Documents/a.txt`. The CLI destination's own root is `/my-files`, and the mirror folder is a path below it.
- **Upload naming.** The CLI names an upload after its local file. Each file is therefore staged at `staging/m-<sha1(id)>/<exact source name>` and hard-linked into a fresh private folder just before the upload.

### Commands

| Command | Effect |
|---|---|
| `axentra-onedrive proton status` | Checks the CLI's SHA-512 and version, then runs a read-only `list /my-files`. Prints only "signed in (ok)" or "not signed in", with no account fields and no file names. |
| `axentra-onedrive proton login` | Shows the third-party notice, checks the binary, then runs `proton-drive auth login` attached to the terminal. The CLI opens a browser and prints the sign-in URL. The session is stored in the CLI's own keyring entry (libsecret). |
| `axentra-onedrive mirror plan [--only PATH] [--json]` | Dry run. Reads OneDrive and Proton Drive and writes the plan to stdout. It makes zero changes on either cloud. It adds one `mode=dry` row to the local manifest. |
| `axentra-onedrive mirror run --only PATH [--round-trip] [--yes]` | Plans, shows the plan, and asks before starting unless `--yes` is given. It then mirrors that one folder. A run without `--only`, or with `--only /`, is refused, because a whole-drive run needs explicit approval and a future `--all`. Ctrl-C stops after the current step: a download in progress is abandoned, an upload in progress is let finish. A second Ctrl-C quits at once, and the next run checks an interrupted upload before trying again. Ctrl-C during planning or at the question ends the command straight away. Only one run at a time can use the staging folder. |
| `axentra-onedrive mirror status [--json]` | The readiness report from the manifest. Makes no network calls. |

### Configuration (`[mirror]` in config.toml, all optional)

| Key | Default | Notes |
|---|---|---|
| `enabled` | `false` | Reserved for automatic runs by the service (M5). The manual commands above ignore it. |
| `dest_root` | `/my-files/OneDrive` | Must be a folder inside `/my-files`. `/my-files` itself is refused. |
| `staging_dir` | empty, meaning `~/.cache/axentra/onedrive/mirror-staging` | Created with mode 0700. Must be an absolute path of its own: not your home folder or a folder holding it, not inside or around the OneDrive folder or the sync service's file cache, and with no `*`, `?`, `[` or `{`. A folder that already exists is used only if the mirror made it or it is empty; one holding anything else is refused and left untouched. |
| `staging_cap_bytes` | `2147483648` (2 GiB) | Files larger than this are skipped as `too_large`. |
| `proton_cli` | `~/.local/share/axentra/proton-drive-cli/0.9.0/proton-drive` | Must be absolute or start with `~/`. `PATH` is never searched. |
| `changed_content` | `diverged` | The only accepted value for now. `create-new-revision` exists in the code (`ChangedSourcePolicy`) but is refused until it is approved. |

The table is left out of the file while it has only default values, so saving settings from the app does not add it. Existing config files load unchanged.

## OneDrive sign-in: no token race

The sync service refreshes `tokens.json` while it runs. The mirror commands run in a separate process beside it, so they open the file with `TokenStore::open_read_only`:

- They use the access token the service keeps fresh, as long as it has more than 60 seconds left. The service renews it 300 seconds before expiry.
- They never refresh, save or remove the file. `set` and `clear` return an error.
- If the token is close to expiry, or the server turned it down with a 401, the file is read again in case the service has renewed it. If it hasn't, the command stops with "sign-in required" and asks for the service to be started (or for `axentra-onedrive login`). The file is left byte-for-byte as it was.

Why the CLI doesn't refresh the token itself: two processes refreshing one sign-in can save an older refresh token over a newer one. Both write `tokens.tmp` before renaming it, so one could rename the other's half-written file into place. A refused refresh also removes the file the service depends on. The service's writes are atomic renames, so a reader always sees a whole file.

Tests: `mirror::tests::the_onedrive_sign_in_is_only_read` (cli) and `a_read_only_sign_in_is_never_refreshed_here` (mock).

## Security controls

1. **Type-level allowlist.** `CliCommand` has only `Version`, `AuthLogin`, `List`, `Info`, `Size`, `CreateFolder`, `Upload` and `Download`. `FileConflict` has only `Skip` and `CreateNewRevision`, so there is no `replace` or `rename`. Downloads always use `skip`, never `remove`. Doctests prove that `FileConflict::Replace` and `CliCommand::Delete` do not compile.
2. **Runtime allowlist.** `CheckedArgv::new` is the only way to build what a runner executes. It enforces the following:
   - `--json` is required.
   - A `--` must come before any path.
   - Strategy values must match exactly. The CLI accepts unique prefixes, so `rep` would otherwise mean `replace`.
   - The `fs` alias and abbreviated commands are refused.
   - Paths outside `/my-files/` are refused.
   - Path segments shaped like node ids are refused.
   - Backslashes and control characters are refused, and so are glob characters (`*?[{`) in local paths.
3. **Process isolation.**
   - The binary runs directly with no shell.
   - stdin is `/dev/null` and the working folder is `/`.
   - The process gets its own process group, which is killed on a timeout or when the call is dropped.
   - Output is capped, and capped output is never trusted.
4. **Environment scrub.**
   - The child keeps only `HOME`, `USER`, `PATH`, `LANG`, `LC_*`, `XDG_*` and `DBUS_SESSION_BUS_ADDRESS`. `DISPLAY` and `WAYLAND_DISPLAY` are added for `auth login` only.
   - `PROTON_DRIVE_LOG_LEVEL=WARNING` is always set. The CLI's own default is DEBUG.
   - The command refuses to run at all if `PROTON_DRIVE_CREDENTIALS_STORE=unsafe_file` is set.
5. **Binary pin.** Before its first command, each destination checks the binary:
   - The SHA-512 must match `PINNED_SHA512`.
   - `version` must report `cli-drive@0.9.0`.
   - Anything else fails closed.
   - `proton login` runs the same check, then the same `CheckedArgv` and scrubbed environment as the mirror. The only difference is that its output goes straight to the terminal.
6. **Non-destructive policy.**
   - New files are uploaded with `-f skip`.
   - If the name is already taken, the claimed SHA-1 and size are compared. If they are equal, the existing file is adopted and recorded. If they differ, the file is marked `diverged` and both copies are left alone.
   - Content that changes on OneDrive after mirroring is marked `diverged`.
   - Deletes and moves on OneDrive are only recorded, never acted on.
7. **No secrets in argv.** The CLI handles the Proton session itself, in libsecret. OneDrive tokens never reach the child process.
8. **Fail closed.** An unknown CLI version, output that doesn't parse, an unknown node type, a newer manifest `user_version`, or a capped output all end in an error, never in success or "not found".

Tests that cover these controls:
- **CLI argv:** `plan_and_run_add_only_and_never_write_to_onedrive` (crates/cli) asserts that every argv the fake `proton-drive` received is an allowlisted read or add command, and that every upload used `--file-conflict-strategy skip`.
- **No Graph writes:** the same test asserts that no `DELETE`, `PATCH`, `PUT` or `POST` reached the OneDrive mock.
- **Destination unchanged:** after an edit, a rename and a delete on OneDrive, the same test asserts that the Proton tree is unchanged.

## Verification levels

Each file's level is recorded in the manifest exactly as reached and is never rounded up.

| Level | Meaning | What it proves | What it does not prove |
|---|---|---|---|
| **L0 planned** | In a plan; nothing transferred. | The file exists on OneDrive and is in scope. | Anything about Proton. |
| **L1 source-verified** | Staged bytes match OneDrive's QuickXorHash (and its SHA-1/SHA-256 when Graph gives them). The SHA-1 and SHA-256 are computed locally. | The bytes we hold are the bytes OneDrive has. | Anything about Proton. |
| **L2 uploaded** | `proton-drive upload` exited 0 and its JSON summary had `failures: []` and one transferred item. | The CLI says it stored a file under that name. | What content Proton holds. |
| **L3 remote-claimed** | Proton's `info` reports `claimedSize` and `claimedDigests.sha1` with `sha1Verified: true`, and they equal our size and SHA-1. | The signed metadata the uploader wrote matches our content. | The stored, encrypted blocks themselves. |
| **L4 round-trip** | The file was downloaded back through the CLI, which runs its signature and `IntegrityError` checks, and the SHA-256 of the download equals the source SHA-256. | The bytes Proton serves decrypt to exactly the source bytes. | Durability over time (it is a point-in-time check). |

A file can reach L4 without L3 if Proton reports no verified SHA-1. Same-named files without a verified SHA-1 are never adopted; they are marked diverged.

## Known limits

- **Whole-drive runs are refused** for now. `mirror run` needs `--only`.
- **No automatic runs yet.** `enabled` is reserved for the service's MirrorWorker (M5). There are no D-Bus methods and no GUI pages yet.
- **Unsupported file names.** File names containing `*`, `?`, `[` or `{` can't be uploaded through the CLI, which expands local paths as globs. They are planned as `skip_unsupported`.
- **Moves and renames on OneDrive** are recorded only. Files mirrored before the move stay at their old path on Proton, and files not mirrored before are uploaded under the new path. Duplicates can build up.
- **Deleted on Proton after mirroring.** Anything someone deletes on Proton afterwards is not re-created. It still shows as mirrored until a future verification pass.
- **Large files.** A file must fit in staging. Anything over `staging_cap_bytes` is skipped as `too_large`.
- **Session-expired wording.** The phrases for an expired Proton session come from Proton's account library, which is not published. They are a best guess, and anything unrecognised is reported as a non-retried error.
- **Thumbnails** are off (`--skip-thumbnails`), because a failed thumbnail fails the whole upload.
- **Telemetry.** The CLI is pre-1.0 and its binary is unsigned (only a SHA-512 is published). It identifies itself as `cli-drive`, and its telemetry follows the Proton account preference.
- **Proton's crypto migration** (expected late 2026 or early 2027) will need a new CLI version. That upgrade is a checkpoint, and the pin makes the mirror fail closed until it happens.
- **Expired OneDrive token.** A mirror command needs the sync service running, or a recent sign-in, because it never refreshes the OneDrive token itself.
