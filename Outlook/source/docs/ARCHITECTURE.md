# Axentra Outlook — architecture and working rules

A Linux desktop mail/calendar/people/to-do client in Rust that mirrors the
layout and behaviour of the new Outlook for Windows / Outlook on the web.
Behaviour and layout come from public documentation (`docs/spec/*.md`) and
measurements of the rendered web app (`docs/spec/reference-measurements.md`).
No Microsoft code, CSS, logos or product artwork is used. Icons are
Microsoft's MIT-licensed Fluent System Icons (`crates/axentra-ui/icons/`).

## Crates

| Crate | What it is |
|---|---|
| `axentra-core` | Domain model (`model.rs`), SQLite store (`store/`), recurrence (`recurrence.rs`), HTML sanitising (`html.rs`), text helpers, demo data (`seed/`), encrypted account backups (`backup/`). No UI, no network. |
| `axentra-sync` | Account connectivity (Microsoft Graph, IMAP/SMTP), runs on its own thread + Tokio runtime, writes into the store, exposes `SyncHandle`. |
| `axentra-ui` | Dioxus 0.7 components. Renders identically in the desktop webview and in liveview. |
| `axentra-desktop` | Binary `axentra-outlook`: opens `~/.local/share/axentra-outlook/axentra.db`, seeds demo data on first run (unless `AXENTRA_NO_DEMO`), starts sync, opens the window. |
| `axentra-preview` | Dev-only liveview server with an in-memory seeded store, for previewing in a browser: `AXENTRA_PREVIEW_PORT=8090 cargo run -p axentra-preview`, then open `http://localhost:8090/app`. |

## Store

* Every entity is a JSON document plus indexed columns (see `store/mod.rs`, trait `Doc`). Add model fields only with a serde default (`#[serde(default)]` on the struct or `Option`), never rename/remove.
* Each feature area owns one `impl Store` file: `mail.rs`, `drafts.rs`, `calendar.rs`, `people.rs`, `tasks.rs`, `sync.rs`. Extra tables/indexes go in that file's `SCHEMA_EXTRA` (idempotent `CREATE ... IF NOT EXISTS`).
* **Frozen API**: existing public `Store` methods keep their signatures and meaning; other areas call them. Improve the bodies, add new methods, but don't break callers. The list of what exists is the code itself (`cargo doc -p axentra-core`).
* The store is synchronous and cheap; call it directly from components. Never hold a store lock across `.await`.

## Backups

`core/src/backup/` writes and restores encrypted backups (Settings › Accounts ›
Backup, `ui/src/settings/backup/`): one `.axobk` file per backup in
`<OneDrive folder>/Axentra/Outlook Backups/`, Argon2id + AES-256-GCM over
deflated JSON. It holds account setups (never secrets), settings and only the
data that exists on no server. Rules for other areas:

* Store keys (`kv`) are backed up only when listed in
  `backup::KV_ALLOWLIST` (or `ACCOUNT_KV` for `<prefix><account id>…` keys).
  Add a user-preference key there if it should survive a restore; never add
  sync state or anything secret. A listed value that refers to ids (lists,
  accounts) must be rewritten in `restore.rs` (`remap_ids`).
* New tables that hold device-only user data need a line in `collect.rs` and
  `restore.rs` to be included.
* For synced accounts an item counts as device-only when it has neither a
  `remote_id` nor a `sync_items` row, so the sync engine must record every
  item it pulls or pushes there.

Backups are written into the Axentra OneDrive mount point only while its FUSE
mount is live, retention only deletes files this device wrote, and restored
drafts never keep a send schedule. `AXENTRA_BACKUP_DIR` overrides the
destination (development and tests).

## UI state and reactivity

`state.rs` defines `AppState` (Copy, in context; get it with `use_app()`):
`module`, `settings`, revision signals (`mail_rev`, `calendar_rev`, `people_rev`, `tasks_rev`, `accounts_rev`), `toasts`, `compose`, `compose_cmd`, `nav`, `search`, `settings_dialog`, `my_day_open`.

* The store isn't reactive. A component that reads store data calls the matching `app.mail_rev()` (etc.) **before** querying, and every mutation is followed by `app.bump_mail()` (etc.). That is the whole reactivity model.
* Settings: read `app.settings.read()`, change with `app.update_settings(|s| ...)` (persists).
* Feedback: `app.toast(..)`, `app.toast_with_undo(text, UndoAction::...)`, `app.toast_with_action(text, label, ToastAction::...)` (a custom button such as "Open folder"), `app.toast_undo_send(text, undo, until)` (the Undo send bar: spinner and a bar that drains until `until`), `app.error(..)`, `app.check(result)`.
* Cross-module: `app.navigate(NavRequest::...)` switches module (`NavRequest::module()`) and leaves a request in `app.nav` that the target module consumes (and clears) — Mail handles `Folder`/`Message`; Calendar `Event`/`CalendarDate`/`NewEvent`/`NewEventWith` (with attendees); People `Contact`/`NewContact` (pre-filled New contact form); To Do `TaskList`/`SmartList`/`Task` (the task's list with the detail pane open).
* Drag data between modules: Mail rows put `application/x-axentra-message-ids` (`[1,2]`) on drags; To Do rows put `application/x-axentra-task-ids`.
* Compose: `app.compose(ComposeRequest::...)` (switches to Mail). While compose is open the Mail ribbon shows `mail::compose::ribbon_tabs()` and forwards picks with `app.compose_command(id)`; `ComposePane` consumes `app.compose_cmd`.
* Search: the header box writes `app.search` (`query`, `active`, `serial`). Each module interprets it for itself (Mail: message search; Calendar: events; People: contacts; To Do: tasks).
* Don't add fields to `AppState`; keep module-local state in the module (signals, or a module context provided at the module root).

## Components (`kit/`, `shell/ribbon.rs`)

* `Icon`, `Button`, `IconButton`, `MenuButton`, `Menu` + `MenuItem` (builder), `Avatar`, `Checkbox`, `Switch`, `TextField`, `Select`, `Pivot`, `Dialog`, `EmptyState`, `Spinner`, `CategoryPill`, `category_colors()`.
* `Ribbon { tabs: Vec<RibbonTab>, on_command, on_toggle_nav?, nav_open }` with `Command` builder (`.split(menu)`, `.menu(menu)`, `.primary()`, `.disabled()`, `.toggled()`, `.icon_only()`). Each module renders its own ribbon at the top of its area.
* `Dialog`: focus trap, initial focus (`initial_focus` selector, else `autofocus`, else the first field, else the default button), Enter presses the default button (`on_default`, or `Button { default_action: true }`) unless focus is on a control that handles Enter itself, and focus returns to the opener on close. `MenuItem::tooltip(..)` explains disabled items; menus with submenus don't scroll (so submenus aren't clipped).
* Props of existing components are a contract: add optional props, don't rename/remove.
* Keyboard: `kit::keys::chord(&e)` gives `"Ctrl+Shift+R"`-style strings. **Every editable element must call `kit::keys::stop_typing_keys(&e)` in its `onkeydown`** so typing never triggers single-key shortcuts. Module containers handle their module's shortcuts; the shell root handles global ones.
* Icons: add an SVG to `crates/axentra-ui/icons/<name>.svg` (Fluent 20px regular/filled from github.com/microsoft/fluentui-system-icons, with `fill="currentColor"` and no width/height attributes) — `build.rs` registers it.

## Styling

* One stylesheet per area in `crates/axentra-ui/styles/`, concatenated in `lib.rs::CSS`. Use only the `--ax-*` tokens from `tokens.css` (light + dark + `system` via media query). Token **names** are frozen; values may be tuned by the shell owner.
* Class prefix: `ax-` for shared classes; module classes use a module prefix (`ax-mail-…`, `ax-cal-…`, `ax-people-…`, `ax-todo-…`, `ax-compose-…`, `ax-acct-…`).
* Theme classes on the root: `ax-theme-{system|light|dark}`, density `ax-density-{roomy|cozy|compact}`.
* Email HTML is always rendered in a sandboxed `iframe` (`sandbox=""` or `"allow-popups"` only, `srcdoc`) after `axentra_core::html::sanitize_email_html`; remote images blocked unless the user allows them.

## Ownership (parallel work)

| Area | Owns (may edit) |
|---|---|
| Shell | `ui/src/shell/**`, `ui/src/kit/**`, `ui/src/settings/mod.rs` and any new `ui/src/settings/*.rs` except `accounts.rs`, `styles/{tokens,base,kit,shell,settings}.css`, `crates/axentra-desktop/**` |
| Mail | `ui/src/mail/**` except `ui/src/mail/compose/**`, `styles/mail.css`, `core/src/store/mail.rs`, `core/src/seed/mail.rs` |
| Compose | `ui/src/mail/compose/**`, `styles/compose.css`, `core/src/store/drafts.rs` |
| Calendar | `ui/src/calendar/**`, `styles/calendar.css`, `core/src/store/calendar.rs`, `core/src/recurrence.rs`, `core/src/seed/calendar.rs` |
| People | `ui/src/people/**`, `styles/people.css`, `core/src/store/people.rs`, `core/src/seed/people.rs` |
| To Do | `ui/src/tasks/**`, `styles/tasks.css`, `core/src/store/tasks.rs`, `core/src/seed/tasks.rs` |
| Sync | `crates/axentra-sync/**`, `ui/src/settings/accounts.rs`, `styles/accounts.css`, `core/src/store/sync.rs` |

Shared files nobody edits without coordination: `core/src/model.rs` (append-only fields with serde defaults are OK), `core/src/store/mod.rs`, `ui/src/state.rs`, `ui/src/lib.rs`, `Cargo.toml` (workspace). New dependencies go in your own crate's `Cargo.toml` only, pinned to an exact version that's at least a few weeks old.

## Definition of done for a change

1. `cargo build --workspace` and `cargo test --workspace` pass with no warnings in files you own; `cargo clippy --workspace` clean for your files.
2. Store logic you add has unit tests.
3. You looked at the result in the preview server (your own port and your own browser tab).
4. Light, dark and all three densities look right; keyboard paths work; empty and error states exist.
