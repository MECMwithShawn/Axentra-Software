# Axentra Outlook — Keyboard shortcuts and accessibility

Status: build specification, area "Keyboard shortcuts and accessibility".
Target: Rust + Dioxus desktop (HTML/CSS rendered in WebKitGTK on Linux).
Behavioural reference: the **new Outlook for Windows / Outlook on the web** (Fluent 2 era, 2024–2026), with notes where **classic Outlook for Windows** differs.

---

## 0. Conventions used in this document

### 0.1 Key notation

| Notation | Meaning |
|---|---|
| `Ctrl+R` | Chord: hold Ctrl, press R. |
| `Alt, H` | Sequence: press and release Alt, then press H (key-tip style). |
| `G then I` | Two-key sequence without modifiers (web/Gmail styles). Second key must arrive within 1500 ms (estimate). |
| `Ctrl+>` | On a US layout this is physically `Ctrl+Shift+.` (see §2.2 for layout matching). |
| `?` | On a US layout this is `Shift+/`. Microsoft lists it as `?` in new Outlook's frequently-used table and as `Shift+?` in the People tables (new Outlook and web) and the web set chooser; it is the same physical key press. |
| `Menu` | The keyboard context-menu key (`ContextMenu` in DOM `key`). |

Microsoft documents its shortcut tables against the US keyboard layout; Axentra does the same and adapts per §2.2.

### 0.2 Source tags

Every shortcut row carries a source tag. Anything not traceable to public documentation is additionally tagged **(estimate)**.

| Tag | Meaning |
|---|---|
| **NO** | Documented by Microsoft for the new Outlook for Windows (support.microsoft.com shortcut page, screen-reader guides, or Microsoft accessibility blog). |
| **WEB** | Documented by Microsoft for Outlook on the web / Outlook.com. |
| **CL** | Documented by Microsoft for classic Outlook for Windows. |
| **3P** | Reputable third-party write-up (Robert Sparnaaij, How-To Geek, Slipstick, etc.). |
| **TD** | Documented for the Microsoft To Do app. |
| **APG** | W3C WAI-ARIA Authoring Practices Guide pattern (a standards recommendation, not a claim about Outlook). |
| **AX** | Axentra design decision (fills a documentation gap; not a claim about Outlook). |

**(unverified)** marks a claim that a fact-check pass could not trace to a public source (as opposed to **(estimate)**, which marks a deliberate Axentra guess). Fact-check pass: 2026-10-19, against the live Microsoft support pages listed under Sources.

---

## 1. Keyboard shortcut set ("mode") setting

### 1.1 What Microsoft offers

| Product | Settings path | Options | Source |
|---|---|---|---|
| Outlook on the web / Outlook.com | Settings → General → Accessibility → Keyboard shortcuts, then **Save**. (Classic OWA: Settings → Mail → General → Keyboard shortcuts.) | Outlook.com; Yahoo Mail; Gmail; Outlook; or turn shortcuts off. Shift+? shows the list for the chosen set | WEB (support page lists the four sets, the off switch, both paths and the Save step); 3P (How-To Geek) |
| New Outlook for Windows | Settings → General → Accessibility → Keyboard shortcuts (Save step per 3P tutorial; not in Microsoft docs) | 3P wording: "Outlook for Windows" (default), "Outlook for Web", or turn off keyboard shortcuts for Mail, Calendar and People. Exact on-screen labels **(unverified)**. A Tech Community thread shows a build whose Accessibility page had only a "Turn off keyboard shortcuts" checkbox plus a line saying Outlook for Windows-style shortcuts are selected, with a link back to Settings | 3P (Eleven Forum tutorial, Microsoft Q&A, Tech Community thread); Microsoft blog confirms only that the shortcut list is reachable from Settings → General → Accessibility. Microsoft's shortcut page does not describe the set chooser for new Outlook at all |
| Classic Outlook for Windows | No set chooser (fixed shortcut set). Only related toggle: File → Options → Mail → Send messages → "CTRL + ENTER sends a message" | None. A first-use prompt appears the first time Ctrl+Enter is pressed in a draft | CL / 3P (Slipstick) |

Facts worth preserving:
- No Outlook client lets a user remap an individual command; the only granularity is "pick a set" or "turn all off" (3P; Tech Community).
- Turning shortcuts off is the documented way users stop accidental Ctrl+Enter sends in the new Outlook (Tech Community thread). "Undo send" (up to 10 s) is the other mitigation.
- OWA's set labelled just "Outlook" is the desktop-style set; it also accepts most single-key Outlook.com keys (3P, Robert Sparnaaij comparison table: N, R, Q, V and G-then-I work in both; search is Alt+Q in "Outlook" style and / in "Outlook.com" style).
- Microsoft's published web table already merges both styles ("Ctrl+R or R", "Ctrl+Q or Q"), so Axentra's single "Outlook for the web" set (below) folds OWA's separate "Outlook.com" set into it. Axentra does not offer a separate "Outlook.com" option (AX).

### 1.2 Axentra setting

Location: **Settings → General → Accessibility → Keyboard shortcuts** (mirrors new Outlook). Control: a radio group (`role="radiogroup"`, label "Keyboard shortcuts") followed by a **Save** / **Discard** button pair at the bottom of the settings page, consistent with the rest of the Settings dialog.

| Order | Label (Axentra wording; Microsoft's exact strings unverified) | Internal id | Behaviour | Default |
|---|---|---|---|---|
| 1 | Outlook for Windows | `KeymapMode::Windows` | Table set A (§3–§9, "Win" column). Ctrl-based; no bare letter keys except `?` and folder type-ahead. Includes classic-compatible aliases marked CL-alias. | **Yes** (AX; matches the new Outlook default as reported by 3P tutorials — Microsoft does not document the default) |
| 2 | Outlook for the web (label (unverified); 3P tutorials write "Outlook for Web") | `KeymapMode::Web` | Table set B ("Web" column): Ctrl+Shift+digit module switching, Shift+Alt+digit calendar views, Alt+Q search, plus bare-letter keys (N, R, Shift+R, Shift+F, E, Q, U, V …). Win-mode Ctrl keys that do not collide remain active. | No |
| 3 | Gmail | `KeymapMode::Gmail` | Web set + Gmail overlay (§11.2). Phase 2. | No |
| 4 | Yahoo Mail | `KeymapMode::Yahoo` | Web set + Yahoo overlay (§11.3). Phase 2. | No |
| 5 | Turn off keyboard shortcuts | `KeymapMode::Off` | Disables application shortcuts such as Ctrl+Enter send and Ctrl+N. Keyboard navigation keeps working, and so do Delete and Shift+Delete in lists (see the lists below). | No |

Under the radio group, a link button: **View keyboard shortcuts** → opens the shortcuts panel (§13) filtered to the selected set.

What `Off` keeps working (these are platform/accessibility navigation, not "shortcuts") (AX):
- Tab, Shift+Tab, arrow keys, Home/End, PageUp/PageDown inside composite widgets.
- Enter, Space, Escape semantics (§15).
- F6 / Shift+F6 / Ctrl+F6 region cycling (needed for keyboard-only access; WCAG 2.1.1).
- Shift+F10 / Menu context menus; Alt key tips (key tips are UI chrome for the ribbon, like menu mnemonics).
- Text editing inside fields: Ctrl+C/X/V/Z/Y/A, Ctrl+Backspace, Ctrl+B/I/U in the compose body.
- Delete and Shift+Delete in the message list **remain active** (estimate; Microsoft does not say whether Off disables them — see Open questions).
- Disabled under Off: Ctrl+Enter send, Ctrl+N, Ctrl+R/Ctrl+Shift+R/Ctrl+F, Ctrl+Q/Ctrl+U, Insert, Ctrl+digit module switching, Ctrl+E search, every bare-letter key.

Persistence: one value per Axentra profile (not per account), stored with other app settings; applies immediately on Save (no restart). Changing the set never clears the undo stack or focus.

---

## 2. Key handling rules (engine-level)

### 2.1 Scope precedence

A key event is resolved against scopes from innermost to outermost; the first match wins and calls `preventDefault()` + `stopPropagation()`.

1. Open modal dialog / alert dialog (focus is trapped; only dialog keys + Esc + text editing).
2. Open menu, popover, combobox list, date-picker popup.
3. Key-tip mode (after Alt is released, §5.4).
4. Focused text input / contenteditable (compose body, subject, recipients, search box, inline rename).
5. Focused composite widget (folder tree, message list, calendar grid, ribbon toolbar, app bar).
6. Region scope (e.g. reading pane, calendar surface).
7. Module scope (Mail, Calendar, People, To Do).
8. Global scope (window).

Consequences engineers must implement:
- `Ctrl+U` = underline in the compose body (scope 4) but mark-unread in the message list (scope 5) — both documented by Microsoft (NO).
- `Ctrl+Y` = redo in text (scope 4) but go-to-folder-pane elsewhere (NO; Microsoft's table words the text meaning as "repeat most recent action" and lists both meanings without scoping them).
- `Ctrl+>` (next message) is physically `Ctrl+Shift+.` on a US layout, the same chord as classic's `Ctrl+Shift+>` "increase font size". In the compose body (scope 4) it changes font size; elsewhere it moves to the next message. Same for `Ctrl+<` / `Ctrl+Shift+<` (AX).
- `Ctrl+Space` = clear formatting in the compose body (CL) but toggle selection in the message list (CL / APG).
- `Ctrl+T`, `Ctrl+L`, `Ctrl+D` are module-scoped: indent / align-left in the compose body (CL); new list / new contact list (To Do / People); mark complete (To Do) vs delete contact (People) (TD / NO).
- `Ctrl+E`, `Ctrl+L`, `Ctrl+R` inside the compose body = paragraph alignment (CL-parity, estimate); outside = search / (People) new contact list / reply.
- `Ctrl+Shift+V` in compose = paste as plain text (AX, WebKit convention); in the message list = move to folder (CL-alias).
- `Ctrl+K` in a recipient field = resolve/check names (CL); in the body = insert hyperlink (NO).
- Bare-letter keys (Web/Gmail/Yahoo sets) and `?` are **never** active in scope 4, and never while `KeyboardEvent.isComposing` is true (IME).

### 2.2 Matching keys across layouts

Dioxus exposes `evt.key()` (logical, layout-aware), `evt.code()` (physical), `evt.modifiers()`, `evt.is_auto_repeating()` and `evt.is_composing()` (keyboard-types). Checked against `dioxus-html` 0.7.10 `KeyboardData`: `key()` returns the `keyboard_types::Key` enum (printable keys arrive as `Key::Character(String)`), `code()` returns `Code`, `location()` is also available, and `modifiers()` comes from the `ModifiersInteraction` trait, which must be in scope (the Dioxus prelude is expected to re-export it — (unverified)). Pin the Dioxus version in `Cargo.toml`; these accessors have moved between releases.

| Binding type | Match on | Reason |
|---|---|---|
| Letters (`Ctrl+R`, `N`, `Q`) | `key` lower-cased if it is an ASCII letter; otherwise fall back to `code` (`KeyR`) | AZERTY/Dvorak users get the letter printed on the key; Cyrillic/Greek layouts still work. |
| Digits (`Ctrl+1`, `Ctrl+Shift+2`, `Alt+3`, `Ctrl+Alt+4`) | `code` (`Digit1`…`Digit9`, `Digit0`, also `Numpad1`…) | Shifted digits produce `!`, `@` etc.; AZERTY digits are shifted. |
| `?` | `key == "?"` (any Shift state) | Location differs across layouts. |
| `Ctrl+>` / `Ctrl+<` | `key` is `>`/`<` with Ctrl, **or** `code` `Period`/`Comma` with Ctrl+Shift | Covers US and layouts where `<`/`>` has its own key. |
| `Alt+-`, `Alt+=` | `code` `Minus` / `Equal` | |
| Function and navigation keys | `key` (`F6`, `Delete`, `Insert`, `ArrowDown` …) | |

### 2.3 Auto-repeat

- Allowed: arrow keys, PageUp/PageDown, Home/End, Tab, Ctrl+>/Ctrl+<, Delete (each repeat acts on the newly focused item).
- Ignored when `is_auto_repeating()`: Ctrl+Enter, Alt+S, Ctrl+N, Ctrl+R, Ctrl+Shift+R, Ctrl+F, Shift+Enter, Shift+Delete, Ctrl+P, module switches, every bare-letter command (AX).

### 2.4 Linux desktop conflicts (AX)

| Keys | Typical owner | Axentra handling |
|---|---|---|
| Ctrl+Alt+Left/Right (/Up/Down) | GNOME workspace switching (default binding on many installs) | Keep the documented binding; also expose Previous/Next toolbar buttons and the Alt+Up/Down (week) and Alt+PageUp/PageDown (month) bindings, which the WM does not take. Show a one-time hint in the calendar if Ctrl+Alt+Arrow never reaches the app (estimate). |
| Alt+F4, Alt+Tab, Alt+Space, Alt+F7/F8/F10 | Window manager | Never bind. |
| Ctrl+Q | Many GTK apps quit on it | Axentra binds Ctrl+Q to **mark as read** (NO). There is no quit accelerator other than the window close button / app menu. |
| Insert | Orca's default "Orca modifier" on desktop layout | Flag is also reachable via the context menu and key tips; consider Ctrl+Shift+G (CL "Flag for follow up" dialog) as an alias (estimate). |
| Ctrl+Shift+I | wry/WebKit developer tools in debug builds | Disable devtools accelerator in release builds; Ctrl+Shift+I is used for Go to Inbox (CL-alias). |
| F10 | GTK menubar activation | Axentra creates no GTK menubar; no conflict. |
| Bare letters | Orca browse mode intercepts letters for structural navigation | Composite widgets (`listbox`, `tree`, `grid`) put Orca into focus mode, so letters reach the app there. Do not expect bare letters to work while the reading-pane document is in browse mode. |

---

## 3. Global and navigation shortcuts

| Action | Win (default) | Web | Scope | Src |
|---|---|---|---|---|
| Go to Mail | Ctrl+1 | Ctrl+Shift+1 | Main window | NO / WEB |
| Go to Calendar | Ctrl+2 | Ctrl+Shift+2 | Main window | NO / WEB |
| Go to app-bar position 3 | Ctrl+3 | Ctrl+Shift+3 | Main window | NO / WEB (Microsoft: Copilot is #3); AX: position-based |
| Go to app-bar position 4 | Ctrl+4 | Ctrl+Shift+4 | Main window | NO / WEB (Microsoft: People is #4) |
| Go to app-bar positions 5–9 | Ctrl+5 … Ctrl+9 | Ctrl+Shift+5 … 9 | Main window | 3P (shortcuts follow app-bar order) — (estimate) |
| Next region | F6 (also Ctrl+F6) | Ctrl+F6 (also F6) | Any window | NO (F6 in mail guide; Ctrl+F6 in new-Outlook calendar guide) / WEB (Ctrl+F6); F6 in the Web set is AX |
| Previous region | Shift+F6 (also Ctrl+Shift+F6) | Ctrl+Shift+F6 (also Shift+F6) | Any window | NO / WEB |
| Move to a different region (alternate) | Ctrl+Shift+Tab → previous region | — | Any window | NO (listed for Calendar; direction not stated → estimate) |
| Show key tips on the ribbon | Alt (press and release) | Alt+. (period) moves focus to ribbon | Main and pop-out windows | NO / WEB |
| Go to Home tab / View tab | Alt, H / Alt, V | Alt+H / Alt+V | Ribbon | NO / WEB |
| Open a split button's menu | Alt+Down | Alt+Down | Focused split button | NO / WEB |
| Show keyboard shortcuts panel | ? | Shift+? | Not in text fields | NO / WEB |
| Go to Search | Ctrl+E (aliases: F3 CL, Alt+Q (estimate for new Outlook)) | Alt+Q (alias "/" — 3P "Outlook.com style") | Main window | NO (Microsoft's new-Outlook table lists Ctrl+E only as People "search for contact"; mail-wide Ctrl+E is CL and (unverified) for new Outlook) / WEB / CL |
| Open context menu for focused item | Shift+F10 or Menu | Shift+F10 or Menu | Focused item | WEB / CL |
| Undo last action | Ctrl+Z | Ctrl+Z | Text: undo typing. Elsewhere: undo last mailbox action (AX) | NO / WEB |
| Redo / repeat | Ctrl+Y | Ctrl+Y | Text only | NO / WEB |
| Go to folder pane | Ctrl+Y | — | Mail, not in text | NO |
| New item for current module | Ctrl+N | N | Mail → message, Calendar → event, People → contact, To Do → task | NO / WEB / TD |
| New message from any module | Ctrl+Shift+M (CL-alias) | — | Main window | CL — (estimate for new Outlook) |
| Print current item / view | Ctrl+P | Ctrl+P | Item or calendar view | NO (calendar guide) / CL |
| Sync now ("Send/Receive") | F9 (CL-alias; Ctrl+M also CL) | — | Main window | CL — (estimate for new Outlook) |
| Close menu / dialog / pop-out item | Esc | Esc | Topmost transient | NO |
| Close pop-out window | Alt+F4 (window manager) | Alt+F4 | Pop-out | NO |
| Zoom app content in / out / reset | Ctrl+= / Ctrl+- / Ctrl+0 | same | Main window | AX (estimate) |
| Open Help pane | F1 | F1 | Main window | AX (estimate) |

**App-bar position rule (AX):** Microsoft keeps Mail at position 1 and Calendar at 2 (fixed, cannot be unpinned) and maps the remaining Ctrl+digit keys to the current app-bar order (3P). Microsoft's own pages disagree on the mapping, which supports a position-based rule: the current shortcut page gives Ctrl+3 = Copilot and Ctrl+4 = People (new Outlook, web, and classic), while the new-Outlook calendar screen-reader guide still says Ctrl+1 to Ctrl+4 open Mail, Calendar, Contacts and To Do. Axentra has no Copilot, so its default rail is Mail (1), Calendar (2), People (3), To Do (4); reordering pinned modules re-maps Ctrl+3…Ctrl+9. The shortcuts panel and tooltips must render the live mapping.

---

## 4. Mail shortcuts

### 4.1 Folder pane (navigation pane)

| Action | Win | Web | Src |
|---|---|---|---|
| Move focus to folder pane | Ctrl+Y | (F6 / Ctrl+F6) | NO |
| Previous / next folder | Up / Down | Up / Down | NO (guide) / CL |
| Expand node (on collapsed) / move to first child (on expanded) | Right | Right | NO / WEB (expand); APG (child) |
| Collapse node (on expanded) / move to parent (on collapsed or leaf) | Left | Left | NO / WEB (collapse); APG (parent) |
| First / last visible node | Home / End | Home / End | AX (APG tree) |
| Expand all siblings | `*` | `*` | AX (APG tree) |
| Jump to folder by name (type-ahead) | Any letter/digit; multi-char buffer, resets after 500 ms idle (estimate). From anywhere: Ctrl+Y, then type (the blog describes Ctrl+Y as "folder jumping" by typed characters) | Same | NO (shortcut table + blog) |
| Open folder (load list) | Enter or Space | Enter or Space | NO (Enter) / CL (Space) |
| Rename folder | F2 | F2 | CL — (estimate for new Outlook) |
| New subfolder | Ctrl+Shift+E | — | CL — (estimate for new Outlook) |
| Delete folder (confirmation; default folders not deletable) | Delete | Delete | CL (via context menu) — (estimate for Delete key) |
| Folder context menu | Shift+F10 / Menu | Shift+F10 / Menu | CL / WEB |

Arrow keys move focus only; the folder opens on Enter/Space (AX — prevents loading every folder while arrowing). Classic Outlook moves focus to the message list on Enter/Space; Axentra keeps focus in the tree (estimate) and the user presses F6.

### 4.2 Message list and reading pane (both)

| Action | Win | Web | Src |
|---|---|---|---|
| Delete (to Deleted Items) | Delete | Delete | NO / WEB |
| Permanently delete (confirmation, §16) | Shift+Delete | Shift+Delete | NO / WEB |
| Mark as read | Ctrl+Q | Ctrl+Q or Q | NO / WEB |
| Mark as unread | Ctrl+U | Ctrl+U or U | NO / WEB |
| Flag / mark flagged item complete | Insert | Insert | NO / WEB |
| Archive | Backspace (CL-alias; Microsoft documents Backspace archiving for classic only — the new-Outlook instructions mention only the Archive button) | E | WEB / CL (Archive support article) |
| Move to folder (opens folder picker) | Ctrl+Shift+V (CL-alias) | V | CL / 3P — (estimate for new Outlook) |
| Copy to folder | Ctrl+Shift+Y (CL-alias) | — | CL — (estimate) |
| Reply | Ctrl+R | Ctrl+R or R | NO / WEB |
| Reply all | Ctrl+Shift+R | Ctrl+Shift+R or Shift+R | NO / WEB |
| Forward | Ctrl+F | Ctrl+Shift+F or Shift+F | NO / WEB |
| Forward as attachment | Ctrl+Alt+F (CL-alias) | — | CL — (estimate) |
| Reply with meeting | Ctrl+Alt+R (CL-alias) | — | CL — (estimate) |
| Open selected message in new window | Shift+Enter | Shift+Enter | NO / WEB |
| Next message | Ctrl+> (alias Ctrl+. CL) | Ctrl+. (estimate) | NO / CL |
| Previous message | Ctrl+< (alias Ctrl+, CL) | Ctrl+, (estimate) | NO / CL |
| Cancel search | Esc | Esc | NO / WEB |
| Go to Inbox | Ctrl+Shift+I (CL-alias) | G then I | CL / 3P |
| Print | Ctrl+P | Ctrl+P | CL — (estimate for new Outlook mail) |

### 4.3 Message list navigation and multi-select

The list is a single composite widget with roving focus (§14). "Focus" = the item with `tabindex=0`; "selection" = items with `aria-selected=true`.

| Action | Keys | Src |
|---|---|---|
| Move focus and select (single) | Up / Down | NO (guide) / CL |
| Extend contiguous selection | Shift+Up / Shift+Down | NO / WEB |
| Extend to first / last loaded item | Ctrl+Shift+Home / Ctrl+Shift+End (APG listbox/tree, optional keys); also Shift+Home / Shift+End (AX, Windows list-view convention — APG does not list these) | APG / AX |
| Select range from anchor to focused item | Shift+Space | APG (optional) |
| Extend by a page | Shift+PageUp / Shift+PageDown | AX |
| First message in folder | Home or Ctrl+Home | NO / WEB |
| Last loaded message (triggers loading the next page; announce "Loading more messages") | End or Ctrl+End | AX (estimate) |
| Move by visible page | PageUp / PageDown | CL |
| Move focus without changing selection | Ctrl+Up / Ctrl+Down | APG (alternative selection model); classic list-view parity (unverified — not on Microsoft's Outlook shortcut page) |
| Toggle selection of focused item | Ctrl+Space (also Space) | APG (Ctrl+Space in the alternative model, Space in the recommended model) |
| Select all loaded items | Ctrl+A | NO (People) / APG (optional) — (estimate for mail) |
| Clear multi-selection (keep focused item selected) | Esc | NO (People "unselect all") — (estimate for mail) |
| Expand conversation / collapse conversation | Right / Left | CL (groups) / AX |
| Move into expanded conversation's child messages | Down after Right | AX |
| Open message (reading pane off → full-height reading view; on → move focus to reading pane) | Enter | NO (guide) — focus move is (estimate) |
| Leave the list | Tab / Shift+Tab (exits composite) | APG |

Multi-select rules (AX):
- Ctrl+click toggles; Shift+click extends from the anchor; the anchor is the last item selected without Shift.
- Hover check circles in each row toggle selection with the mouse; they are `aria-hidden` and not focusable (keyboard uses Space).
- With ≥2 selected, the reading pane shows a "N items selected" state with bulk actions; announce "N selected" politely after the selection settles (debounce 300 ms, estimate).
- Commands (delete, move, flag, read/unread, categorize, archive) act on the selection; if nothing is selected, on the focused item.
- Ctrl+A in a folder with unloaded items shows an info bar "All N loaded conversations are selected. Select all N in this folder" (wording estimate); bulk ops on the whole folder go through the server-side batch path.

### 4.4 Reading pane / opened message

| Action | Keys | Src |
|---|---|---|
| Top / bottom of message or conversation | Home / End (Web also Ctrl+Home / Ctrl+End) | NO / WEB |
| Page up / down | PageUp / PageDown | NO / WEB |
| Page down / up (alternative) | Space / Shift+Space | CL |
| Move between sections (header, attachments, body, inline actions) | Tab / Shift+Tab | NO (blog: Tab moves between sections) |
| Move within a group (e.g. action buttons, attachment cards) | Arrow keys | NO (blog) |
| Close opened message, return to list | Esc | NO (guide) |
| Close inline reply / new message | Esc | NO / WEB |
| Reply / Reply all / Forward / Send / New | Ctrl+R / Ctrl+Shift+R / Ctrl+F / Ctrl+Enter / Ctrl+N | NO |

---

## 5. Compose

### 5.1 Commands

| Action | Win | Web | Src |
|---|---|---|---|
| New message | Ctrl+N (Ctrl+Shift+M alias) | N | NO / WEB / CL |
| Send | Ctrl+Enter (alias Alt+S) | Ctrl+Enter (alias Alt+S) | NO / WEB (Alt+S documented for meeting forms) / CL |
| Save draft | Ctrl+S | Ctrl+S | CL (Ctrl+S saves an item). WEB lists Ctrl+S only for calendar forms and contacts; new Outlook lists it only for contacts — (estimate for mail drafts in both) |
| Close compose (draft is kept automatically) | Esc | Esc | NO / WEB; autosave behaviour (estimate) |
| Close compose window | Alt+F4 | — | NO |
| Attach file | Alt (or F6) to reach the ribbon → Alt+N or Right arrow to the Insert tab → Down to "Attach file" button → Enter → a menu offers "Browse this computer" (local file) or OneDrive (link) → pick a file. Axentra shows only "Browse this computer" (GTK file chooser via the xdg FileChooser portal) unless a cloud provider is connected (AX). Axentra key tips: Alt, N, A, F | same | NO (basic-tasks guide) / CL (Alt+N, A, F (unverified)) |
| Insert hyperlink | Ctrl+K (body) | Ctrl+K | NO / WEB |
| Check names | Ctrl+K (recipient field) | — | CL |
| Spelling and grammar | F7 | — | Office/Word-editor convention; not on Microsoft's Outlook shortcut page for any version — (unverified) |
| Show Insert / Format text / Options tab | Alt, N / Alt, O / Alt, P | — | NO (Alt+N); others (estimate) |

Ctrl+Enter safety (AX): Axentra does **not** copy classic's first-use prompt; it relies on "Undo send" (if enabled in Mail settings) and the "Turn off keyboard shortcuts" option, as the new Outlook does.

### 5.2 Text editing and formatting (body)

| Action | Keys | Src |
|---|---|---|
| Copy / Cut / Paste | Ctrl+C / Ctrl+X / Ctrl+V | NO |
| Paste as plain text | Ctrl+Shift+V | AX |
| Delete character left / word left | Backspace / Ctrl+Backspace | NO |
| Undo / Redo | Ctrl+Z / Ctrl+Y (also Ctrl+Shift+Z, AX Linux convention — see Open questions) | NO / AX |
| Toggle insert/overwrite | Insert (body only) | WEB |
| Bold / Italic / Underline | Ctrl+B / Ctrl+I / Ctrl+U | NO |
| Increase / decrease font size | Ctrl+Shift+> or Ctrl+] / Ctrl+Shift+< or Ctrl+[ | CL — (estimate for new Outlook) |
| Center / Left / Right align | Ctrl+E / Ctrl+L / Ctrl+R | CL — (estimate) |
| Justify | Ctrl+Shift+J | CL — (estimate) |
| Bulleted list | Ctrl+Shift+L | CL — (estimate) |
| Increase / decrease indent | Ctrl+T / Ctrl+Shift+T | CL — (estimate) |
| Clear formatting | Ctrl+Space (classic also lists Ctrl+Shift+Z; Axentra gives Ctrl+Shift+Z to redo instead — see Open questions) | CL — (estimate for new Outlook) |
| Leave the body (to ribbon) | F6 / Shift+F6 | NO |

Tab inside the body moves focus out (to the next field/control) by default so keyboard users are never trapped; Ctrl+Tab… is not used. Inside a list item, Tab/Shift+Tab indent/outdent the list level instead (AX, estimate). Document this in the shortcuts panel.

### 5.3 Field order and recipient wells

Tab order in a new message (NO guide, extended by AX): **From** (only with multiple accounts) → **To** → **Cc** → **Bcc** (when shown) → **Subject** ("Add a subject") → **Message body** → attachment well (if any) → **Send** split button. Microsoft's guide documents only To → Cc → Subject → body, with focus starting in **To** after Ctrl+N; the From and Bcc positions are AX.

| Context | Keys → behaviour | Src |
|---|---|---|
| Typing in To/Cc/Bcc | Suggestions popup opens; Down/Up browse; Enter or Tab adds the highlighted contact as a pill; Esc closes the popup | NO (Down + Enter) / AX (Tab, Esc) |
| `;` or `,` after a typed address | Commits it as a pill | CL parity (estimate) |
| Backspace in empty input | First press focuses the last pill, second press removes it | AX (estimate) |
| Left in empty input | Focus last pill; Left/Right move between pills | AX |
| Delete / Backspace on focused pill | Remove pill, focus moves to next pill or input | AX |
| Enter on focused pill | Opens the recipient's contact card | AX (estimate) |
| Ctrl+K in recipient input | Resolve typed names against directory | CL |

Announcements: entering a field reads its label ("To, edit"; "Cc, edit"; "Add a subject"; "Message body" — NO guide). Adding a pill announces "Added {display name}" politely (AX).

---

## 6. Calendar

### 6.1 Commands and views

| Action | Win | Web | Src |
|---|---|---|---|
| Go to Calendar | Ctrl+2 | Ctrl+Shift+2 | NO / WEB |
| New event / invite attendees | Ctrl+N (aliases Ctrl+Shift+A appointment, Ctrl+Shift+Q meeting) | N | NO / WEB / CL |
| Open selected event | Enter | Enter | NO / WEB |
| Delete selected event | Delete | Delete | NO / WEB |
| Day view | Ctrl+Alt+1 | Shift+Alt+1 | NO / WEB |
| Work week view | Ctrl+Alt+2 | Shift+Alt+2 | NO / WEB |
| Week view | Ctrl+Alt+3 or Alt+- | Shift+Alt+3 | CL (Ctrl+Alt+3); NO (Alt+-) — see note A |
| Month view | Ctrl+Alt+4 or Alt+= | Shift+Alt+4 | NO / WEB |
| Three-day view | Alt+3 | — | NO |
| N-day view starting today (1–7) | Alt+1 … Alt+7 | — | NO (basic-tasks calendar screen-reader guide; not in the shortcut table); classic allows Alt+1…Alt+9 and Alt+0 = 10 days (CL) |
| Go to today | Alt+Shift+Y or Home | Alt+Shift+Y | NO / WEB |
| Next / previous time period | Ctrl+Alt+Right / Ctrl+Alt+Left | Shift+Right / Shift+Left | NO / WEB |
| Next / previous day | Ctrl+Alt+Right / Ctrl+Alt+Left (Day view) | same | NO / WEB |
| Next / previous week | Alt+Down / Alt+Up | Alt+Down / Alt+Up | CL (Down = next). NO **and** WEB tables both say the opposite (Down = previous) — see note B |
| Next / previous month | Alt+PageDown / Alt+PageUp | same | CL (PageDown = next). NO and WEB tables both say the opposite — see note B |
| Go to date (opens date dialog) | Ctrl+G | — | CL — (estimate for new Outlook) |
| Move to a different region | F6 or Ctrl+Shift+Tab (also Ctrl+F6, which in new Outlook announces a "Jump to …" target such as date selection; Enter jumps) | Ctrl+F6 or Ctrl+Shift+F6 | NO (shortcut table; Ctrl+F6 from the calendar navigation guide) / WEB |
| Next / previous event or area | Tab / Shift+Tab | Tab / Shift+Tab | NO / WEB |
| Print | Ctrl+P (opens print preview) | Ctrl+P | NO (guide) / WEB |

Note A: Microsoft's new-Outlook table lists Ctrl+Alt+2 for both Work week and Full week (Full week also gets Alt+-). This looks like a documentation slip: the web table uses Shift+Alt+2 for Work week and Shift+Alt+**3** for Full week, and classic uses Ctrl+Alt+2 / Ctrl+Alt+**3**, so the digit pattern points to 3 for Full week. Axentra keeps Ctrl+Alt+3 for Week plus the documented Alt+- (unverified against the live app).
Note B: Both modern Microsoft tables (new Outlook **and** Outlook on the web) pair Alt+Down / Alt+PageDown with "previous" and Alt+Up / Alt+PageUp with "next". Only classic documents the opposite (Alt+Down = next week, same weekday; Alt+PageDown = next month). Two modern tables agreeing with each other (they appear to share one source) is weaker evidence than it looks, but it is the current documentation. Axentra uses classic semantics (Down/PageDown = later) because they match the arrow direction in the grid and the mini-month's PageDown; a product owner should confirm by pressing the keys in a live new-Outlook or OWA session (hands-on use, not decompiling). Flagged in Open questions.
Note C: In classic, Left/Right in the grid and Ctrl+Left/Ctrl+Right both move by one day (CL); the new-Outlook and web tables use Ctrl+Alt+Left/Right for previous/next day in Day view and as "previous/next time period".

### 6.2 Event form (appointment / meeting)

| Action | Keys | Src |
|---|---|---|
| Save appointment | Ctrl+S | WEB |
| Send meeting | Ctrl+Enter or Alt+S | WEB |
| Close without saving | Esc (inline/web) / Alt+F4 (pop-out window) | WEB / NO |
| Set up recurrence | Ctrl+G (in form) | CL — (estimate) |
| Scheduling assistant | Alt+X | CL — (estimate) |

Axentra adds a "Discard changes?" confirmation on Esc when the form is dirty (AX, estimate — OWA's guide says Esc exits the composer without saving; the new-Outlook guide says Esc closes the Details window when nothing was changed). First field on open: Title, announced "Add a title" (WEB guide says "Add a title for the event"). Microsoft's screen-reader guides describe sending an edited meeting as Shift+Tab to Send then Enter, and saving an appointment as Shift+Tab to Save then Space — so Send/Save must be reachable by Shift+Tab from the first field (NO / WEB).

### 6.3 Calendar grid keyboard model (AX; Microsoft's accessibility blog of 6 September 2024 announced arrow-key navigation of calendar time slots for an October 2024 rollout, without publishing the key map)

Time grid (Day, Work week, Week, N-day):

| Keys | Behaviour |
|---|---|
| Up / Down | Move one time slot (slot length = calendar time scale; default 30 min, estimate). From the first slot, Up moves to the all-day row. |
| Left / Right | Same time, previous / next day column. Past the first/last visible day, pages the view by one period. |
| Shift+Up / Shift+Down | Extend the slot selection (classic: change block duration). |
| Ctrl+Home / Ctrl+End | Start / end of working hours on the focused day (classic Home/End semantics; estimate). |
| PageUp / PageDown | Scroll one viewport of hours, moving focus by the same amount. |
| Home | Go to today (NO). |
| Enter on empty slot(s) | Open quick-create popover pre-filled with the selected range. |
| Enter on event | Open event (peek/details). |
| Tab / Shift+Tab | Next / previous event in chronological order across the visible range (NO). Tab past the last event leaves the grid. |
| Delete on event | Delete flow (§16.3). |
| Shift+F10 / Menu on event | Event context menu (Accept, Tentative, Decline, Categorize, Delete…). |

Month view: arrows move by day / week; Home / End = first / last day of the focused week; PageUp / PageDown = same weekday previous / next month (CL month-view keys); Enter on a day opens quick-create for that day; Tab enters the events listed in the focused day.

Mini month (date picker in the navigation pane): APG date-picker grid keys — arrows by day/week, PageUp/PageDown by month, Shift+PageUp/Shift+PageDown by year, Home/End start/end of week, Enter/Space selects the date and navigates the main view.

Screen reader text for events: "{title}, {start time} to {end time}, {date}, {location}, organizer {name}, {response status}" (field order (unverified): the current new-Outlook calendar navigation guide only says browsing starts by announcing the current date, whether it has events, and the month's event count; it does not give a per-event field order). Slots: "{weekday} {date}, {start} to {end}, no events" or "… {n} events".

---

## 7. People

| Action | Win | Web | Src |
|---|---|---|---|
| Go to People | Ctrl+3 (app-bar position 3 in Axentra; Microsoft documents Ctrl+4 because Copilot is #3) | Ctrl+Shift+3 (Microsoft: Ctrl+Shift+4) | NO / WEB / AX |
| Create contact | Ctrl+N | Shift+N | NO / WEB |
| Create contact list | Ctrl+L | Shift+L | NO / WEB |
| Edit contact or list | Shift+E | Shift+E | NO / WEB |
| Save contact or list | Ctrl+S | Ctrl+S | NO / WEB |
| Discard edits / unselect all | Esc | Esc | NO / WEB |
| Delete contact or list | Delete or Ctrl+D | Delete | NO / WEB |
| Search contacts | Ctrl+E | Alt+Q | NO / WEB |
| Select all | Ctrl+A | Ctrl+A | NO / WEB |
| Move in list | Up / Down | Up / Down | NO / WEB |
| Add contacts to selection | Shift+Up/Down, then Space | same | WEB |
| Toggle checkbox | Space | Space | WEB |
| Show People shortcuts | Shift+? | Shift+? | NO / WEB |
| Home tab / View tab | Alt, H / Alt, V | Alt+H / Alt+V | NO / WEB |
| Ribbon | Alt | Alt+. | NO / WEB |
| Context menu | Shift+F10 | Shift+F10 | WEB |
| Clear selection and refresh | Shift+F5 | Shift+F5 | WEB |
| Close dialog | Esc | Esc | WEB |

---

## 8. To Do

Microsoft documents these for the standalone To Do app (its page says "Microsoft To Do on Windows 10" and gives no web-specific list); the new Outlook embeds To Do but does not publish separate bindings, so all rows are (estimate) for the Outlook-embedded module.

| Action | Keys (both sets) | Src |
|---|---|---|
| New task (focus "Add a task" input) | Ctrl+N (Web set: N) | TD — (estimate) |
| New list | Ctrl+L | TD — (estimate) |
| Add focused task to My Day | Ctrl+T | TD — (estimate) |
| Mark focused task complete / incomplete | Ctrl+D (alias Insert, CL tasks) | TD documents Ctrl+D for "complete" only; the toggle back to incomplete and the Insert alias are (estimate) |
| Search tasks | Ctrl+E (AX, for consistency with Mail/People) and Ctrl+F (TD-documented, active inside To Do only) | AX / TD |
| Paste | Ctrl+V | TD (listed; what pasting does outside a text field is not described — (unverified)) |
| Sync now | F9 (TD uses Ctrl+R; not adopted because Ctrl+R = reply) | AX |
| Commit "Add a task" input | Enter (task created; input stays focused for the next task) | AX (estimate) |
| Open task details pane | Enter on task | AX |
| Close details pane (focus returns to task) | Esc | AX |
| Delete task (confirmation) | Delete | AX (estimate) |

TD's Ctrl+1 (open the sidebar in minified view) is not adopted because Ctrl+1 switches modules.

---

## 9. Search

| Action | Win | Web | Src |
|---|---|---|---|
| Focus search box (announced "Search, edit" — JAWS wording; Narrator adds "Banner", NVDA says "Search landmark") | Ctrl+E (aliases F3, Alt+Q) | Alt+Q (alias /) | NO (People table; mail guide confirms the announcements) / WEB / CL / 3P |
| Browse suggestions | Down / Up | Down / Up | (unverified) — not described in the current navigation guides |
| Run search | Enter | Enter | AX |
| Close suggestions → second press clears query and exits search | Esc, Esc | Esc, Esc | NO / WEB ("Esc cancels search" in the shortcut tables) — two-step behaviour (estimate) |
| Exit search (button left of the box) | Shift+Tab to "Exit search" → Enter | same | (unverified) — not in the current new-Outlook navigation guide |
| Search scope: current folder / all items of the current module ("All Mail Items" in Mail) / include subfolders | Ctrl+Alt+K / Ctrl+Alt+A / Ctrl+Alt+Z | — | CL — (estimate for new Outlook). Ctrl+Alt+A is **not** "all mailboxes" |
| Advanced search panel | Ctrl+Shift+F | — | CL — (estimate) |

Results behaviour: when results arrive, focus moves to the **first result item** (NO blog) and a polite announcement reads "{n} results" or "No results" (AX). Suggestion groups: All, Mail, Files, Events, People (unverified — not found in the current Microsoft guides; treat as AX).

---

## 10. Classic Outlook equivalents (only where they differ)

| Action | New Outlook (Win set) | Classic Outlook | Notes |
|---|---|---|---|
| New message | Ctrl+N | Ctrl+N (in Mail) / Ctrl+Shift+M (anywhere) | Axentra keeps Ctrl+Shift+M as alias |
| Send | Ctrl+Enter | Alt+S; Ctrl+Enter only if enabled (first-use prompt) | |
| Open message in its own window | Shift+Enter | Ctrl+O or Enter | |
| Next / previous message | Ctrl+> / Ctrl+< | Ctrl+. / Ctrl+, (also Alt+Up / Alt+PageUp for previous in Reading pane) | Axentra accepts both |
| Search | Ctrl+E | Ctrl+E or F3 | F3 alias kept |
| Go to Folder pane | Ctrl+Y | Ctrl+Y = "Go to folder" dialog. (Ctrl+6 was Folder list in older builds; the current classic table maps Ctrl+6 to Org Explorer — see next row) | |
| Archive | (Backspace alias) | Backspace archives selected messages in the Reading-pane view (not from an opened message window); admins can disable it with the `DisableOneClickArchive` policy | CL (Archive support article) |
| Save item | Ctrl+S (documented only for contacts in new Outlook; estimate elsewhere) | Ctrl+S | |
| Module switching | Ctrl+1 Mail, Ctrl+2 Calendar, Ctrl+3 Copilot, Ctrl+4 People (rail order) | Ctrl+1 Mail, 2 Calendar, 3 Copilot, 4 People, 5 To Do, 6 Org Explorer, 7 Loop, 8 Tasks (rail order in recent builds) | |
| Key tips | Alt | Alt or F6 (ribbon), Alt+Q = Tell Me | |
| Region cycling | F6 / Shift+F6 | F6 / Tab between small panes; Ctrl+Shift+Tab or Shift+Tab between Folder pane, main view, Reading pane, To-Do bar | |
| Move to folder | (Ctrl+Shift+V alias) | Ctrl+Shift+V | |
| Copy to folder | (alias) | Ctrl+Shift+Y | |
| Flag dialog | — | Ctrl+Shift+G | |
| Check for new mail | (F9 alias) | F9 or Ctrl+M; Shift+F9 = download full items for the current folder | |
| Address book | — | Ctrl+Shift+B | |
| Go to Inbox / Outbox | (Ctrl+Shift+I alias) | Ctrl+Shift+I / Ctrl+Shift+O | |
| Delete and ignore conversation | — | Ctrl+Shift+D | |
| Mark as not junk | — | Ctrl+Alt+J | |
| Forward as attachment | (alias) | Ctrl+Alt+F | |
| Advanced Find | — | Ctrl+Shift+F | |
| Week / Work week views | Ctrl+Alt+2 (both, per doc) / Alt+- | Ctrl+Alt+3 week, Ctrl+Alt+2 work week, Alt+- week | |
| Schedule view | — | Ctrl+Alt+5 | |
| Today | Alt+Shift+Y or Home | Alt, H, O, D | |
| Calendar previous / next day | Ctrl+Alt+Left / Right | Ctrl+Left / Ctrl+Right (or Left/Right in grid) | |
| Calendar next / previous week, month | Alt+Down / Alt+Up, Alt+PageDown / Alt+PageUp (classic semantics kept, §6.1 note B) | Alt+Down = next week, Alt+Up = previous; Alt+PageDown = next month, Alt+PageUp = previous | New-Outlook and web tables document the reverse |
| Quick Steps | — (not implemented) | User-assignable shortcut keys (believed to be Ctrl+Shift+1…9 — (unverified); not on Microsoft's shortcut page) | See Open questions |
| New appointment / meeting | Ctrl+N | Ctrl+Shift+A / Ctrl+Shift+Q | Aliases kept |
| New task / contact / contact group | Ctrl+N in module | Ctrl+Shift+K / Ctrl+Shift+C / Ctrl+Shift+L | |
| Find in item | — | F4 (Shift+F4 next) / Ctrl+H replace | |
| Save As | — | F12 | |
| Item properties | — | Alt+Enter | |
| Reminders window | — | Alt+S snooze, Alt+D dismiss, Alt+O open | Axentra uses these inside its reminder window (estimate) |

---

## 11. Alternate sets

### 11.1 "Outlook for the web" set — bare-letter keys (summary)

Active only outside text fields. All rows WEB unless tagged.

| Key | Action | Key | Action |
|---|---|---|---|
| N | New message / event | E | Archive |
| R | Reply | Shift+R | Reply all |
| Shift+F | Forward | Q / U | Mark read / unread |
| V | Move to folder (3P) | G then I | Go to Inbox (3P) |
| / | Search (3P, Outlook.com style) | Shift+N | New contact (People) |
| Shift+L | New contact list (People) | Shift+E | Edit contact (People) |

### 11.2 Gmail overlay (Phase 2)

Microsoft does not publish OWA's Gmail-set table. Keys below come from Google's published Gmail shortcuts (desktop list re-checked 2026-10-19) and are mapped to the nearest Outlook concept; every mapping is (estimate). Note: in Gmail itself single-key shortcuts only work after the user switches them on (Settings → See all settings → Keyboard shortcuts on); in Axentra, choosing the Gmail set is that opt-in.

| Gmail key | Gmail action | Axentra action |
|---|---|---|
| c | Compose | New message |
| d | Compose in a new tab | New message in a pop-out window |
| / | Search | Focus search |
| k / j | Newer / older conversation | Focus previous / next item in list |
| n / p | Next / previous message in conversation | Next / previous message in expanded conversation |
| o or Enter | Open | Open message |
| u | Back to list | Close reading view, focus list |
| e | Archive | Archive |
| # | Delete | Delete |
| r / a / f | Reply / reply all / forward | same |
| Ctrl+Enter | Send | Send |
| Shift+I / Shift+U | Mark read / unread | same |
| s | Star | Flag / unflag |
| ! | Report spam | Mark as junk |
| v | Move to | Move to folder |
| l | Label as | Categorize |
| m | Mute | Ignore conversation |
| z | Undo | Undo last action |
| x | Select conversation | Toggle selection |
| * then a / * then n | Select all / none | Select all / clear selection |
| g then i / t / d / s | Go to Inbox / Sent / Drafts / Starred | Inbox / Sent Items / Drafts / Flagged view |
| ? | Shortcut help | Shortcuts panel |

### 11.3 Yahoo Mail overlay (Phase 2)

From Yahoo's published desktop shortcuts (help article SLN3579, re-checked 2026-10-19); mappings (estimate). Yahoo shows its full list with Shift+?; Axentra maps that to the shortcuts panel. Yahoo documents Cmd+Enter (Mac wording) for send; Ctrl+Enter on Linux is (estimate). Yahoo's article pairs Right/Left with "previous/next" in message view without making the order unambiguous; the direction below is (unverified). Yahoo's L / Shift+L are star / unstar, mapped to flag / unflag.

| Yahoo key | Axentra action | Yahoo key | Axentra action |
|---|---|---|---|
| N | New message | S or / | Focus search |
| M | Go to Inbox | Up / Down | Previous / next item in list |
| [ / ] | Previous / next tab (Focused/Other) | Shift+A | Next account |
| ; | Open Settings | Space | Toggle selection |
| Shift+Up / Shift+Down | Extend selection | K / Shift+K | Mark read / unread |
| L / Shift+L | Flag / unflag | D | Move to folder |
| Delete or Backspace | Delete | E | Archive |
| P | Print | Shift+E | Move to Inbox |
| Shift+P | Preview first attachment | R / A / F | Reply / reply all / forward |
| Ctrl+Enter | Send | Right / Left (reading view) | Next / previous message (unverified direction) |
| Shift+? | Shortcuts panel | | |

---

## 12. Focus model and F6 region cycling

### 12.1 Region order — Mail (main window)

Documented order for the new Outlook: **Ribbon tabs → Left rail app bar → Navigation pane → Message list** (NO guide; F6 forward, Shift+F6 back. The guide says the reading pane is not a stop in the cycle and recommends turning it off). On the app bar, Narrator and JAWS announce the module as a toggle button (e.g. "Mail toggle button"). For comparison, OWA's documented Ctrl+F6 cycle in Mail is search banner → navigation pane → command toolbar → message list (WEB guide). Axentra:

| # | Region | Accessible name (announced) | Focus target on entry | Notes |
|---|---|---|---|---|
| 1 | Ribbon | "Ribbon tabs" | Selected ribbon tab | NO |
| 2 | App bar | "App bar" | Active module button | NO ("left rail appbar") |
| 3 | Navigation pane | "Navigation pane" | Last-focused tree item, else selected folder | NO; skipped when the pane is collapsed (focus the pane's show/hide button instead, estimate) |
| 4 | Message list | "Message list" | Last-focused item, else selected item, else first item | NO |
| 5 | Reading pane | "Reading pane" | Message header (subject heading) or inline compose's last field | AX — deliberate addition: Microsoft's documented new-Outlook cycle has no reading-pane stop. Skipped when reading pane is off and no message is open (estimate) |
| — | Title bar (search, My Day, notifications, settings, account) | "Search" / banner | — | Not in the F6 cycle (NO: reached with Tab/Shift+Tab); Ctrl+E jumps straight to search |

After the last region, F6 wraps to region 1; Shift+F6 goes in reverse. While an Undo toast is visible it is a temporary final stop (estimate). Each region remembers its last-focused element (roving memory) for re-entry.

### 12.2 Region order — other contexts (AX unless tagged)

| Context | Order |
|---|---|
| Calendar | Ribbon → App bar → Navigation pane (date picker, then calendar list — two stops, matching OWA's documented "Jump to" order of date picker then calendar list) → Calendar toolbar (Today, previous/next, date range, view switcher) → Calendar grid → Event details pane (when open). OWA's documented Ctrl+F6 list: New event button, date picker, calendar list, view switcher, calendar view options, calendar view (WEB). |
| People | Ribbon → App bar → Navigation pane (contact folders/lists) → Contact list → Contact details |
| To Do | Ribbon → App bar → Lists pane → Task list (with "Add a task" input first) → Task details pane (when open) |
| Pop-out compose window | Ribbon → Header fields (From/To/Cc/Bcc/Subject) → Body → Attachment well (if any) |
| Pop-out read window | Ribbon → Message header and actions → Body → Attachment well |
| Settings dialog | Not F6-cycled (modal): Tab order through category list (Up/Down), sub-tabs, then settings body; Esc closes (NO guide) |

OWA and new-Outlook variant: in the calendar, Ctrl+F6 lands on a "Jump to {region}" target (new Outlook announces e.g. "Jump to date selection") and Enter performs the jump (WEB / NO calendar guides). Axentra moves focus directly (no intermediate link) (AX).

### 12.3 Skip links and initial focus

- Hidden-until-focused skip links at the top of the DOM: "Skip to message list", "Skip to reading pane", "Skip to navigation pane" (labels estimate; the Microsoft blog documents only that skip links exist at the start of the window and that activating one puts focus directly on the message or the message-list item, not on the region container).
- App start and search completion: focus the **first message list item**, not the list container (NO blog).
- Opening a message in its own window: focus the message body document so JAWS/NVDA-style auto-read can happen (NO blog, Sept 2024: JAWS and NVDA read the whole body on open; Narrator support was still in development then); for Orca, announce subject + sender politely and leave focus on the body.

### 12.4 Focus restoration rules (AX, APG)

| Event | Focus goes to |
|---|---|
| Dialog / menu / popover closes | The element that opened it |
| That element no longer exists (e.g. item deleted) | Its successor in the same list; else predecessor; else the list container (empty state) |
| Item deleted / moved / archived from the list | Next item below; if it was the last, the new last item |
| Compose closed (Esc) | Message that was being replied to; for a new message, the list item that was focused before |
| Search exited | The list item focused before search began |
| Module switched | The module's remembered region and element; first visit → its default (Mail: first message) |

Focus must never land on `<body>`; a debug assertion should log any `focusout` that leaves `document.activeElement === body` while a window is active.

---

## 13. Keyboard shortcuts panel (`?`)

Microsoft documents the three entry points: the support page, Settings → General → Accessibility, and pressing `?` (NO blog). Axentra anatomy (all sizes estimate, Fluent 2 tokens):

- Modal dialog, `role="dialog"`, `aria-modal="true"`, title **Keyboard shortcuts** (h2, 20px/28px Subtitle1 semibold). Width 560px, max-height 80% of window, padding 24px, radius 8px.
- Under the title: read-only line "Shortcut set: {mode label}" + link **Change** (opens Settings → General → Accessibility).
- Search field "Search shortcuts" (filters rows live; announce "{n} shortcuts found").
- Sections (h3, 14px semibold): General, Mail, Reading, Compose, Formatting, Calendar, People, To Do, Search. Section order starts with the current module.
- Row: action label (Body1 14px/20px, colorNeutralForeground1) left; key chips right. A sequence renders chips separated by ", " or " then ".
- Key chip: height 20px, min-width 20px, padding 0 6px, radius 4px, 1px border colorNeutralStroke1 (#D1D1D1 light / #666666 dark), background colorNeutralBackground3 (#F5F5F5 / #141414), text 12px colorNeutralForeground1 (#242424 / #FFFFFF). Accessible text of a row: "{action}: {keys spoken}" e.g. "Reply: Control plus R".
- Close: Esc or the dismiss (X) button top-right (32px).

---

## 14. Arrow-key navigation and roving tabindex

Rule: each composite widget is **one** Tab stop. Exactly one descendant has `tabindex="0"`; all others `tabindex="-1"`. Arrow keys move DOM focus (real `focus()`, not `aria-activedescendant`) — chosen because WebKitGTK/AT-SPI and Orca track real focus most reliably (estimate).

| Widget | Orientation | Arrows | Home/End | Type-ahead | Wrap |
|---|---|---|---|---|---|
| Ribbon tab list | Horizontal | Left/Right switch tabs (NO) | First/last tab | No | Yes |
| Ribbon toolbar | Horizontal | Left/Right between controls; Down opens a menu button | First/last control | No | No |
| App bar | Vertical | Up/Down (NO) | First/last | No | No |
| Folder tree | Vertical | §4.1 | §4.1 | Yes (NO) | No |
| Message list | Vertical | §4.3 | §4.3 | No | No |
| Focused/Other tabs | Horizontal | Left/Right, selection follows focus | — | No | Yes |
| Reading-pane action bar | Horizontal | Left/Right (NO blog: arrows within groups) | First/last | No | No |
| Attachment cards | Horizontal | Left/Right; Enter opens preview; Shift+F10 menu (Download, Save to…, Remove in compose) | First/last | No | No |
| Calendar grid | 2-D | §6.3 | §6.3 | No | No |
| Date picker | 2-D | §6.3 | Start/end of week | No | No |
| Menus | Vertical | Up/Down; Right opens submenu; Left closes submenu | First/last | Yes (first letter) | Yes |
| Recipient pills | Horizontal | Left/Right | First/last pill | No | No |
| Settings category list | Vertical | Up/Down (NO guide) | First/last | No | No |

Disabled items inside toolbars and menus stay focusable with `aria-disabled="true"` (APG) so users can discover them; disabled buttons elsewhere use the `disabled` attribute.

Virtualised lists: the item holding `tabindex="0"` must stay mounted even when scrolled out of view (render it in an off-screen slot) so that Tab back into the list and screen-reader focus never break. Every item carries `aria-setsize` (total in folder, or `-1` while unknown) and `aria-posinset`.

---

## 15. Enter / Space / Escape semantics

| Context | Enter | Space | Escape |
|---|---|---|---|
| Button | Activate | Activate (on keyup) | — (bubbles) |
| Split button primary part | Run default action | Run default action | — |
| Split/menu button arrow part | Open menu (Alt+Down also) | Open menu | Close menu, focus button |
| Folder tree item | Open folder | Open folder | Return focus to message list (estimate) |
| Message list item | Open (see §4.3) | Toggle selection | Clear multi-selection; then (if searching) cancel search |
| Opened message (full view) | Activate focused link/button | Page down | Close and return to list (NO) |
| Inline reply / compose | New line (body) | Space character | Close compose; draft kept (NO / estimate) |
| Search box | Run search / pick suggestion | Space character | Close suggestions; second press exits search (estimate) |
| Calendar slot | Quick-create | Toggle slot selection (estimate) | Clear slot selection |
| Calendar event | Open details | Open details | Close peek/popover |
| Menu item | Activate, close menu | Activate (checkbox/radio items toggle, menu stays open, estimate) | Close menu |
| Dialog | Activate focused button (default button if focus in a text field) | Activate focused control | Cancel/close dialog |
| Alert dialog (destructive) | Activate focused button (initial focus per §16.2) | Activate focused button | Cancel |
| Ribbon / key-tip mode | Activate | Activate | Back out one key-tip level; at top level, leave key-tip mode and return focus to where it was |
| Toast with Undo | Activate Undo when focused | Activate | Dismiss toast (focus unchanged) |

---

## 16. Delete vs Shift+Delete

### 16.1 Behaviour matrix

| Situation | Delete | Shift+Delete |
|---|---|---|
| Item in any folder except Deleted Items | Move to Deleted Items; toast "Deleted" with **Undo** (AX wording); focus → next item | Permanent-delete confirmation (§16.2) |
| Item in Deleted Items | Permanent-delete confirmation (classic behaviour, CL; estimate for new Outlook) | Same confirmation |
| Item in Junk Email | Move to Deleted Items | Confirmation |
| Draft open in compose | n/a (text editing) | n/a (text: cut in classic) |
| Multi-selection | Applies to all selected; toast "N items deleted" + Undo | Confirmation uses plural text |
| Folder (tree) | Confirmation "Delete folder?" → folder moves to Deleted Items; default folders cannot be deleted (CL); command disabled for them | Not bound for folders (AX) |
| Calendar event | §16.3 | Same as Delete (events go through cancellation logic) |
| Contact / list | Delete (NO) after confirmation (estimate) | Same |

Undo: Ctrl+Z (outside text) or the toast's Undo button reverses Delete/Move/Archive within the session; toast lifetime 7 s (estimate); announce "Undone". Permanent deletes are never undoable and show no Undo.

Backend for Delete / Archive / Move (fills a gap; the keyboard command is only useful if the server op matches Outlook):

| Command | Microsoft Graph (v1.0) | IMAP |
|---|---|---|
| Delete (to Deleted Items) | `POST /me/messages/{id}/move` with `destinationId: "deleteditems"` (well-known name, locale-independent). Do **not** use `DELETE /me/messages/{id}` for this: its page does not say the item lands in Deleted Items (see §16.2) | `UID MOVE` to the mailbox flagged `\Trash` (RFC 6154 SPECIAL-USE; MOVE is RFC 6851 and part of IMAP4rev2). Without MOVE: `UID COPY`, then `UID STORE +FLAGS.SILENT (\Deleted)` and a scoped `UID EXPUNGE` (§16.2) |
| Archive | `move` with `destinationId: "archive"` (Graph notes this is the one-click Archive folder, not the Exchange archive mailbox) | `UID MOVE` to the `\Archive` mailbox; if none is advertised, ask the user to pick one once (AX) |
| Move / Copy to folder | `POST …/move` / `POST …/copy` with the folder id | `UID MOVE` / `UID COPY` |
| Junk / Not junk | `move` to `junkemail` / `inbox` (estimate: Outlook's own junk reporting also involves server-side filtering that a plain move does not trigger) | `\Junk` mailbox (RFC 6154) |
| Mark read / unread, flag | `PATCH /me/messages/{id}` with `isRead`, or `flag.flagStatus` = `flagged` / `complete` / `notFlagged` | `UID STORE ±FLAGS (\Seen)` / `(\Flagged)`; IMAP has no "complete" state — keep it locally or in a keyword (estimate) |

Graph `move` and `copy` return `201 Created` with the message **under a new id** (the operation makes a copy in the destination and removes the original). Undo must therefore move the returned id back to the original `parentFolderId`, and the list must re-key the row. Permissions: `Mail.ReadWrite` for the signed-in user's mailbox; shared or delegated mailboxes go through `/users/{id|upn}/…` and need the delegated `Mail.ReadWrite.Shared` scope (estimate — not re-checked in this pass). Bulk operations should use JSON batching (`POST /$batch`, at most 20 requests per batch). A `200` on the batch does not mean every item succeeded: each sub-response has its own status (e.g. 403, 429), so the UI must handle partial failure — keep failed rows in place, announce "{n} of {m} items couldn't be deleted" assertively (AX wording), and exclude them from Undo.

### 16.2 Permanent-delete confirmation dialog

- `role="alertdialog"`, `aria-modal="true"`, `aria-labelledby` → title, `aria-describedby` → body.
- Title: "Permanently delete this item?" / "Permanently delete {n} items?" (AX wording, own words).
- Body (AX wording): IMAP and other non-Exchange accounts: "This removes it from your mailbox and can't be undone." Microsoft 365 / Outlook.com accounts (because Outlook's Shift+Delete is a soft delete, see Backend below): "It won't go to Deleted Items. You may be able to get it back from Recover deleted items for a limited time."
- Buttons: **Delete** (primary) and **Cancel** (secondary). Initial focus on **Delete** so Shift+Delete, Enter keeps classic muscle memory (claim that classic's prompt defaults to Yes is (unverified)). Note the APG modal-dialog guidance points the other way: for dialogs that finish an action that is hard to undo, such as deleting data, it suggests initial focus on the least destructive control (Cancel). Product decision — see Open questions. Esc = Cancel.
- Surface: Fluent dialog, width 448px, padding 24px, radius 8px (estimate); overlay colorBackgroundOverlay rgba(0,0,0,0.4) light / rgba(0,0,0,0.5) dark.
- On confirm: focus → next item in list; polite announcement "{n} items permanently deleted".
- Backend, Microsoft Graph — **correction**: Outlook's Shift+Delete is a *soft* delete. Graph's mailFolder docs describe the well-known folder `recoverableitemsdeletions` as holding items deleted from Deleted Items or with Shift+Delete in Outlook, and users can still restore them through "Recover deleted items". Graph's `permanentDelete` action is stronger: it puts the item in the **Purges** folder of the dumpster, Outlook clients can no longer reach it, and it is purged after the retention period unless a hold applies. So:
  - To mirror Outlook's Shift+Delete, use `DELETE /me/messages/{id}` (204). Graph's delete page does not state where the item goes; that it lands in Recoverable Items\Deletions is (estimate, inferred from the mailFolder docs and widely reported behaviour) — verify against a test tenant.
  - Reserve `POST /users/{id|upn}/messages/{id}/permanentDelete` (also `…/mailFolders/{id}/messages/{id}/permanentDelete`; `/me/…` is the usual alias for the signed-in user; 204; no body; `Mail.ReadWrite` delegated or application) for an explicit "Purge" command, if Axentra offers one. It is listed for the global service only — not for US Government L4/L5 or the 21Vianet (China) cloud — so feature-detect and fall back to `DELETE` there.
- Backend, IMAP: `UID STORE +FLAGS.SILENT (\Deleted)` then `UID EXPUNGE <uid-set>` for exactly those UIDs — never a bare `EXPUNGE`, which would also purge other `\Deleted` messages. `UID EXPUNGE` is part of IMAP4rev2 (RFC 9051); on IMAP4rev1 servers it exists only if the `UIDPLUS` capability (RFC 4315) is advertised. If neither `IMAP4rev2` nor `UIDPLUS` is advertised (estimate, AX fallback): run `UID SEARCH DELETED`; if no other message carries `\Deleted`, a plain `EXPUNGE` is safe; otherwise temporarily remove `\Deleted` from the others, `EXPUNGE`, and restore their flags — or refuse and tell the user, because a concurrent client can race this.
- Related: Deleted Items toolbar exposes "Recover items deleted from this folder" for Exchange accounts (3P university guides; Graph's docs call the feature "Recover Deleted Items from Server"); out of scope for this spec except that it must be keyboard reachable.

### 16.3 Calendar delete flows (AX, estimate)

- Organizer of a meeting: Delete opens "Cancel meeting" compose with optional message; Send = cancellation; Esc aborts.
- Attendee: Delete opens a menu: "Decline and delete" (with send/no-send response options) / "Delete".
- Recurring: choose "This event", "This and all following", "All events" (radio group in a dialog; default "This event").
- Graph backend (verified against the v1.0 reference; all need `Calendars.ReadWrite`):
  - Organizer: `POST /me/events/{id}/cancel` with optional `comment` → 202 Accepted; the event goes to Deleted Items and attendees get the cancellation. Only the organizer may call it (an attendee gets 400). Pass an occurrence id to cancel a single occurrence.
  - Attendee "Decline and delete": `POST /me/events/{id}/decline` with optional `comment` and `sendResponse` (defaults to true; false = "don't send a response") → 202 Accepted, then remove it locally (estimate: Exchange normally removes a declined meeting from the calendar itself).
  - Plain "Delete" (own appointment, no attendees): `DELETE /me/events/{id}`.
  - "This and all following" has no single Graph call (estimate): end the series by updating the master's `recurrence.range` end date to the day before the chosen occurrence (organizer only).

---

## 17. ARIA roles per region

| Region / element | Element + role | Accessible name | Key states / properties |
|---|---|---|---|
| Window document | `<html lang>` | Document title "{Folder} - {account} - Axentra Outlook" (estimate) | `lang` from UI locale |
| Skip links | `<a href="#…">` | "Skip to …" | Visible on focus |
| Title bar | `<header role="banner">` | — | Contains search |
| Search | `<div role="search">` + `<input role="combobox">` | "Search" | `aria-expanded`, `aria-controls` → suggestions `role="listbox"`, `aria-activedescendant` allowed here (focus stays in input) |
| Ribbon tabs | `role="tablist"` | "Ribbon tabs" | tabs `aria-selected`, `aria-controls`; `aria-keyshortcuts="Alt+H"` etc. |
| Ribbon commands | `role="tabpanel"` → `role="toolbar"` | "{Tab name}" | `aria-orientation="horizontal"` |
| Split button | `role="group"` with two `<button>`s | "{Command}" and "More {command} options" | arrow button `aria-haspopup="menu"`, `aria-expanded` |
| Overflow | `<button aria-haspopup="menu">` | "More options" | |
| App bar | `<nav aria-label="App bar">` → `role="toolbar" aria-orientation="vertical"` | Module names | active button `aria-current="page"`; `aria-keyshortcuts="Control+1"` … (live mapping) |
| Navigation pane | `<section role="region">` | "Navigation pane" | Collapsible; toggle button `aria-expanded` |
| Folder tree | `role="tree"` | "Folders" | `treeitem` with `aria-level`, `aria-expanded` (parents only), `aria-selected` (the open folder), `aria-setsize`, `aria-posinset`; unread count in name: "Inbox, 12 unread" |
| Favorites / account roots | top-level `treeitem` | "Favorites", "{account email}" | expandable groups |
| Message list region | `<section role="region">` | "Message list" | contains header toolbar + list |
| List header controls | Focused/Other `role="tablist"`; Filter and Sort `aria-haspopup="menu"` buttons; "Select" toggle `aria-pressed` | "Focused", "Other", "Filter", "Sort" | |
| Message list (Conversation view off) | `role="listbox"` `aria-multiselectable="true"` | "{Folder name}" | `role="option"`, `aria-selected`, `aria-setsize`, `aria-posinset` |
| Message list (Conversation view on) | `role="tree"` `aria-multiselectable="true"` | "{Folder name}" | conversation = `treeitem` with `aria-expanded`; child messages = nested `treeitem` `aria-level="2"` |
| Row quick actions (hover icons) | `<button tabindex="-1" aria-hidden="true">` | — | Keyboard uses shortcuts/context menu instead |
| Date group headers ("Today", "Yesterday", "Last week") | `role="presentation"` visual + text folded into first item's description | — | Avoid non-option children inside listbox |
| Reading pane | `<main>` | "Reading pane" | |
| Message header | `<h2>` subject; sender block; `role="toolbar"` "Message actions" | | |
| Message body | `<iframe title="Message body">` (sandboxed, srcdoc) | "Message body" | inner document `role="document"`; `lang` from message if known |
| Attachments | `role="list"` of cards; each card a `<button>` + menu button | "Attachments, {n}" | |
| Info bars (blocked images, importance, external sender) | `role="note"`; actions are buttons | | Not live (static) |
| Compose region | `<section role="region">` (inline) / `<main>` (pop-out) | "Compose" / "New message" | |
| Recipient field | `<input role="combobox">` + pills `role="list"` | "To", "Cc", "Bcc" | suggestions `role="listbox"` |
| Subject | `<input>` | "Add a subject" | |
| Body editor | `contenteditable` `role="textbox"` `aria-multiline="true"` | "Message body" | |
| Calendar toolbar | `role="toolbar"` | "Calendar navigation" (estimate) | date range text in a `aria-live="polite"` span (announces "October 19 – 25, 2026" after navigation) |
| View switcher | `role="radiogroup"` or menu button | "View: Week" | |
| Time grid / month grid | `role="grid"` | "{View}, {date range}" | `columnheader` per day (`aria-current="date"` on today); `rowheader` per hour; `gridcell` per slot with `aria-selected`; events are `<button>`s inside cells |
| Date picker | `role="grid"` | "{Month year}" | `aria-current="date"`, `aria-selected` |
| Contact list | `role="listbox"` multiselectable | "Contacts" | |
| Task list | `role="grid"` | "{List name}" | row = task; cells: complete `role="checkbox"` "Mark as completed", title, importance toggle `aria-pressed` "Mark as important" |
| Menus | `role="menu"`, `menuitem`, `menuitemcheckbox`, `menuitemradio`, `separator` | | `aria-checked`, `aria-haspopup` for submenus |
| Dialog | `role="dialog"` `aria-modal="true"` | title via `aria-labelledby` | focus trap; inert background |
| Destructive confirmation | `role="alertdialog"` | title | `aria-describedby` body |
| Toast | Rendered inside the persistent polite live region (`role="status"`) | Toast text | Focus never moves to a toast. While a toast is visible it is an extra F6 stop after the last region (estimate); Ctrl+Z triggers its Undo regardless of focus |
| Error banner | `role="alert"` | | |
| Tooltips | `role="tooltip"` linked via `aria-describedby` | | Show shortcut: "Reply (Ctrl+R)" (estimate); hidden on Esc |
| Key tip badges | `aria-hidden="true"` | | Keys are exposed via `aria-keyshortcuts` instead |

Use `aria-keyshortcuts` on every control that has a direct chord (values use `Control`, `Shift`, `Alt` spelling: `aria-keyshortcuts="Control+Shift+R"`), regenerated when the shortcut set changes.

### 17.1 Accessible-name templates (AX, estimate)

- Message item: `"{Unread, }{Important, }{Flagged, }{Pinned, }{Has attachments, }{Replied, |Forwarded, }{Meeting request, }{sender}, {subject}, {time}, {preview}"`; conversations add `"{n} messages"` (expanded/collapsed comes from `aria-expanded`).
- Drafts: prefix `"Draft, "`; Sent Items: `"To {recipients}"` instead of sender.
- Folder: `"{name}{, n unread}{, n items}"` (item count only for Drafts/Outbox, mirroring visible counts).
- Event: §6.3. Day cell: `"{weekday}, {month} {day}{, today}, {n} events"`.
- Never rely on visual-only cues: unread (bold + bar), flag colour, category colour, importance icons must all appear in text.

---

## 18. Screen-reader announcements and live regions

Two persistent visually-hidden live regions are mounted at app start (they must exist before content changes): `#live-polite` (`role="status"`, `aria-live="polite"`) and `#live-assertive` (`role="alert"`). To repeat an identical message, clear the node, then set text after 100 ms (estimate).

| Event | Region | Text (AX wording) | Rules |
|---|---|---|---|
| New mail arrives, window focused, Mail module visible | polite | "New message from {sender}: {subject}" | Only for Inbox (Focused tab if Focused Inbox is on) and favourited folders; batch arrivals within 2 s into "{n} new messages" (estimate); never move focus or selection; keep the focused item and scroll position stable (insert above with scroll anchoring) |
| New mail, window not focused or other module | — | Desktop notification via org.freedesktop.Notifications (Orca reads these natively), title = sender, body = subject | Respects Settings → General → Notifications; sound per the same setting (the new Outlook documents a sound on arrival; no announcement is documented) |
| Item deleted / moved / archived | polite | "Deleted", "Moved to {folder}", "Archived" (+ "{n} items") | |
| Read/unread/flag toggled by shortcut | polite | "Marked as read", "Marked as unread", "Flagged", "Flag cleared", "Marked complete" | |
| Selection count changes (≥2) | polite | "{n} selected" | debounced 300 ms |
| Search results | polite | "{n} results" / "No results" | |
| Message sent | polite | "Sent" | |
| Draft saved (manual Ctrl+S only) | polite | "Draft saved" | Autosaves are silent |
| Calendar period changed | polite | date range text | via toolbar live span |
| View changed | polite | "Week view" etc. | |
| Undo | polite | "Undone" | |
| Shortcut set changed | polite | "Keyboard shortcuts: {label}" | |
| Sync error / send failure / offline | assertive | "Couldn't send '{subject}'. Saved in Drafts." / "You're offline" | Also a visible banner |
| Long load (> 1 s) in list | polite | "Loading messages" then nothing on completion unless focus target changed | |
| Reminder due | assertive | "Reminder: {title}, {start time}" | Reminder window also takes focus only if the app is focused (estimate) |

Setting (AX): Settings → General → Accessibility → "Announce new messages with screen readers" (checkbox, default on).

---

## 19. Focus indicator (visual)

Fluent 2 implementation in the MIT `microsoft/fluentui` repo (re-checked on `master`, 2026-10-19):
- Generic focus outline (`createFocusOutlineStyle`): a `::after` overlay (`position: absolute`, `pointer-events: none`, `z-index: 1`) with a **2px** solid border in `colorStrokeFocus2` and corner radius `borderRadiusMedium` (4px). With no offset option, each edge sits at `calc(2px * -1)`, i.e. the ring is drawn 2px outside the element; an offset pushes it further out. The width is a literal `2px` in source (a comment says the `strokeWidthThick` token caused bugs), which equals `strokeWidthThick`. While focused, the element's own border colour is set to `transparent` and its native outline is suppressed. Under `forced-colors: active` the `::after` border becomes `Highlight`. The `::after` needs the focused element to be `position: relative` (or another containing block).
- Buttons (`createCustomFocusIndicatorStyle`): 1px border in `colorStrokeFocus2` + `0 0 0 strokeWidthThin` inset box-shadow in `colorStrokeFocus2` (together a 2px inner ring), radius `borderRadiusMedium`, `z-index: 1`, plus a 2px **transparent** outline (`colorTransparentStroke`) so forced-colors mode, which drops box-shadows, still paints a ring. Under `forced-colors: active` a focused button's border is `ButtonText` (not Highlight); hover and pressed use a `Highlight` border and text on a `HighlightText` background.

| Token | Light | Dark | Use |
|---|---|---|---|
| colorStrokeFocus2 | #000000 | #FFFFFF | Focus ring |
| colorStrokeFocus1 | #FFFFFF | #000000 | Optional inner contrast line on brand-filled controls |
| strokeWidthThick | 2px | 2px | Ring width |
| borderRadiusMedium | 4px | 4px | Ring radius |

Axentra rules:
- Show only for keyboard focus (`:focus-visible`; WebKitGTK supports it). After a shortcut moves focus programmatically (F6, Ctrl+Y, Ctrl+E), the ring must show — set focus with `focus({ focusVisible: true })` where supported, or toggle a `data-kbd-focus` attribute on the root after any keydown (estimate fallback). Support for the `focusVisible` option per MDN compat data: Firefox 104, Safari 18.4, Chrome 145. WebKitGTK should have it from the release built on the same WebKit as Safari 18.4 (likely 2.48 — (unverified)); feature-detect at runtime (a getter on the options object reveals whether the engine reads `focusVisible`) and keep the `data-kbd-focus` fallback for older distro WebKitGTK.
- Inside scroll containers (folder tree, message list, contact list, calendar grid) draw the ring **inset** by 2px so it is not clipped (estimate).
- On brand-filled controls (primary buttons, selected app-bar item, selected date) add the 1px `colorStrokeFocus1` inner line between fill and ring (estimate).
- Ring must not be obscured by sticky headers (WCAG 2.4.11): when focusing a list item, scroll it fully into view below the sticky list header.
- Contrast: #000000 on #FFFFFF / #FFFFFF on #292929 exceeds 3:1 against adjacent colours (Fluent 2: non-text ≥ 3:1).

---

## 20. High-contrast mode

### 20.1 Detection on Linux (AX)

WebKitGTK cannot be relied on for `forced-colors: active`. WebKit does parse the media features (MDN compat data: `prefers-contrast` since Safari 14.1, `forced-colors` since Safari 16), but `forced-colors` only becomes `active` when the platform forces a palette — in practice Windows contrast themes — and no Linux source for it is known, so on WebKitGTK it is expected to stay `none`. Whether WebKitGTK maps `prefers-contrast: more` to the GTK or portal contrast setting is (unverified) and needs an engineering spike on the minimum supported WebKitGTK. Axentra therefore applies its own high-contrast theme, setting `data-contrast="high"` plus `data-theme="dark|light"` on `<html>`, in this priority order:

1. User override: Settings → General → Accessibility → **High contrast**: "Use system setting" (default) / "On" / "Off".
2. xdg-desktop-portal Settings `org.freedesktop.appearance` key `contrast` (`u`): `0` = no preference, `1` = higher contrast; treat any other value as `0` (the portal spec says so). Read it with `ReadOne` (interface version 2; `Read` is deprecated because it double-wraps the variant — use it only as a fallback on version-1 portals) and listen to `SettingChanged(namespace, key, value)` for live updates. `ReadAll(["org.freedesktop.appearance"])` fetches `color-scheme`, `accent-color`, `contrast` and `reduced-motion` in one call.
3. GNOME fallback: `org.gnome.desktop.a11y.interface high-contrast = true`.
4. GTK theme name containing "HighContrast" (`HighContrastInverse` → dark variant).
5. CSS `prefers-contrast: more` in the WebView, if it reports a value.
6. Also honour `forced-colors: active` if it ever reports true (future-proof); the system-colour CSS below works in both paths.

Light/dark variant: from the portal `color-scheme` key (0 = no preference, 1 = prefer dark, 2 = prefer light), else the app theme setting.

### 20.2 Palettes

High-contrast dark — values from Fluent's global high-contrast colours (fluentui `packages/tokens/src/global/colors.ts`: `hcCanvas`, `hcCanvasText`, `hcHighlight`, `hcHighlightText`, `hcHyperlink`, `hcDisabled`; re-checked 2026-10-19):

| Role (CSS system colour) | Hex | Used for |
|---|---|---|
| Canvas | #000000 | All surfaces |
| CanvasText | #FFFFFF | Text, borders, icons |
| Highlight | #1AEBFF | Selected item background, focus ring, hover border |
| HighlightText | #000000 | Text on Highlight |
| LinkText | #FFFF00 | Links |
| GrayText | #3FF23F | Disabled text and borders |
| ButtonFace / ButtonText | #000000 / #FFFFFF (Axentra mapping, estimate; Fluent's `hcButtonFace`/`hcButtonText` globals are #FFFFFF/#000000 and are used for inverted states) | Buttons |

High-contrast light (estimate; modelled on the legacy Windows "High Contrast White" scheme):

| Role | Hex |
|---|---|
| Canvas | #FFFFFF |
| CanvasText | #000000 |
| Highlight | #37006E |
| HighlightText | #FFFFFF |
| LinkText | #00009F |
| GrayText | #600000 |

### 20.3 Rendering rules in high contrast

- Remove all shadows, gradients, acrylic/mica-like translucency and background images; separation comes from 1px CanvasText borders.
- Every interactive control gets a visible 1px CanvasText border at rest (backgrounds alone vanish). Hover: border and text turn Highlight (Fluent button forced-colors hover uses Highlight border/text on HighlightText background).
- Selected list/tree item: Highlight background, HighlightText text and icons. Focused + selected: add a 2px CanvasText ring inset 2px inside the Highlight fill (estimate) so focus and selection stay distinguishable.
- Focus ring: 2px solid Highlight (Fluent's generic forced-colors focus outline uses `Highlight`; note Fluent buttons instead switch their focus border to `ButtonText` in forced colours — Axentra uses Highlight everywhere for one consistent cue (AX)).
- Disabled: GrayText for text, border and icon; no opacity tricks.
- Icons: SVG with `fill="currentColor"`; no colour-only icons. Coloured status glyphs (presence, category dots, flag red) get a CanvasText outline and the meaning must be in text.
- Unread messages: bold subject + sender and a 3px CanvasText bar in place of the brand bar (estimate); "Unread" in accessible name.
- Categories: render as outlined chips with the category name text; colour swatch dropped.
- Calendar events: Canvas fill, 2px CanvasText border, CanvasText text; tentative = dashed border; out-of-office = double border (estimate); free = 1px dotted. Selected event: Highlight fill. Today's column header: Highlight underline 3px.
- Message bodies (HTML mail): inject an override stylesheet into the body iframe forcing text/background/link colours to the palette (`color: CanvasText !important; background: Canvas !important; a { color: LinkText !important }`, borders → CanvasText); images remain. Add a reading-pane info bar action "Show original colours" for the current message (estimate).
- Never use `opacity` < 1 to signal state; never use colour as the only state cue (WCAG 1.4.1).
- The CSS should be written with system-colour keywords (`Canvas`, `CanvasText`, `Highlight`…) behind `@media (forced-colors: active)` **and** duplicated under `[data-contrast="high"]` with the hex values above, so both detection paths work.

---

## 21. Motion, zoom, text size, target size (AX)

- Reduced motion: honour portal `org.freedesktop.appearance` `reduced-motion = 1` (documented key; older portals may not provide it), else GNOME's `org.gnome.desktop.interface enable-animations = false` (estimate fallback), and `prefers-reduced-motion: reduce` in the WebView; set all transition/animation durations to 0 ms except focus-ring appearance (instant anyway).
- Zoom: Ctrl+= / Ctrl+- / Ctrl+0 scale the whole WebView 50–200% in 10% steps (estimate); reading-pane zoom is separate (View tab). Layout must reflow without clipping at 200% text (Fluent 2 guidance) and keep the F6 cycle intact when panes collapse.
- Respect GNOME `text-scaling-factor` by multiplying the root font size.
- Minimum pointer target 24×24 CSS px (WCAG 2.5.8); standard Fluent buttons are 32px tall.
- Text contrast ≥ 4.5:1 (normal), ≥ 3:1 (large text, icons, borders of inputs) per Fluent 2 accessibility guidance.

---

## 22. Key tips (Alt) (AX detail; existence and Alt-to-show documented NO)

- Trigger: Alt keydown followed by Alt keyup with no other key or mouse button in between → enter key-tip mode. Alt held with another key is a chord, not key tips.
- Badges appear on ribbon tabs first (Home H, View V, Help Y; compose: Message H, Insert N, Format text O, Draw D, Options P — letters other than H, V, N are estimates). Typing a tab's letter activates the tab and shows badges for its commands (one or two letters, e.g. Reply R P, Reply all R A, Forward F W mirroring classic Home tab tips; estimate).
- Esc backs out one level; Alt or a click anywhere exits. Focus returns to the element focused before key-tip mode unless a command moved it.
- Badge visual: background colorNeutralBackgroundStatic (#333333 light / #3D3D3D dark), text colorNeutralForegroundStaticInverted #FFFFFF, 12px semibold, height 18px, min-width 16px, padding 0 4px, radius 2px, anchored bottom-centre of the control, overlapping by 4px (all sizes estimate). High contrast: Canvas background, CanvasText text, 1px CanvasText border.

---

## 23. Edge cases

1. **Bare-letter keys while a text field is focused**: ignored (WCAG 2.1.4 character-key shortcuts — users can also turn them off; they are inactive in text fields).
2. **IME composition**: ignore all shortcuts while composing (Enter in an IME candidate list must not send).
3. **Keys pressed inside the message-body iframe**: iframe key events do not bubble to the parent document. Attach a keydown listener to `iframe.contentDocument` (requires `sandbox="allow-same-origin"` without `allow-scripts`) and forward to the keymap; never enable scripts in mail content. Whether WebKitGTK fires a parent-registered listener on a document whose browsing context has scripting disabled by the sandbox is (unverified) — test it on the minimum supported WebKitGTK; if it does not fire, the fallback needs a security review (estimate options: make the body iframe non-focusable and scroll it from the parent, or move to sanitised HTML in a shadow root) — enabling `allow-scripts` is never an option.
4. **Ctrl+F in the reading pane** is Forward, not Find (NO). Find-in-message has no documented new-Outlook key; offer it in "More actions" (estimate).
5. **Delete with nothing selected** (empty folder): no-op, announce nothing.
6. **Shift+Delete in a search-results list spanning folders**: confirmation lists "{n} items from {m} folders" (estimate).
7. **Shared/delegate mailboxes**: Delete moves the item to the mailbox owner's Deleted Items by calling `POST /users/{owner}/messages/{id}/move` with `destinationId: "deleteditems"` (an explicit move, not a Graph default — `DELETE` does not document a destination); this needs write access to the shared mailbox (`Mail.ReadWrite.Shared`, estimate). Shift+Delete needs the same permission — if denied (403), show an assertive error and keep the items.
8. **Module switch while compose has unsent text**: inline drafts autosave; pop-out compose windows are unaffected.
9. **Shortcut set switched while a G-then-I sequence is pending**: cancel the pending sequence.
10. **Ctrl+N with focus in the search box**: still creates a new item for the module (global), clearing nothing.
11. **Ctrl+Enter with no recipients**: do not send; assertive "Add at least one recipient" and focus the To field.
12. **Escape inside nested popups** (suggestions inside compose inside reading pane): closes only the innermost layer per press.
13. **Rapid Delete auto-repeat at the end of the list**: stop at the last item; do not wrap.
14. **Calendar Home key** inside a text field in the event form edits text, not "go to today".
15. **Right-to-left UI locales**: Left/Right arrow semantics flip for tree expand/collapse, toolbars and calendar day movement; Ctrl+Alt+Left/Right still mean previous/next period in reading order (estimate).
16. **Pop-out windows**: each window has its own F6 cycle, live regions and key-tip state; module shortcuts (Ctrl+1…) in a pop-out focus the main window and switch module there (estimate).
17. **Orca structural navigation**: the reading-pane body is browse-mode content; H/K/etc. belong to Orca there. Message-list single-letter keys only work in focus mode inside the composite.
18. **Context menu by keyboard**: position it at the focused element's bounding box (bottom-left, flipped to fit), not at the mouse pointer.

---

## 24. Implementation notes (must not miss)

1. **One keymap engine**, data-driven: `struct Binding { keys: KeySeq, command: CommandId, scopes: ScopeMask, modes: ModeMask, in_text: bool, repeat: bool }`, loaded from a static table per `KeymapMode`. Tooltips, `aria-keyshortcuts`, the `?` panel and the settings preview all read this table — never hard-code labels.
2. Install a single capture-phase keydown handler at the document root (and one per body iframe, §23.3) that resolves scope from `document.activeElement` ancestry using `data-scope` attributes; call `prevent_default()`/`stop_propagation()` on a match so WebKitGTK defaults do not also fire.
3. Disable the WebView's default context menu and devtools accelerators in release builds; implement Shift+F10 / Menu and the `contextmenu` event yourself.
4. Ctrl+digit and Ctrl+Shift+digit match on `code`; letters on `key` with `code` fallback; `?`, `<`, `>` per §2.2. Test with US, UK, German (QWERTZ), French (AZERTY) and Russian layouts.
5. Respect `is_composing()` and ignore auto-repeat for one-shot commands (§2.3).
6. Roving tabindex with real DOM focus for every composite; keep the focused item mounted in virtualised lists; maintain `aria-setsize`/`aria-posinset`.
7. F6 cycle is computed from currently visible regions (collapsed panes and hidden reading pane are skipped) and each region remembers its last-focused element.
8. Mount both live regions at startup; never create them on demand. Debounce/batch announcements as specified; never announce autosave.
9. New-mail arrival must not steal focus, change selection, or shift the list under the user's focus.
10. Shift+Delete always confirms; IMAP uses UID EXPUNGE scoped to the chosen UIDs (needs IMAP4rev2 or UIDPLUS; fallback in §16.2); Graph uses `DELETE` (soft delete, matching Outlook's Shift+Delete), with `permanentDelete` reserved for an explicit purge and feature-detected per cloud. Normal Delete is a Graph `move` to `deleteditems` / IMAP `MOVE` to `\Trash`, and is undoable (Ctrl+Z / toast) using the new item id the move returns.
11. High contrast is Axentra's own theme (`data-contrast="high"`) driven by the portal `contrast` key and GNOME/GTK fallbacks, plus `forced-colors` CSS for future WebKit support. Inject palette overrides into HTML mail bodies.
12. Focus ring: 2px `colorStrokeFocus2` (#000000 light / #FFFFFF dark), 4px radius, inset inside scroll containers, Highlight in high contrast, visible after programmatic focus moves triggered from the keyboard.
13. Linux conflicts: Ctrl+Alt+Arrow may never arrive (GNOME workspaces) — provide alternatives; Insert may be Orca's modifier — provide alternatives for Flag; never bind Super, Alt+F-keys or Alt+Tab.
14. `KeymapMode::Off` must still allow full keyboard operation (Tab/arrows/Enter/Esc/F6/Shift+F10/key tips).
15. Every visual-only cue (unread, flagged, category, importance, attachment, meeting, tentative) must also be present in the accessible name.
16. Automated checks: run axe-core in dev builds against each screen; manual passes with Orca on GNOME (Wayland and X11) and with accerciser to inspect the AT-SPI tree; keyboard-only smoke test of every row in §3–§9.
17. Keep Microsoft's documentation inconsistencies (Notes A and B in §6.1, Ctrl+U/Ctrl+Y double meanings) covered by tests so behaviour is deliberate, not accidental.

---

## Open questions

Status after the 2026-10-19 fact-check pass is given in *italics* after each item.

1. Calendar week/month direction: Microsoft's new-Outlook table pairs Alt+Down/Alt+PageDown with "previous"; classic pairs them with "next". Axentra uses classic semantics — confirm with product. *Still open. The Outlook-on-the-web table matches the new-Outlook table (Down/PageDown = previous), so two of Microsoft's three tables disagree with classic. Needs a hands-on check in a live new Outlook or OWA session.*
2. Week view key: Microsoft lists Ctrl+Alt+2 for both Work week and Week in new Outlook; Axentra uses Ctrl+Alt+3 for Week (classic). Confirm. *Likely a documentation slip (web uses Shift+Alt+3 for Full week; classic uses Ctrl+Alt+3), and Alt+- is documented for Full week in new Outlook anyway. Keep Ctrl+Alt+3; still unverified against the live app.*
3. Does "Turn off keyboard shortcuts" disable Delete / Shift+Delete / Insert in the message list in the new Outlook? Not documented; Axentra keeps Delete and Shift+Delete active. *Still open. The only extra detail found is 3P wording that the switch turns off shortcuts "for Mail, Calendar, and People".*
4. Default app-bar order (and therefore Ctrl+3/Ctrl+4 targets) depends on the layout spec; this spec assumes Mail, Calendar, People, To Do. *Microsoft's own pages disagree (shortcut page: Copilot 3, People 4; new-Outlook calendar guide: Mail, Calendar, Contacts, To Do), which supports the position-based rule. Still a layout-spec dependency.*
5. Archive key in the Windows set: no documented new-Outlook binding; Backspace is proposed from classic parity — confirm or drop. *Partly resolved: Backspace-to-archive is documented by Microsoft for classic Outlook (Archive support article; admin-disableable there), not for new Outlook. Keeping it as a CL-alias is defensible; product to confirm.*
6. Ctrl+Shift+Z: Linux users expect redo; classic Outlook uses it for clear formatting. Proposed: redo. *Confirmed that classic lists Ctrl+Shift+Z **or** Ctrl+Space for clear formatting, so Ctrl+Space still covers clear formatting if Ctrl+Shift+Z becomes redo. New Outlook documents neither. Product decision.*
7. Gmail and Yahoo Mail sets: Microsoft does not publish OWA's tables for them; ship in Phase 2 using the mappings in §11 or drop them? *Google's and Yahoo's own tables were re-checked and §11 updated (Gmail `d`; Yahoo Shift+?). Microsoft's OWA mapping remains unpublished. Product decision.*
8. Should Axentra offer a separate "Outlook (classic)" set (Alt+S send default, Ctrl+Shift+M, F9, Ctrl+./Ctrl+,) instead of folding classic aliases into the Windows set? *Product decision; no new facts.*
9. Quick Steps shortcuts (classic Ctrl+Shift+1…9) collide with web-set module keys; which range should Quick Steps use, if implemented? *The Ctrl+Shift+1…9 range is not on Microsoft's shortcut page and the Quick Steps article could not be retrieved in this pass, so the classic range itself is (unverified). Product decision.*
10. Permanent-delete dialog initial focus: Delete (Outlook parity) vs Cancel (APG's safer default). *APG's modal-dialog guidance confirmed: for hard-to-undo actions such as deleting data, start on the least destructive control. The "classic defaults to Yes" premise is unverified. Product decision.*
11. Key-tip letters for ribbon commands are not documented for the new Outlook; the proposed letters need a product decision. *Still open. Only Alt, H and Alt, V (People) and Alt+N for the compose Insert tab are documented.*
12. WebKitGTK support for `prefers-contrast` and `forced-colors` on Linux is unconfirmed. *WebKit parses both (Safari 14.1 / 16), but nothing shows WebKitGTK wiring them to GTK or portal settings. Engineering spike needed; §20.1's own detection stays the plan.*
13. Exact on-screen labels of the new-Outlook keyboard-shortcut setting (radio options vs. a single "Turn off keyboard shortcuts" checkbox) are (unverified); Microsoft's support page does not describe the new-Outlook chooser. Check a current build before freezing strings.
14. Shift+Delete semantics on Microsoft 365: Graph `DELETE` (soft delete, user-recoverable, matches Outlook) vs `permanentDelete` (Purges, not user-recoverable, global cloud only). This spec now picks `DELETE`; confirm with product and verify the destination folder on a test tenant.
15. `focus({ focusVisible: true })` and parent-registered keydown listeners on a sandboxed (no `allow-scripts`) body iframe both need a check on the minimum supported WebKitGTK release.

---

## Sources

- https://support.microsoft.com/en-us/office/keyboard-shortcuts-for-outlook-3cdeb221-7ae5-4c1d-8c1d-9e63216c1efd
- https://support.microsoft.com/en-us/accessibility/outlook/keyboard-shortcuts-for-outlook
- https://support.microsoft.com/en-us/article/015951f7-6657-4864-b7a8-f2cfa296554d
- https://support.microsoft.com/en-US/accessibility/outlook/use-a-screen-reader-to-explore-and-navigate-outlook-mail
- https://support.microsoft.com/en-au/office/basic-tasks-using-a-screen-reader-with-email-in-outlook-3fe74ea4-b512-490f-bb42-95fdeb722b9e
- https://support.microsoft.com/en-US/accessibility/outlook/use-a-screen-reader-to-explore-and-navigate-outlook-calendar
- https://support.microsoft.com/en-US/accessibility/outlook/basic-tasks-using-a-screen-reader-with-the-calendar-in-outlook
- https://support.microsoft.com/en-us/accessibility/outlook/learn-how-to-navigate-in-outlook-using-accessibility-features
- https://support.microsoft.com/en-us/todo/using-keyboard-shortcuts-in-microsoft-to-do
- https://blogs.microsoft.com/accessibility/accessibility-and-usability-improvements-in-the-new-outlook-for-windows/
- https://techcommunity.microsoft.com/discussions/outlookgeneral/disable-ctrlenter-to-send-mail/4040831
- https://learn.microsoft.com/en-us/graph/api/message-permanentdelete?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/windows/apps/design/accessibility/high-contrast-themes
- https://blogs.windows.com/msedgedev/2020/09/17/styling-for-windows-high-contrast-with-new-standards-for-forced-colors/
- https://fluent2.microsoft.design/accessibility
- https://github.com/microsoft/fluentui (packages/tokens/src/alias/lightColor.ts, darkColor.ts; packages/tokens/src/global/colors.ts, brandColors.ts; packages/react-components/react-tabster/src/focus/createFocusOutlineStyle.ts; packages/react-components/react-button/library/src/components/Button/useButtonStyles.styles.ts)
- https://www.howtogeek.com/412677/how-to-change-your-outlook.com-shortcuts-to-match-gmail-or-yahoo/
- https://robert365.com/article/keyboard-shortcuts-for-outlookweb-owa-and-outlookcom
- https://robert365.com/article/outlook-navigation-pane-on-the-left
- https://www.slipstick.com/how-to-outlook/send-using-ctrl-enter/
- https://support.google.com/mail/answer/6594
- https://help.yahoo.com/kb/SLN3579.html
- https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html
- https://www.w3.org/WAI/ARIA/apg/practices/keyboard-interface/
- https://www.w3.org/WAI/ARIA/apg/practices/landmark-regions/
- https://www.w3.org/WAI/ARIA/apg/patterns/treeview/
- https://www.w3.org/WAI/ARIA/apg/patterns/listbox/
- https://www.w3.org/WAI/ARIA/apg/patterns/grid/
- https://www.w3.org/WAI/ARIA/apg/patterns/toolbar/
- https://www.w3.org/WAI/ARIA/apg/patterns/tabs/
- https://www.w3.org/WAI/ARIA/apg/patterns/menu-button/
- https://www.w3.org/WAI/ARIA/apg/patterns/combobox/
- https://www.w3.org/WAI/ARIA/apg/patterns/dialog-modal/
- https://www.w3.org/WAI/ARIA/apg/patterns/alertdialog/
- https://www.w3.org/WAI/ARIA/apg/patterns/dialog-modal/examples/datepicker-dialog/
- https://www.w3.org/TR/wai-aria-1.2/
- https://www.w3.org/TR/WCAG22/
- https://www.w3.org/TR/css-color-adjust-1/
- https://developer.mozilla.org/en-US/docs/Web/CSS/@media/prefers-contrast
- https://www.rfc-editor.org/rfc/rfc9051

Added in the 2026-10-19 fact-check pass:

- https://support.microsoft.com/en-us/office/archive-in-outlook-for-windows-25f75777-3cdc-4c77-9783-5929c7b47028 (Backspace archiving in classic; DisableOneClickArchive)
- https://learn.microsoft.com/en-us/graph/api/message-delete?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/message-move?view=graph-rest-1.0 (new id after move; well-known `destinationId`)
- https://learn.microsoft.com/en-us/graph/api/resources/mailfolder?view=graph-rest-1.0 (well-known folder names; `recoverableitemsdeletions` and Shift+Delete)
- https://learn.microsoft.com/en-us/graph/api/event-cancel?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/event-decline?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/json-batching
- https://learn.microsoft.com/en-us/answers/questions/4627876/outlook-keyboard-shortcuts (3P: new Outlook allows only "turn all off")
- https://techcommunity.microsoft.com/discussions/outlookgeneral/disable-ctrlenter-to-send-mail/4040831/replies/4062395
- https://www.elevenforum.com/t/change-keyboard-shortcuts-to-use-in-outlook-for-windows-11.17464 (3P tutorial for the new-Outlook set chooser; page could not be fetched directly, wording taken from its search summary)
- https://www.w3.org/WAI/ARIA/apg/patterns/dialog-modal/ (initial focus on the least destructive action)
- https://docs.rs/dioxus-html/latest/dioxus_html/events/struct.KeyboardData.html (dioxus-html 0.7.10)
- https://github.com/mdn/browser-compat-data (api/HTMLElement.json `focus` options `focusVisible`; css/at-rules/media.json `prefers-contrast`, `forced-colors`, `prefers-reduced-motion`)
- https://developer.mozilla.org/en-US/docs/Web/API/HTMLElement/focus
- https://developer.mozilla.org/en-US/docs/Web/CSS/@media/forced-colors
- https://www.rfc-editor.org/rfc/rfc4315 (UIDPLUS: UID EXPUNGE on IMAP4rev1)
- https://www.rfc-editor.org/rfc/rfc6851 (MOVE)
- https://www.rfc-editor.org/rfc/rfc6154 (SPECIAL-USE: \Trash, \Archive, \Junk)
