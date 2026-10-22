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
| `axentra-desktop` | Binary `axentra-outlook`: makes sure only one copy runs per database (see Single instance), opens `~/.local/share/axentra-outlook/axentra.db`, seeds demo data on first run (unless `AXENTRA_NO_DEMO`), starts sync, opens the window (undecorated: the header is the title bar, see below). |
| `axentra-preview` | Dev-only liveview server with an in-memory seeded store, for previewing in a browser: `AXENTRA_PREVIEW_PORT=8090 cargo run -p axentra-preview`, then open `http://localhost:8090/app`. `AXENTRA_FAKE_WINDOW_CONTROLS=1` (or a GNOME button layout such as `close,minimize,maximize:`) draws the desktop title bar with no-op window controls; `AXENTRA_FAKE_WINDOW_TILED` (e.g. `top bottom left`) starts it tiled. `AXENTRA_FAKE_ACCOUNT_PHOTO=1` gives the demo account a drawn, synthetic profile photo. |

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
* Server data stays out: the `sync_*` tables and `account_photos` (each
  Microsoft account's own profile photo, fetched again after a restore).
* For synced accounts an item counts as device-only when it has neither a
  `remote_id` nor a `sync_items` row, so the sync engine must record every
  item it pulls or pushes there.

Backups are written into the Axentra OneDrive mount point only while its FUSE
mount is live, retention only deletes files this device wrote, and restored
drafts never keep a send schedule. `AXENTRA_BACKUP_DIR` overrides the
destination (development and tests).

## UI state and reactivity

`state.rs` defines `AppState` (Copy, in context; get it with `use_app()`):
`module`, `settings`, revision signals (`mail_rev`, `calendar_rev`, `people_rev`, `tasks_rev`, `accounts_rev`), `account_photos`, `toasts`, `compose`, `compose_cmd`, `nav`, `search`, `settings_dialog`, `my_day_open`.

* The store isn't reactive. A component that reads store data calls the matching `app.mail_rev()` (etc.) **before** querying, and every mutation is followed by `app.bump_mail()` (etc.). That is the whole reactivity model.
* Settings: read `app.settings.read()`, change with `app.update_settings(|s| ...)` (persists).
* Own-account avatars: wherever one of the user's own accounts is drawn (header, Account Manager, Settings › Accounts), pass `photo: app.account_photo(id)` to `kit::Avatar`. The map is read from the store at start and on every `bump_accounts` (the sync engine bumps when a photo arrives, changes or goes). Sender, contact and attendee avatars never take it.
* Feedback: `app.toast(..)`, `app.toast_with_undo(text, UndoAction::...)`, `app.toast_with_action(text, label, ToastAction::...)` (a custom button such as "Open folder"), `app.toast_undo_send(text, undo, until)` (the Undo send bar: spinner and a bar that drains until `until`), `app.error(..)`, `app.check(result)`. Ctrl+Z has one stack for the whole app (`AppState::undo_last`, newest first): every undo bar puts its action on it, `app.push_undo(undo)` adds one without a bar (a flag or category change), and the held send of an Undo send bar stays on it until the message goes out (`app.forget_undo(toast)`), so Ctrl+Z outside a text field takes back the most recent action wherever focus is (`compose::send_queue::undo`). A send still reading its body (`SendHandoff`) is taken back before it goes, and in the form itself, after the app took Ctrl+Enter or Alt+S, the page sends Ctrl+Z there instead of undoing typing in the message about to go out (`SEND_UNDO_JS`).
* Cross-module: `app.navigate(NavRequest::...)` switches module (`NavRequest::module()`) and leaves a request in `app.nav` that the target module consumes (and clears) — Mail handles `Folder`/`Message`; Calendar `Event`/`CalendarDate`/`NewEvent`/`NewEventWith` (with attendees); People `Contact`/`NewContact` (pre-filled New contact form); To Do `TaskList`/`SmartList`/`Task` (the task's list with the detail pane open).
* Drag data between modules: Mail rows put `application/x-axentra-message-ids` (`[1,2]`) on drags; To Do rows put `application/x-axentra-task-ids`.
* Compose: `app.compose(ComposeRequest::...)` (switches to Mail). While compose is open the Mail ribbon shows `mail::compose::ribbon_tabs()` and forwards picks with `app.compose_command(id)`; `ComposePane` consumes `app.compose_cmd`.
* Search: the header box writes `app.search` (`query`, `active`, `serial`). Each module interprets it for itself (Mail: message search; Calendar: events; People: contacts; To Do: tasks).
* Don't add fields to `AppState`; keep module-local state in the module (signals, or a module context provided at the module root).

## Single instance

`axentra-desktop/src/instance.rs` runs first in `main()`, before the store
opens. The database path in use (`AXENTRA_DB` or the default), made absolute
and canonical as far as it exists (a profile folder not created yet keys the
same as once it is), is hashed into a private runtime directory
`$XDG_RUNTIME_DIR/axentra-outlook-<hash>/` (0700, checked for owner and
symlinks; `/tmp/axentra-<uid>/` without a usable `XDG_RUNTIME_DIR`). The first
launch holds an `flock` on `instance.lock` there for its lifetime (a crash
releases it) and binds the datagram socket `show` (0600) at once; if the
socket can't be bound it keeps the lock anyway. A later launch on the same
database sends `show` (plus `XDG_ACTIVATION_TOKEN` or `DESKTOP_STARTUP_ID`
when its launcher set one) from an abstract socket of its own, without
blocking, and waits up to 3 s for the answer:

* `shown` (the GTK thread restored, raised and focused the main window) or
  `starting` (the main loop or the window is not up yet; it shows when it
  is): the launch exits 0 after `SHOWN_GRACE` (2 s), unless within it the
  app lets go of the lock or sends `closing` after all (it answered before
  it got to a Close click already on its way, which a busy machine can
  take seconds over); then it goes on as for `closing`.
* `closing` (the app's close was asked for, `instance::closing`, from its
  Close button or the window manager's close; or the window has gone): the
  launch waits up to 10 s for the lock and runs the app itself. An app that
  has begun closing answers this without presenting its window, and sends it
  to the launches it told `shown` in the last 2 s. When another launch takes
  the lock first (it binds a new `show` socket), this one asks that launch
  instead, so two launches at once end with one app.
* no answer (the app is frozen, its queue is full, or it has no socket): the
  launch says the app is not responding (stderr and a dialog) and exits 1,
  rather than open the database twice. If the socket was replaced while it
  waited (the app asked exited without answering and another launch took the
  lock), it asks the new holder instead; if the lock came free (a request
  queued on an app busy closing is never answered), it runs the app at once.

The runtime directory is used by its canonical path, so a launch whose
`XDG_RUNTIME_DIR` goes through a symlink still recognises the answer (the
app reports the path it bound).

The running app answers from the GTK thread (a glib idle source), so an
answer means its main loop is alive. On Wayland it activates with the token
through xdg-activation; without a token GNOME only shows "… is ready" (and a
minimized window stays minimized: xdg-shell has no way to unminimize), and
when a token turns out stale the window is presented again without it so the
notice still appears. On X11 it uses the startup id and a fresh server
timestamp. Another database (a test profile) is another key and runs side
by side.

Every launch also takes an `flock` on `<db>.instance`, a file next to the
database's real path (`~/.local/share/axentra-outlook/axentra.db.instance`
for the default profile; created by 0.1.4 and left in place), and holds it
for its lifetime. This catches two launches at the same instant with
different runtime directories, which take different `instance.lock` files and
neither of which has the database open yet when the other looks: the one that
finds `<db>.instance` taken waits up to 1 s for it (a copy that is quitting
lets go as it exits), then lets go of its own lock and polls for up to 3 s for
the holder to show up with the database open, and is refused like any other
copy (or says the app is not responding). When the file can't be used (a
read-only folder) the other checks stand alone.

The lock only guards copies that take it. Before the store opens,
`instance::other_copies` looks through `/proc` (this user's processes named
`axentra-outlook…`, read only) for one that has the same database open: a
copy older than 0.1.4 (still running after an upgrade; its program file
shows as deleted) or one whose lock is elsewhere (another
`XDG_RUNTIME_DIR`). The check runs while the launch holds the lock but
before it answers anyone (`instance::start`). If there is one, the launch
lets go of the lock, then says so in a dialog and exits 1: a launch made
while that dialog is up (the other copy closed, as it says) runs the app
rather than being answered "starting" by a process that has no window and
is about to exit. When the runtime directory can't be used at all, the app
warns on stderr and runs with only that check.

## Window frame (client-side decorations)

The desktop window has no native decorations; the header is the title bar
(`docs/spec/shell.md` §3.1, §12.4). `axentra-ui` never depends on
`dioxus-desktop`: the host provides a `shell::window::WindowControls`
context (Copy: `minimize`, `toggle_maximize`, `close`, `start_drag`,
`start_resize(ResizeEdge)` callbacks, `is_maximized` and `tiled`
(`TiledEdges`) signals, the `ButtonLayout` and the double/middle-click
`TitlebarAction`s).

* `axentra-desktop/src/titlebar.rs` reads the desktop's preferences once at
  startup (GNOME `org.gnome.desktop.wm.preferences`, or GTK's
  `gtk-decoration-layout` elsewhere; `AXENTRA_BUTTON_LAYOUT` overrides) and
  wraps `axentra_ui::App` in `CsdRoot`, which provides the context from the
  tao window and keeps `is_maximized` and `tiled` in sync with GDK's
  window-state events (tao reports no tiling; resize events are a fallback
  for maximise).
* With the context, the shell draws caption buttons on the side the desktop
  puts them, marks empty header space with `data-ax-drag-region`, adds
  invisible resize handles (`window::ResizeHandles`) and sets
  `data-window="restored|tiled|maximized"` on `.ax-app` (rounded frame;
  square with no frame line or handles on the `data-tiled` edges; square
  and edge to edge). The caption buttons come last in the DOM (tab order
  after the avatar), CSS places the left-side group. Modal overlays dim
  through a clipped `::before` and menu/flyout backdrops are clipped the
  same way, so caption buttons stay clickable under them.
  Mouse handling is a page script (`window::TITLEBAR_JS`): a drag starts after
  4px of movement, so clicks and double-clicks stay with the page; only the
  moving press's own mouse click is swallowed, and a press after a move
  never counts as a double-click.
* Without it (liveview, Settings › General › Appearance › "Use the system
  title bar", `AXENTRA_SYSTEM_TITLEBAR=1`) the header is a plain app header.
  Don't put interactive content on `data-ax-drag-region` elements themselves;
  children that are buttons or inputs are never drag handles.

## Components (`kit/`, `shell/ribbon.rs`)

* `Icon`, `Button`, `IconButton`, `MenuButton`, `Menu` + `MenuItem` (builder), `Avatar` (optional `photo`: `data:image/` URLs only; initials otherwise, and when the photo fails to load), `Checkbox`, `Switch`, `TextField`, `Select`, `Pivot`, `Dialog`, `EmptyState`, `Spinner`, `CategoryPill`, `category_colors()`.
* `Ribbon { tabs: Vec<RibbonTab>, on_command, on_toggle_nav?, nav_open }` with `Command` builder (`.split(menu)`, `.menu(menu)`, `.primary()`, `.disabled()`, `.toggled()`, `.icon_only()`). Each module renders its own ribbon at the top of its area.
* `Dialog`: focus trap, initial focus (`initial_focus` selector, else `autofocus`, else the first field, else the default button), Enter presses the default button (`on_default`, or `Button { default_action: true }`) unless focus is on a control that handles Enter itself, and focus returns to the opener on close. `MenuItem::tooltip(..)` explains disabled items; menus with submenus don't scroll (so submenus aren't clipped).
* Resizable surfaces (`kit::surface`): `Dialog { resize_key: "contact-form", min_width, min_height, .. }` (and `AcctDialog`) makes a dialog resizable from eight invisible grips; `width`/`height` become the default size. In-app windows use `use_surface` + `SurfaceGrips` with `SurfaceSpec::movable()` and mark their title area `data-ax-surface-move` (compose pop-out, read window). The page script (`dom.js`) does the dragging; CSS (`.ax-surface` in `kit.css`) keeps surfaces inside the window minus 16px (below the title bar in CSD mode), so they stay clamped when the window shrinks. Sizes (and pop-out positions) are saved per key in the store entry `shell.surfaces`; modal dialogs reopen centred at the saved size; double-clicking a grip goes back to the default. A gesture keeps the sides it doesn't drag as saved (`surface::kept`): moving a pop-out in a small window doesn't shrink it, and dragging only the width of a content-height dialog (no `height`) keeps it content-high and vertically centred. A resized dialog gives its extra height to the child marked `ax-surface-fill` (a direct child of the body, or of another fill); other "when resized" rules are anchored as `.ax-dialog[data-ax-sized] > .ax-dialog-body ...` so they don't reach dialogs opened inside a resized one. Give every dialog with substantial content (lists, forms, previews) a key; small confirmations stay fixed.
* Props of existing components are a contract: add optional props, don't rename/remove.
* Text fields: keep the controlled `value:`; `kit/typed_text.js` (installed with the page runtime) skips the late echoes of earlier keys that renders write back, and `kit::dom::TypedTextSync` (mounted by the shell) settles the doubtful ones. A field that a key commits and empties says so, and the page empties it as the key goes down, before the next key can land behind the old text: `data-ax-enter-clears="true"` for Enter (only when the handler took the key with `prevent_default()`, so a failed add keeps the text), `data-ax-address-list="split"` (compose wells: Enter, Tab, a separator typed at the end) or `"keys"` (attendees: Enter, Tab, `;`/`,` taken with `prevent_default()`, and a pasted list, which the app adds whole). Keys pressed in a text field before a click but delivered after it (a busy page) act in that field, never in the one clicked: each click's press time (`event.timeStamp`) goes on a short focus timeline with the text field that had focus up to it, and each key goes by its own press time, with no time limit (characters, Backspace and Delete; Ctrl+Backspace and Ctrl+Delete delete a word there; Ctrl+Z and Ctrl+Y use that field's own history; a late Ctrl+V's paste event, which reads the clipboard, is turned into an insert there; other plain keys and caret chords are dropped, app shortcuts work wherever focus is); keys typed straight on after Enter in `data-ax-enter-keeps-typing` (the search box) or after Tab out of `data-ax-tab-keeps-typing` (the message body) go back too. An address box commits what is typed on blur only after `kit::dom::keys_landed`, and a form that saves or sends must first add text still typed in its address boxes (compose `recipients::commit_typed`, calendar `fields::commit_typed`). A task that reads a component's own signals after a delay runs as that component's task (`kit::after`), never on a module root (`ctx.spawn`, `kit::spawn_in`), which outlives it. Handlers that can run several times before the next render (Enter in a chip well) must build on live state (a signal or `CopyValue`), not on a list captured by the render.
* Tasks that must outlive the control that started them (a menu, a dialog) run in an ancestor's scope with `kit::spawn_in` (`Form::spawn`, `TodoCtx::spawn`), so they are dropped with that ancestor. Never `Runtime::spawn(scope, ..)`: in dioxus-core 0.7 it doesn't tie the task to the scope, and the task wakes up after the component has gone and panics.
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
