# Changelog

## 0.1.4 (pre-release, 2026-10-22)

- **One window per profile:** opening Axentra Outlook while it is already running (from Axentra Command, the
  application menu or a terminal) shows the open window instead of starting a second session on the same mail,
  and the second launch exits at once. On X11 the window comes to the front. On Wayland it comes to the front
  only when the launcher passes an activation token (GNOME's app grid and dash do); a launch from Axentra Command
  or a terminal makes GNOME show "Axentra Outlook is ready" instead, and a minimized window stays minimized until
  you click that. If the open copy has stopped responding, the launch says so after about 3 seconds instead of
  opening (it exits when you close the message). A launch while the app is closing (its Close button or the
  window manager's) opens it again once the old copy has gone, also when the old copy is short of CPU and
  takes seconds to close; a launch that showed the open window exits about 2 seconds later, after checking
  that the app stays. Two launches at once open one app and the other shows it, even when they use different
  runtime folders (`XDG_RUNTIME_DIR`): a lock file next to the database (`axentra.db.instance`, in
  `~/.local/share/axentra-outlook` for the default profile) lets one in, and the other waits up to a second
  for it, then up to 3 seconds for that copy to open the database, and says another copy is running (or that
  it is not responding). Before 0.1.4 both could open, or one failed with "database is locked". A crash never
  locks you out: the locks go with the process. A separate profile (`AXENTRA_DB`) still runs side by side.
  `--version` and `--help` print instead of opening; other arguments (files, `mailto:` links) are ignored with
  a note.
- **Upgrading:** quit Axentra Outlook before (or right after) installing 0.1.4. Older versions take no lock,
  so while one is still open 0.1.4 won't start beside it on the same mail; it tells you to close the old window.
  Once that window is closed, opening Axentra Outlook again opens it, even while the message is still up.
- **No more lost letters while typing:** fast typing, also on a busy machine, no longer drops characters in text
  fields (compose Subject, To, Cc and Bcc, event title, location, attendees and description, contact fields,
  task title, steps and notes, settings and dialog filters, searches). Fields the app empties (a picked
  suggestion, a chip just made, Escape in a search) are always emptied, and Ctrl+Z / Ctrl+Y (or Ctrl+Shift+Z)
  undo and redo in text fields, each field on its own (an undo never changes another field), one typing burst
  at a time, also in the message body and the signature editor. In a task title, a space typed before a pause
  stays (the autosave no longer trims the text you are typing, so the next word ran into the last).
- **Keys land where you typed them:** on a busy machine, keys typed just before a click into another field
  (a task title, then its note or a step; an event title, then its attendees) act in the field you typed them
  in, at its caret, never in the one you clicked, however far behind the app is and however many clicks came
  in between; also when the click opens another task (the title, note or step you were typing keeps them and
  is saved with them). That includes Ctrl+V (the paste goes into that field), Ctrl+Backspace and Ctrl+Delete
  (a word of that field) and Ctrl+Z / Ctrl+Y (that field's own undo); other keys meant for that field (Enter,
  arrows) are dropped rather than acted on elsewhere, and app shortcuts (Ctrl+S, Ctrl+1) work as always. Keys
  typed right after Enter in Search refine the
  search: they go back into the box with focus, so Space no longer opens a result; a key after a pause, or any
  other key first (an arrow, Tab), stays in the results, so Space, `?` and Shift+E work there; Backspace or
  Delete right after Enter edits the search (they archived or deleted the top message). Tab in the
  message body still moves on to Send, but text typed straight on (" world" after "Hello") goes back into the
  body instead of Space or Enter sending the unfinished message; after a pause, Space presses Send as before.
  A key typed right after the Escape that ends a search is kept in the box.
- **Addresses are never dropped:** an address typed in To, Cc or Bcc without Enter, Tab or `;` is now sent with
  Ctrl+Enter and Alt+S (as a click on Send always did) and kept in the draft by Esc, Ctrl+S, pop-out and leaving
  Mail; an invalid one becomes a red chip to fix. The Cc and Bcc buttons put the caret in the line they show.
  In events, an attendee typed or pasted but not added yet is saved with Ctrl+S, Ctrl+Enter or the Save/Send
  button (an invalid one stops the save with a red chip); a pasted list (`a@x; b@y`) becomes one attendee per
  address at once, so an address typed straight after it starts afresh; leaving the box with a pasted list adds
  every entry, invalid ones as red chips. A list pasted one address per line (a column from a sheet), or with
  only spaces between addresses, gives one recipient or attendee per address in compose and events alike,
  instead of one merged address that compose refused and an event saved as a single invalid attendee. The
  event's first toolbar button does what it said when you clicked it: Save with an attendee still in the box
  saves, with or without a title (it opened the attendee menu, or asked for a title as if sending). Leaving an
  address box adds the address only once the letters typed before the click have arrived, so a busy machine
  no longer cuts it short.
- **Quick entry:** in To Do (and the My Day pane), typing a task, pressing Enter and typing the next one right
  away gives separate tasks; the next title no longer lands behind the previous one. "Add a task" stays at the
  top of the list while it scrolls, so each task you add below the fold scrolls into view under it; rows
  passing behind it no longer show at its corners. Add step stays in view as the steps you add push it down
  the details pane. A task that couldn't be added stays in
  the box, selected, so the next one typed replaces it. In compose To, Cc and Bcc and in event attendees, an address typed right after Enter, Tab, `;` or
  `,` starts a new recipient instead of running into the last one, quick attendee entries no longer drop each
  other, and Enter or Tab picks the suggestion for what is in the box now, not for what was there a moment ago.
  Settings › Mail › Junk email takes blocked and safe senders the same way (type, Enter, type the next); an
  entry that isn't an address or domain stays selected, so the next one typed replaces it, and is marked with
  the reason under the box; typing no longer makes the box, its Add button and the next section jump.
- **Undo send and Ctrl+Z:** Ctrl+Z outside a text field takes back the most recent action wherever focus is
  (the message list, the reading pane, the app, another module), with one undo list for the whole app: while
  "Sending…" shows, that is the send, so the message goes back to Drafts and reopens instead of going out (it
  said "Nothing to undo", or undid an older archive, and sent it anyway). Ctrl+Z pressed right after Ctrl+Enter,
  even before the message has been queued (a busy machine) or with focus still in the message, takes the send
  back too, without undoing your typing in it. Flag and category changes are undone the same way, and an undo
  bar's Undo button and Ctrl+Z never undo the same action twice.
- **Account photo:** a Microsoft account (Outlook.com or Microsoft 365) shows its own profile photo instead of
  your initials on the header account button, in the account menu and in Settings › Accounts. It comes from
  Microsoft Graph with the `User.Read` permission already granted at sign-in (no new permissions, no new
  sign-in), is checked a few seconds after start and after sign-in, then at most every 6 hours, and is kept
  on this device only (not in backups; deleted with the account). IMAP and Gmail accounts keep initials.
- **Polish (dozens of fixes from four review rounds on the desktop engine):**
  - Read window: the subject sits in its title bar (and drags the window), the conversation no longer repeats
    it, printing starts at the top of the page, and a message with a blocked picture prints on one page. A
    press on its title bar focuses it, so Esc closes it; closing it with its Close button puts focus back on
    the message list, as Esc does.
  - Dialogs: a click on the backdrop or a window behind an open dialog leaves focus in the dialog, so Esc
    and Enter still work there; Shift+Tab stays inside dialogs and closes menus (WebKitGTK reports it as an
    unknown key).
  - Mail: long sender addresses ellipsize instead of running into the date; menus with submenus fit the
    window (taller ones scroll); Ctrl+Z after a delete or move no longer breaks the UI; "Open in new window"
    right after typing keeps the message; hovers are visible in the light theme; keyboard focus rings only the
    focused row; a long message prints on as many pages as it needs; search suggestions show only while you
    type in the box; pop-out windows open below the header with the system title bar too.
  - Calendar: tabbing through all-day events no longer scrolls the grid; "Discard changes?" opens on Cancel,
    so Enter after Esc keeps the event; time lists close on Escape; month view fits five weeks; Escape from
    quick create puts focus back on the grid; "Move recurring event" keeps Cancel on one row.
  - People and To Do: F6 skips empty panes; a task added below the fold scrolls into view; long steps,
    categories and due dates ellipsize. A task title, note or step you are typing is saved when you leave
    To Do with a shortcut (Ctrl+1, Ctrl+2), as is text typed in "Add step" or "Add a task" without Enter
    (added as Enter would), and a task completed just before leaving stays completed. Reminder cards no
    longer cover the end of a long menu (Change theme), and move beside an open side pane (Notifications, My
    Day, Help) instead of covering it. Ctrl+5 no longer draws focus-ring specks between the To Do panels.
    Switching lists while "Add a task" has focus, or opening another task within a second of renaming one,
    no longer closes the app (also in 0.1.2).
  - Shell and Settings: F6 follows the app order; header flyouts close when Tab leaves them; New contact and
    Keyboard shortcuts open at their normal height; settings search breadcrumbs name each level once; classic
    ribbon labels are no longer cut ("New contact" showed "New"); error text in the dark theme meets WCAG AA
    contrast (the junk sender box's "isn't an email address" line among them).
  - Compose: closing, discarding or popping out a message while one of its actions was still finishing could
    end the whole app; now it can't. Letters typed just before pop-out, Esc, Ctrl+S or Send land in the
    message first. Send (or Ctrl+Enter) followed at once by a module shortcut still sends, with the whole
    body; a message that needed you (no subject, say) is kept in Drafts with a note saying why. Leaving
    Mail right after typing keeps the last words of the body in the draft. Discarding or closing a popped-out
    message puts focus back on the message list (it was lost to the app).
  - People: "Discard changes?" for an edited new contact shows above the form, so Cancel, Esc and the close
    button work, Keep editing puts you back in the field you were editing, and Discard (also after Keep
    editing) puts focus back on the New contact button.

Source: [`source/`](source/) (snapshot of commit `fdf55ab`). See [release notes](releases/0.1.4.md).

## 0.1.3 (pre-release, 2026-10-20)

- **Resizable dialogs:** Settings and the other dialogs with real content (Add account, Restore from backup,
  Keyboard shortcuts, What's new, Customize ribbon, the move/copy folder picker, Sweep, Message details,
  Categories, Help, attachment preview, address book, drawing, Add calendar, Cancel meeting, the People
  new contact, contact list and import dialogs, To Do settings) resize from any edge or corner. Each has a
  minimum size and stays inside the window, below the title bar. Sizes are remembered per dialog and included
  in backups; double-click an edge to go back to the default size. Small confirmations and short forms keep a
  fixed size.
- **Movable pop-outs:** the compose pop-out and the read window (**Open in new window**) move by their title
  bar and remember their position and size. The read window has a slim title bar with its Close button.
- Contents follow the size: Settings' page area grows and its rows put the control under the label when
  narrow; lists, previews and the drawing canvas use the extra height; a width-only drag keeps short dialogs
  content-high.
- Fixed: the compose pop-out opened off-centre and partly cut off; the read window's menus could appear in the
  wrong place; attachment preview images keep their aspect ratio; Help dialog buttons no longer spill over; the
  IMAP server row in Add account no longer scrolls sideways; releasing a resize drag over the dimmed
  background never closes the dialog.

Source: [`source/`](source/) (snapshot of commit `fc2520f`). See [release notes](releases/0.1.3.md).

## 0.1.2 (pre-release, 2026-10-20)

- **Merged title bar:** the blue header is now the window's title bar, like the new Outlook for Windows. Minimize,
  maximize/restore and close follow your desktop's button layout; drag empty header space to move the window,
  double-click to maximize, resize from the edges and corners. Rounded corners when floating, square when
  maximized or tiled. **Settings › General › Appearance › Use the system title bar** brings the normal title bar
  back.
- Reviewed independently for window behaviour and layout; fixed before release: a click right after a drag
  being ignored, re-grabbing just after a drag maximizing the window, menus and dialogs covering the window
  buttons, and too little drag space at narrow widths.

Source: [`source/`](source/) (snapshot of commit `0f6f30e`). See [release notes](releases/0.1.2.md).

## 0.1.1 (pre-release, 2026-10-19)

- **Backup:** Settings › Accounts › Backup saves an encrypted backup (AES-256-GCM, Argon2id password key) of
  account setups, settings and device-only data to the OneDrive folder every month and on demand; restore
  previews the backup and merges without duplicates. Passwords and sign-in tokens are never included.
- The Microsoft client ID is remembered after the first sign-in, so it isn't needed again.
- Cross-module actions: Add to contacts from a message, Schedule meeting from a contact with the attendee
  filled in, open a task from My Day or reminders; toast actions and an Undo send countdown.
- Account status in the folder pane, Add account from the folder pane and account manager, RSVP responses sent
  to Microsoft 365, new Mail settings pages (attachment reminder, blocked and safe senders, sweep rules).
- Fixed: the message read window now refreshes when you open another message in release builds.

Source: [`source/`](source/) (snapshot of commit `da0c289`). See [release notes](releases/0.1.1.md).

## 0.1.0 (pre-release, 2026-10-19)

First release. Linux only (Zorin OS 18 / Ubuntu 24.04, x86_64).

- Outlook-style shell: header with search, app rail, Simplified and Classic ribbon with Customize, My Day,
  notifications, help, settings with search, Fluent 2 light/dark themes, colour themes, three densities.
- Mail: folder tree and favorites, Focused/Other, conversations, columns layout, multi-select, context menus,
  drag to folders, flags, categories, pin, snooze, junk and block, sweep, search keywords, Undo.
- Reading pane with sandboxed HTML, blocked remote images, attachments and meeting responses.
- Compose with recipient autocomplete, rich text, attachments, autosaved drafts, Schedule send and Undo send.
- Calendar with day / work week / week / month views, drag to create and move, recurrence, reminders.
- People with contact lists and vCard/CSV import/export; To Do with smart lists, steps and repeating tasks.
- Microsoft 365 / Outlook.com sync over Microsoft Graph; IMAP/SMTP accounts; Secret Service keyring storage.
- `.deb` package with menu entry, icons and an Axentra Command manifest.

Source: [`source/`](source/) (snapshot of commit `6926c2f`). See [release notes](releases/0.1.0.md).
