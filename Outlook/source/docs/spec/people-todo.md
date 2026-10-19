# Axentra Outlook: People and To Do

Build specification for the **People** module, the **To Do** module and the **My Day** side pane of Axentra Outlook. Axentra Outlook is a Linux desktop mail and calendar client written in Rust, with a Dioxus (HTML/CSS) UI. It mirrors the layout and behaviour of the new Outlook for Windows and Outlook on the web (Fluent 2 era, 2024-2026). Where classic Outlook behaves differently, this spec says so.

## 0. Conventions used in this document

| Marker | Meaning |
|---|---|
| `[S12]` | A fact taken from source 12 in the **Sources** list at the end of this file. |
| `(estimate)` | Public documentation gives no value for this. It is a best estimate that fits Fluent 2 guidance. It is **not** documented Outlook behaviour, and it can be tuned. |
| `(unverified)` | The item comes from third-party material, from an older Microsoft article, or from common practice, and a current Microsoft or standards source could not confirm it during the 2026-10-19 fact-check. Treat it like an estimate until design or QA confirms it. |
| **Axentra decision** | A deliberate product choice for something that is undocumented, needs a Microsoft-only service, or does not apply on Linux. It makes no claim about what Outlook does. |
| `colorXxx` | A Fluent 2 (Fluent UI React v9) alias token. Hex values come from the MIT-licensed `microsoft/fluentui` token sources with the `brandWeb` ramp applied [S37][S38]. The inline comments in those token files show older brand hex values; ignore them and use the values in this spec. |
| "Rest / Hover / Pressed / Selected / Focused / Disabled" | The standard interaction states. Every interactive element in this spec must implement all states that apply to it. |

All user-visible strings in `"double quotes"` are the exact labels to ship in en-US. Put them in the localisation catalogue (`people.*`, `todo.*`, `myday.*` keys). Never hard-code them in components.

---

## 1. Shared design tokens for this area

### 1.1 Colour tokens (light and dark)

| Token | Light | Dark | Used for (in this area) |
|---|---|---|---|
| `colorNeutralBackground1` | `#ffffff` | `#292929` | List panes, details pane, task cards, dialogs |
| `colorNeutralBackground1Hover` | `#f5f5f5` | `#3d3d3d` | Contact row hover, task row hover |
| `colorNeutralBackground1Pressed` | `#e0e0e0` | `#1f1f1f` | Row pressed |
| `colorNeutralBackground1Selected` | `#ebebeb` | `#383838` | Selected contact row (estimate: which token Outlook uses) |
| `colorNeutralBackground2` | `#fafafa` | `#1f1f1f` | People navigation pane, To Do sidebar (estimate) |
| `colorNeutralBackground3` | `#f5f5f5` | `#141414` | Canvas behind the task cards in the To Do list (estimate) |
| `colorNeutralBackground6` | `#e6e6e6` | `#333333` | Neutral avatar background (no name, or a phone number) |
| `colorSubtleBackgroundHover` | `#f5f5f5` | `#383838` | Navigation item hover, subtle buttons |
| `colorSubtleBackgroundPressed` | `#e0e0e0` | `#2e2e2e` | Navigation item pressed |
| `colorSubtleBackgroundSelected` | `#ebebeb` | `#333333` | Selected navigation item, selected smart list |
| `colorNeutralForeground1` | `#242424` | `#ffffff` | Primary text (names, task titles) |
| `colorNeutralForeground2` | `#424242` | `#d6d6d6` | Navigation labels, icons at rest |
| `colorNeutralForeground3` | `#616161` | `#adadad` | Secondary text (email, company, task subtitle, counts) |
| `colorNeutralForeground4` | `#707070` | `#999999` | Placeholder text, index headings (estimate) |
| `colorNeutralForegroundDisabled` | `#bdbdbd` | `#5c5c5c` | Disabled commands |
| `colorNeutralStroke1` | `#d1d1d1` | `#666666` | Input borders |
| `colorNeutralStroke2` | `#e0e0e0` | `#525252` | Dividers between sections and panes |
| `colorNeutralStroke3` | `#f0f0f0` | `#3d3d3d` | Hairlines inside cards |
| `colorNeutralStrokeAccessible` | `#616161` | `#adadad` | Ring of the task completion circle (estimate) |
| `colorBrandBackground` | `#0f6cbd` | `#115ea3` | Primary buttons, filled completion circle |
| `colorBrandBackgroundHover` | `#115ea3` | `#0f6cbd` | Primary button hover |
| `colorBrandBackgroundPressed` | `#0c3b5e` | `#0c3b5e` | Primary button pressed |
| `colorBrandBackground2` | `#ebf3fc` | `#082338` | Drop-target highlight, "Added to My Day" chip (estimate) |
| `colorBrandBackground2Hover` | `#cfe4fa` | `#0c3b5e` | Drop-target hover (estimate) |
| `colorBrandForeground1` | `#0f6cbd` | `#479ef5` | "Add a task" placeholder and plus icon, filled star, "Today" due label (estimate) |
| `colorBrandForeground2` | `#115ea3` | `#62abf5` | Hovered brand text |
| `colorBrandForegroundLink` | `#115ea3` | `#479ef5` | Email and phone links in the contact details pane |
| `colorCompoundBrandStroke` | `#0f6cbd` | `#479ef5` | Selection indicator bar, active tab underline |
| `colorNeutralForegroundOnBrand` | `#ffffff` | `#ffffff` | Check mark inside a filled completion circle |
| `colorPaletteRedForeground1` | `#bc2f32` | `#e37d80` | Overdue due dates (estimate: token choice) |
| `colorStrokeFocus2` | `#000000` | `#ffffff` | 2 px keyboard focus outline |
| `colorBackgroundOverlay` | `rgba(0,0,0,0.4)` | `rgba(0,0,0,0.5)` | Dialog scrim |

Shadows [S39]. Light theme uses ambient `rgba(0,0,0,0.12)` and key `rgba(0,0,0,0.14)`. Dark theme uses ambient `rgba(0,0,0,0.24)` and key `rgba(0,0,0,0.28)`.

| Token | Value pattern | Used for |
|---|---|---|
| `shadow2` | `0 0 2px A, 0 1px 2px K` | Task cards (estimate) |
| `shadow4` | `0 0 2px A, 0 2px 4px K` | Hovered task card (estimate) |
| `shadow16` | `0 0 2px A, 0 8px 16px K` | Menus, date pickers, drag ghost |
| `shadow64` | `0 0 8px A, 0 32px 64px K` | Dialogs (New contact, Delete confirmations) |

### 1.2 Type, spacing, radius, motion

From Fluent 2 global tokens [S39]. Font family: use the global Axentra font stack defined in the typography spec. Segoe UI is not available on Linux.

| Ramp name | Size / line height / weight | Used for |
|---|---|---|
| Caption1 | 12 / 16 / 400 | Secondary lines, subtitles of task rows, field labels |
| Caption1Strong | 12 / 16 / 600 | Count badges, index headings (estimate) |
| Body1 | 14 / 20 / 400 | Contact names in the list, task titles, menu items |
| Body1Strong | 14 / 20 / 600 | Selected navigation item, section headers |
| Subtitle2 | 16 / 22 / 600 | Section titles in the details pane (estimate) |
| Subtitle1 | 20 / 28 / 600 | Task title in the detail pane (estimate) |
| Title3 | 24 / 32 / 600 | Contact name in the details header, list title in To Do (estimate) |

Spacing scale: 2, 4, 6, 8, 10, 12, 16, 20, 24, 32 px. Radius scale: 2, 4, 6, 8, 12, 16 px, plus circular. Durations: 50, 100, 150, 200, 250, 300, 400, 500 ms. Curves: `curveDecelerateMid cubic-bezier(0,0,0,1)`, `curveAccelerateMid cubic-bezier(1,0,1,1)`, `curveEasyEase cubic-bezier(0.33,0,0.67,1)`, `curveEasyEaseMax cubic-bezier(0.8,0,0.2,1)` [S39]. Every animation in this spec must drop to 0.01 ms under `prefers-reduced-motion: reduce`, as Fluent's own Avatar does [S36].

---

## 2. People module

### 2.1 Entry points

| Entry | Behaviour | Source |
|---|---|---|
| App rail icon "People" (two-person silhouette) | Opens the People module and restores the last selected node. | [S1][S10] |
| `Ctrl+4` | Go to People. In new Outlook, `Ctrl+3` goes to Copilot. Axentra leaves `Ctrl+3` unassigned so that `Ctrl+4` matches muscle memory. | [S7] |
| `Ctrl+Shift+4` | Also accepted. This is the web-style binding (Outlook on the web uses `Ctrl+Shift+1` to `Ctrl+Shift+4` for Mail, Calendar, Copilot and People). | [S7] |
| Profile card "Add to contacts" (from Mail) | Opens the New contact form with name and email filled in. In new Outlook the link sits at the bottom of the profile card and the form is confirmed with "Save". On the web it sits under "More options". | [S1] |

Classic Outlook difference: `Ctrl+3` opens Contacts and `Ctrl+4` opens Tasks. Axentra follows new Outlook.

### 2.2 Layout

The page has three panes under the command bar (ribbon): a **navigation pane**, a **list pane** and a **details pane** [S3][S8].

```
+--------------------------------------------------------------------------------+
| Ribbon tabs: Home | View | Help                                                |
| [New contact v] [Edit] [Delete] [Add to favorites] [Add to list v] [Email] ... |
+----------------+---------------------------+-----------------------------------+
| NAV PANE       | LIST PANE                 | DETAILS PANE                      |
| [<<] hide      | All contacts      [By first name v]                           |
| Favorites      | [Search contacts       ]  |  (96px avatar)  Name              |
| Your contacts  | A                         |                 Job title         |
|   All contacts |  (AV) Ada Lovelace        |                 Company           |
|   All contact  |       ada@example.com     |  [Email] [Chat] [Call] [Video]    |
|     lists      |       Analytical Ltd      |  Contact information              |
|   Contacts     | B                         |  Organization                     |
|   <folders>    |  ...                      |  Notes                            |
| Groups         |                           |                                   |
| Suggested      |                           |                                   |
+----------------+---------------------------+-----------------------------------+
```

| Pane | Default width | Min / max | Notes |
|---|---|---|---|
| Navigation | 240 px (estimate) | 200 / 360 px (estimate) | Resizable splitter. A hide/show button sits at the top of the pane [S8]. Collapsed state persists per profile. |
| List | 360 px (estimate) | 280 / 640 px (estimate) | Resizable. At 640 px or wider it switches to the multi-column layout (2.5.4). |
| Details | Remaining width | 360 px minimum (estimate) | Below the minimum, the details pane slides over the list as an overlay with a "Back" button (estimate). |

Breakpoints (estimate):

- Window width under 1024 px: the details pane overlays the list.
- Window width under 720 px: the navigation pane collapses to a hamburger button in the list header.

Landmarks for screen readers. Outlook on the web exposes these regions [S8]: a banner (search box and header buttons), a "Hub navigation menu" (the tree in 2.3: contacts, contact lists, groups and the organisation directory), a navigation landmark named after the selected hub item (for example "Your contacts") that holds the contact list, a main region with the selected contact's details, and the ribbon announced as "Ribbon tabs" plus the tab name. Axentra maps them as follows: the tree gets `role="navigation"` with `aria-label="Hub navigation menu"`; the list pane gets `role="navigation"` with `aria-label` set to the selected node's name, and the list inside it has `role="grid"` (multi-column) or `role="listbox"` (single column); the details pane gets `role="main"`.

Region cycling: the documented keys are `Ctrl+F6` and `Ctrl+Shift+F6`, and the article states that `F6` is not used in the web app [S8]. Axentra binds `Ctrl+F6` / `Ctrl+Shift+F6` as documented, and also binds `F6` / `Shift+F6` as a desktop convention (Axentra decision). The cycle order is ribbon, hub navigation, list, details.

### 2.3 Navigation pane

#### 2.3.1 Tree (top to bottom)

| # | Node label | Icon (Fluent System Icons, 20 px) | Shows | Availability | Source |
|---|---|---|---|---|---|
| 1 | "Favorites" | `Star` | Contacts and contact lists marked as favourite | All accounts | [S2][S3][S4] |
| 2 | "Your contacts" (per-account section header, expandable) | `PeopleList` (estimate) | Header only. Selecting it behaves like "All contacts" for that account | All accounts | [S8][S1] |
| 2a | "All contacts" | `Person` (estimate) | Union of every contact folder in the account | All | [S2][S4][S9] |
| 2b | "All contact lists" | `PeopleTeam` (estimate) | Every personal contact list | All | [S4][S5] |
| 2c | "Contacts" | `Folder` | The default contacts folder | All | [S3] |
| 2d | User folders (user-named) | `Folder` | One node per contact folder; nested folders are indented 16 px per level (estimate) | Exchange, CardDAV address books | [S1] |
| 2e | Categories (tags), optional | Coloured category dot 12 px | Contacts carrying that category | Exchange | [S3] |
| 3 | "Groups" (expandable) | `PeopleCommunity` (estimate) | Microsoft 365 groups. Children are "Members" (groups you belong to) and "Owners" (groups you own). The plural labels come from the older web article; current new Outlook wording is (unverified) | Work/school Exchange only | [S3] |
| 4 | "Suggested" | `PersonLightbulb` (estimate) | People you exchanged mail with who are not saved as contacts | All (local index) | Axentra decision, modelled on classic Outlook's Suggested Contacts and the web's "Frequently contacted" view [S9]. **Off by default** (see 2.12) |
| 5 | "Directory" (optional) | `Building` (estimate) | Organisation address book: global address lists, meeting rooms and distribution lists | Work/school Exchange only | [S8][S10] |

The web help also calls the contact-list node "Your contact lists" [S3]. Outlook.com and the contact-list help use "All contact lists" [S4][S5]. Axentra ships "All contact lists".

Multiple accounts: "Favorites" stays at the top and spans all accounts. Each account then gets its own "Your contacts" section, headed by the account's email address in Caption1Strong, with its own children (estimate).

#### 2.3.2 Navigation item anatomy and states

- Height 32 px (estimate). Horizontal padding 12 px (estimate). Icon 20 px with an 8 px gap before the label. Label is Body1 in `colorNeutralForeground2`, with ellipsis when truncated and a tooltip showing the full name.
- Section headers ("Your contacts", "Groups") have a 12 px chevron on the left (`ChevronRight` collapsed, `ChevronDown` expanded). The chevron rotates over 150 ms on `curveEasyEase` (estimate).
- People nodes show no unread or count badges (estimate). A count appears only in the list pane header.

| State | Background | Text / icon | Extra |
|---|---|---|---|
| Rest | transparent | `colorNeutralForeground2` | |
| Hover | `colorSubtleBackgroundHover` | `colorNeutralForeground1` | |
| Pressed | `colorSubtleBackgroundPressed` | `colorNeutralForeground1` | |
| Selected | `colorSubtleBackgroundSelected` | `colorNeutralForeground1`, Body1Strong, filled icon variant | A 3 x 16 px pill in `colorCompoundBrandStroke`, radius circular, 4 px from the left edge (estimate) |
| Focused (keyboard) | as current state | as current state | 2 px `colorStrokeFocus2` outline inset 2 px, radius 4 px |
| Drop target (dragging contacts) | `colorBrandBackground2` | | Only folders and contact lists accept drops |

Keyboard: `Up`/`Down` move, `Right` expands or enters a child, `Left` collapses or goes to the parent, and `Enter`/`Space` select. Selecting a node moves the list pane to that view but leaves focus in the navigation pane.

#### 2.3.3 Navigation context menus

| Target | Items (in order) | Source |
|---|---|---|
| "Your contacts" | "New folder" | [S1] |
| User folder | "New subfolder" (estimate), "Rename folder" (estimate), "Delete folder" (estimate) | |
| "Contacts" (default) | "New subfolder" (estimate). It cannot be renamed or deleted. | |
| A category node | "Remove category from all contacts" (estimate) | Categories are deleted by removing them from every contact [S2] |

"New folder" adds an inline text box with the default text "New folder" selected (estimate). `Enter` commits, `Esc` cancels, and an empty name cancels. Deleting a folder opens a confirmation dialog that lists how many contacts will be deleted (estimate).

### 2.4 Commands (ribbon Home tab and context menus)

Ribbon tabs are "Home", "View" and "Help" [S8]. Commands change with the selection [S3].

| Command label | Icon | Shortcut | Enabled when | Behaviour | Source |
|---|---|---|---|---|---|
| "New contact" (split button) | `PersonAdd` | `Ctrl+N` | Always | Opens the New contact form (2.8) in the selected folder, or "Contacts" if no folder is selected | [S1][S7] |
| ▾ "New contact list" | `PeopleAdd` | `Ctrl+L` | Always | Opens the contact list editor (2.9) | [S1][S7][S10] |
| "Edit" | `Edit` | `Shift+E` | One editable item selected | Puts the details pane into edit mode (same fields as 2.8) | [S1][S7] |
| "Delete" | `Delete` | `Delete` or `Ctrl+D` | One or more deletable items selected | Confirmation dialog (2.14) | [S1][S7] |
| "Add to favorites" / "Remove from favorites" | `Star` / `StarOff` | none (estimate) | One or more contacts or lists selected. Shows the remove label if every selected item is already a favourite | Toggles the favourite flag | [S2][S4] |
| "Add to list" (menu) | `PeopleList` | none | One or more contacts **with an email address** selected (contacts without one can't join a list [S4]) | Menu shows each contact list. Lists the contact is not in show a plus button; lists it is already in show an "X" that removes it [S4]. Footer item: "New contact list" creates a list seeded with the selection [S4][S5] | [S4][S5] |
| "Email" | `Mail` | none (estimate) | Selection has at least one email address | New message addressed to every selected contact. Contact lists expand to their members when sent | [S50] |
| "Chat" | `Chat` | none | One contact with an IM address and a chat handler configured | See 2.7.2 | Axentra decision |
| "Add Category" (menu) | `Tag` | none | One or more Exchange contacts selected | Lists categories with check marks, then "New category". The ribbon label in the current new Outlook People hub is "Add Category"; on the web the same menu is "Categorize" under "More options" (also reachable by hovering a row and choosing "More options") [S2]. Axentra uses "Add Category" on the ribbon and "Categorize" in context menus | [S2] |
| "Import contacts" | `ArrowImport` | none | Always | Import dialog (2.13). New Outlook puts this button directly on the ribbon [S6] | [S6] |
| "Export contacts" | `ArrowExport` | none | Account has at least one contact | Export dialog (2.13). New Outlook puts this button directly on the ribbon [S6] | [S6] |
| "Manage contacts" (menu, web layout only) | `Settings` | none | Always | On Outlook on the web, import and export sit under "Manage contacts" [S6]. Axentra shows the two buttons directly (new Outlook layout) and does not ship this menu unless the ribbon is in its simplified, narrow form (estimate). "Manage sources" is **not** a ribbon item: it lives on the profile card / details pane (2.7.1) [S11] | [S6][S11] |

View tab (estimate): "Sort by" (menu, same items as 2.5.2), "Display contacts by" ("First name" / "Last name") [S3][S4], "Show navigation pane" (toggle), "Density" ("Comfortable" / "Compact").

Keyboard shortcuts in People, from the new Outlook table [S7]:

| Action | Shortcut |
|---|---|
| Create contact | `Ctrl+N` |
| Create contact list | `Ctrl+L` |
| Edit contact | `Shift+E` |
| Delete contact | `Delete` or `Ctrl+D` |
| Save contact (in form) | `Ctrl+S` |
| Discard contact / unselect all | `Esc` |
| Search contacts | `Ctrl+E` |
| Select all | `Ctrl+A` |
| Shortcut list | `?` opens the full list; `Shift+?` inside People shows the People shortcuts |
| Home tab / View tab KeyTips | `Alt`, `H` / `Alt`, `V` (unverified) |

Outlook on the web uses different People bindings: `Shift+N` (new contact), `Shift+L` (new contact list), `Delete` only (no `Ctrl+D`) and `Alt+Q` (search) [S7]. Axentra follows the new Outlook table above. Flag/unflag in Mail is `Insert`; on a flagged message the same key marks the flag complete [S7].

The context menu on a contact row lists, in order: "Edit", "Add to favorites"/"Remove from favorites", "Add to list" ▸, "Email", "Chat", "Categorize" ▸, separator, "Export contact (.vcf)" (Axentra decision), separator, "Delete". Order is (estimate); right-click "Add to favorites" is documented [S2].

### 2.5 Contact list pane

#### 2.5.1 Header

- Row 1, 44 px (estimate): view title in Subtitle2 (for example "All contacts", "Favorites" or a folder name), then the item count in Caption1, `colorNeutralForeground3`, e.g. "248" (estimate). On the right, a sort and filter button whose label shows the current sort, e.g. "By first name", with a chevron [S2][S3].
- Row 2, 40 px (estimate): search box, placeholder "Search contacts", magnifier icon, clear button. `Ctrl+E` focuses it [S7]. Filtering is live. Debounce 150 ms (estimate). Matching is case- and accent-insensitive on display name, given name, surname, nickname, every email, company, every phone (digits only) and notes (estimate).

#### 2.5.2 Sorting and display

"Sort by" options [S2]. Microsoft documents two sets. The established new Outlook and the web list first name, last name, company, home city, work city and recently added. The newer People hub (documented for the Insiders build) lists only first name, last name, company and recently added. Axentra ships the four-option set as the default menu, and keeps "Home city" and "Work city" behind the View tab as Axentra extras (Axentra decision). The help describes the options in prose, so the exact menu-item capitalisation is (estimate).

| Option | Sort key | Index headings |
|---|---|---|
| "First name" (default) | givenName, then surname, then display name | First grapheme of the key |
| "Last name" | surname, then givenName | First grapheme of the key |
| "Company" | companyName; empty values sort last under "#" (estimate) | First grapheme of company |
| "Recently added" | createdDateTime, newest first | Date buckets: "Today", "Yesterday", "This week", "This month", "Older" (estimate) |
| "Home city" (older builds only; Axentra extra) | homeAddress.city | First grapheme of city |
| "Work city" (older builds only; Axentra extra) | businessAddress.city | First grapheme of city |

The button label follows the pattern "By first name", "By last name", "By company", "By home city", "By work city", "By recently added" (estimate, extrapolated from the documented "By first name" [S2]).

"Display contacts by" [S3] (documented for Outlook on the web under Settings; the new Outlook article documents no such option [S2], so its placement on the View tab is an estimate):

- "First name": rows show `Given Middle Surname`.
- "Last name": rows show `Surname, Given`.

If a contact has neither name, use `companyName`, then the primary email, then the first phone number, then the literal "(No name)" (estimate).

Collation: use ICU collation for the UI locale (for example the `icu_collator` crate). Never sort by byte value.

#### 2.5.3 Alphabetical index headings

- Sticky header row, 28 px (estimate). Text is Caption1Strong in `colorNeutralForeground3` with 12 px left padding (estimate). The text is the uppercase first grapheme of the sort key.
- Digits, symbols and empty keys group under "#", which always comes last (estimate). Latin letters with diacritics fold to their base letter (so "Á" goes under "A"), using locale rules (estimate). Scripts without case (CJK, Arabic, Hangul) use the first grapheme as is.
- Clicking a heading opens a jump grid. The grid is a flyout of letter buttons, 4 columns x 7 rows, that contains only the letters present plus "#". Picking a letter scrolls to that heading (estimate). This is an Axentra decision.
- Type-ahead: with focus in the list, typing a printable character moves to the first row whose sort key starts with it. Within 1 s (estimate) further keystrokes extend the prefix.

#### 2.5.4 Row anatomy

Single-column layout (list pane under 640 px):

```
| [ ] (32px avatar)  Display name                    (star on hover) |
|                    primary@email.example                           |
|                    Company                                         |
```

- Row height is 64 px with three lines in Comfortable density, or 48 px with two lines (name and email; company dropped) in Compact (estimate). Horizontal padding 12 px, gap between avatar and text 12 px (estimate).
- Line 1: display name, Body1, `colorNeutralForeground1`, single line with ellipsis.
- Line 2: primary email, Caption1, `colorNeutralForeground3`.
- Line 3: company, Caption1, `colorNeutralForeground3`. The line is omitted if empty and the row stays the same height (estimate).
- Contact lists show a square avatar (2.6.5) and "N members" instead of the email.
- Selection checkbox: a 16 px circular checkbox that replaces the avatar on hover, or shows for every row while a multi-selection is active (estimate; this follows the new Outlook message list pattern).
- Favourite star: 16 px, right-aligned, shown on hover or when the contact is a favourite. Clicking it toggles the favourite (estimate).

Multi-column layout (list pane 640 px or wider): this matches the "modern, multi-column" People Hub that Microsoft rolled out to Outlook on the web in the first half of 2026, together with automatic linking of duplicate contacts [S12]. The announcement names no columns, so the column set below is an estimate. Columns: "Name" (avatar plus display name), "Email", "Company", "Phone" (mobile, else business). Clicking a column header sorts by that column, and a second click reverses the order (estimate). Rows are 40 px with a 24 px avatar (estimate).

| State | Background | Notes |
|---|---|---|
| Rest | `colorNeutralBackground1` | |
| Hover | `colorNeutralBackground1Hover` | Checkbox and star appear |
| Pressed | `colorNeutralBackground1Pressed` | |
| Selected | `colorNeutralBackground1Selected` | 3 px `colorCompoundBrandStroke` bar at the left edge (estimate) |
| Selected + hover | `colorNeutralBackground1Hover` blended over the selected state (estimate) | |
| Focused | Unchanged | 2 px `colorStrokeFocus2` inset outline |
| Read-only source (directory or linked) | Unchanged | 12 px lock badge after the name, tooltip "Managed by your organization" (estimate) |

Selection model:

- Click selects one row and loads its details.
- `Ctrl+click` toggles a row. `Shift+click` selects a range.
- `Ctrl+A` selects all and `Esc` clears the selection [S7].
- `Up`/`Down`, `Home`/`End` and `PageUp`/`PageDown` move focus and selection.
- `Space` toggles the focused row in multi-select.
- With 2 or more rows selected, the details pane shows "N contacts selected" and big buttons for "Email", "Add to list", "Add to favorites", "Categorize" and "Delete" (estimate).

Virtualisation is required. Render only the visible rows plus 10 rows above and below (estimate). The list must stay smooth at 50,000 contacts.

#### 2.5.5 Empty states

Use original Axentra wording. Do not use Microsoft artwork.

| View | Title | Body | Action button |
|---|---|---|---|
| All contacts (none) | "No contacts yet" | "People you add will show up here." | "New contact" |
| Favorites | "No favorites yet" | "Select a contact and choose Add to favorites to keep them close." | none |
| All contact lists | "No contact lists" | "Group people into a list to email them all at once." | "New contact list" |
| Folder (empty) | "This folder is empty" | none | "New contact" |
| Groups (unsupported account) | "Groups aren't available for this account" | none | none |
| Search, no hits | "No results for "{query}"" | "Check the spelling or try a different name." | none |

### 2.6 Avatars (initials, colours, photos)

Axentra copies the deterministic rules of the Fluent UI React v9 `Avatar` component (MIT) [S34][S35][S36]. Using the same algorithm keeps colours stable across devices and sessions.

#### 2.6.1 Sizes used in this area

| Context | Size | Initials font (from Fluent's size rule [S36]) |
|---|---|---|
| Favourite people in the Mail folder pane, chips, people picker | 20-24 px | 10 px, weight 600 |
| Multi-column list row | 24 px (estimate) | 10 px, weight 600 |
| Single-column list row | 32 px (estimate) | 14 px, weight 600 |
| Contact list member rows | 32 px (estimate) | 14 px, weight 600 |
| Details pane header | 96 px (estimate) | 20 px, weight 600 |
| New/Edit contact photo picker | 72 px (estimate) | 20 px, weight 600 |

Fluent's rule for initials font size [S36]: size 24 px or less uses 10 px; 28 px or less uses 12 px; 40 px or less uses 14 px; 56 px or less uses 16 px; 96 px or less uses 20 px; anything larger uses 24 px. Weight is always semibold (600). At 16 px or below, only the first initial is shown [S34].

#### 2.6.2 Initials algorithm

Port this exactly from Fluent `getInitials` [S35]:

1. If the display name is empty, there are no initials. Show the person icon on the neutral colour.
2. Remove any bracketed enclosures: `(...)`, `[...]` and `{...}`. An unmatched opening bracket also removes everything after it. For example, "Ada Lovelace (Contoso)" becomes "Ada Lovelace".
3. Remove these characters: ASCII control characters; ASCII punctuation in the ranges `!` to `/`, `:` to `@`, `[` to `` ` ``, and `{` to U+00BF; U+0250 to U+036F; U+E000 to U+FFFF. This also strips the hyphen, so "Jean-Luc Picard" becomes "JeanLuc Picard".
4. Collapse runs of whitespace to one space, then trim.
5. If the first code point is Arabic, Hangul, Hiragana, Katakana or a BMP CJK ideograph, there are no initials, so show the person icon. Supplementary-plane CJK characters (U+20000 and up) are allowed.
6. If what remains looks like a phone number (only digits and spaces, optionally followed by `ext` or `x` and more digits), there are no initials.
7. Split on single spaces. The initials are the uppercased first code point of word 1, plus:
   - 2 words: the first code point of word 2.
   - 3 words: the first code point of word 3 (the middle word is skipped).
   - 4 or more words: nothing more. Only one initial is shown.
8. In RTL UI, swap the two characters.

Examples: "Ada Lovelace" gives "AL". "Mary Ann Smith" gives "MS". "Juan Carlos de la Vega" gives "J". "+1 555 0100" gives the icon. "ada@example.com" gives "A".

Source of the display name: the contact's `displayName`, else the given name and surname joined, else the company, else the email local part.

#### 2.6.3 Colour assignment (deterministic palette)

The colour key is the string that gets hashed:

- If the contact has a primary SMTP address, use it, trimmed and lowercased.
- Otherwise use the display name.

This is an **Axentra decision**. Fluent hashes `idForColor`, falling back to `name` [S34]. A key based on the email address keeps the colour stable when someone renames the contact. Contact lists and groups use their object ID as the key.

Two more Axentra decisions that Fluent does not make:

- Fluent's `Avatar` defaults to `color="neutral"`; the palette is used only when the caller asks for `colorful` [S34]. Axentra always uses the colourful palette for people with initials.
- Normalise the key to Unicode NFC before hashing. Fluent hashes whatever string it is given, so the same name typed in decomposed form (NFD) gets a different colour: "José Núñez" hashes to 19210 (index 10, seafoam) in NFC but to 5203 (index 13, teal) in NFD.

Hash, ported from Fluent `getHashCode` [S34]:

```rust
/// Must iterate UTF-16 code units (JS charCodeAt), NOT chars/bytes.
pub fn fluent_hash(s: &str) -> u32 {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut h: u32 = 0;
    for i in (0..units.len()).rev() {
        let ch = units[i] as u32;
        let shift = (i % 8) as u32;
        h ^= (ch << shift).wrapping_add(ch >> (8 - shift));
    }
    h
}
pub fn avatar_color_index(key: &str) -> usize { (fluent_hash(key) % 30) as usize }
```

Every intermediate value stays below 2^24, so `u32` arithmetic gives the same result as JavaScript. Test vectors (computed by this spec's author with the algorithm above, and recomputed independently during the 2026-10-19 fact-check with identical results; keys are NFC):

| Key | Hash | Index | Colour |
|---|---|---|---|
| `""` | 0 | 0 | dark-red (but an empty key always uses the neutral colour; see below) |
| `Ada Lovelace` | 9219 | 9 | forest |
| `ada.lovelace@example.com` | 8611 | 1 | cranberry |
| `Grace Hopper` | 10076 | 26 | beige |
| `Alan Turing` | 12203 | 23 | pink |
| `José Núñez` | 19210 | 10 | seafoam |
| `李雷` | 84142 | 22 | lilac |
| `José Núñez` in NFD (must not occur after normalisation) | 5203 | 13 | teal |

The palette is in Fluent order and has exactly 30 entries. **Do not reorder it** [S34]. In light theme, background = `colorPalette{Name}Background2` (the tint40 shade) and initials = `colorPalette{Name}Foreground2` (shade30). Dark theme swaps them: background = shade30 and initials = tint40. Two exceptions in dark theme: dark-red and plum use shade20 for the background [S37].

| # | Name | Light bg | Light fg | Dark bg | Dark fg |
|---|---|---|---|---|---|
| 0 | dark-red | `#d69ca5` | `#420610` | `#590815` | `#d69ca5` |
| 1 | cranberry | `#eeacb2` | `#6e0811` | `#6e0811` | `#eeacb2` |
| 2 | red | `#f1bbbc` | `#751d1f` | `#751d1f` | `#f1bbbc` |
| 3 | pumpkin | `#efc4ad` | `#712d09` | `#712d09` | `#efc4ad` |
| 4 | peach | `#ffddb3` | `#8f4e00` | `#8f4e00` | `#ffddb3` |
| 5 | marigold | `#f9e2ae` | `#835b00` | `#835b00` | `#f9e2ae` |
| 6 | gold | `#ecdfa5` | `#6c5700` | `#6c5700` | `#ecdfa5` |
| 7 | brass | `#e0cea2` | `#553e06` | `#553e06` | `#e0cea2` |
| 8 | brown | `#ddc3b0` | `#50301a` | `#50301a` | `#ddc3b0` |
| 9 | forest | `#bdd99b` | `#294903` | `#294903` | `#bdd99b` |
| 10 | seafoam | `#a8f0cd` | `#00723b` | `#00723b` | `#a8f0cd` |
| 11 | dark-green | `#9ad29a` | `#063b06` | `#063b06` | `#9ad29a` |
| 12 | light-teal | `#a6e9ed` | `#00666d` | `#00666d` | `#a6e9ed` |
| 13 | teal | `#9bd9db` | `#02494c` | `#02494c` | `#9bd9db` |
| 14 | steel | `#94c8d4` | `#00333f` | `#00333f` | `#94c8d4` |
| 15 | blue | `#a9d3f2` | `#004377` | `#004377` | `#a9d3f2` |
| 16 | royal-blue | `#9abfdc` | `#002c4e` | `#002c4e` | `#9abfdc` |
| 17 | cornflower | `#c8d1fa` | `#2c3c85` | `#2c3c85` | `#c8d1fa` |
| 18 | navy | `#a3b2e8` | `#001665` | `#001665` | `#a3b2e8` |
| 19 | lavender | `#d2ccf8` | `#3f3682` | `#3f3682` | `#d2ccf8` |
| 20 | purple | `#c6b1de` | `#341a51` | `#341a51` | `#c6b1de` |
| 21 | grape | `#d9a7e0` | `#4c0d55` | `#4c0d55` | `#d9a7e0` |
| 22 | lilac | `#e6bfed` | `#63276d` | `#63276d` | `#e6bfed` |
| 23 | pink | `#f7c0e3` | `#80215d` | `#80215d` | `#f7c0e3` |
| 24 | magenta | `#eca5d1` | `#6b0043` | `#6b0043` | `#eca5d1` |
| 25 | plum | `#d696c0` | `#43002b` | `#5a003b` | `#d696c0` |
| 26 | beige | `#d7d4d4` | `#444241` | `#444241` | `#d7d4d4` |
| 27 | mink | `#cecccb` | `#343231` | `#343231` | `#cecccb` |
| 28 | platinum | `#cdd6d8` | `#3b4447` | `#3b4447` | `#cdd6d8` |
| 29 | anchor | `#bcc3c7` | `#202427` | `#202427` | `#bcc3c7` |

Neutral avatar: used when there are no initials or the key is empty. Background is `colorNeutralBackground6` (`#e6e6e6` / `#333333`). The icon is `Person` in `colorNeutralForeground3` (`#616161` / `#adadad`) [S36]. Icon size by avatar size, from Fluent's style rules [S36]: 16 px or less uses 12 px; 24 px or less uses 16 px; 40 px or less uses 20 px; 48 px or less uses 24 px; 56 px or less uses 28 px; 72 px or less uses 32 px; larger uses 48 px. So the 32 px list avatar gets a 20 px icon, the 72 px form avatar 32 px, and the 96 px header avatar 48 px.

High-contrast mode (`forced-colors: active`): background `Canvas`, 1 px border `CanvasText`, initials `CanvasText`. This is an **Axentra decision**: Fluent's Avatar styles contain no forced-colors rule [S36].

#### 2.6.4 Photos and precedence

Use the first one that exists:

1. The contact's own photo (Graph `photo`, vCard `PHOTO`).
2. The directory or linked-source photo.
3. Initials on the palette colour.
4. The neutral person icon.

The photo is a circle and is `object-fit: cover`. Fade it in over 150 ms once decoded. Never flash initials and then swap to the photo once it has loaded and been cached (estimate). Photo editing in the form: choose a file, then drag to reposition inside a circle mask with a zoom slider, then "Apply" [S3][S4]. Accepted formats are JPEG, PNG and WebP. Re-encode to a 648 x 648 JPEG (estimate) before upload.

#### 2.6.5 Lists and groups

Fluent defines group avatars as **square** [S40]. Contact lists and Microsoft 365 groups use `shape=square` with radius by size [S36]: 24 px or less gets 2 px, 48 px or less gets 4 px, 72 px or less gets 6 px, larger gets 8 px. Content is the initials of the list name on its palette colour. If the list name yields no initials, show the `People` icon (estimate).

Presence badges are not used: Axentra has no presence service. Fluent notes that badges are hard to see at 32 px and below anyway [S40].

### 2.7 Contact details pane

#### 2.7.1 Header

```
(96px avatar)   Display name                         [Edit] [star] [...]
                Job title
                Company · Department
[ Email ]  [ Chat ]  [ Call ]  [ Video ]
```

- Padding is 24 px (estimate). The avatar is 96 px with a 16 px gap to the text (estimate).
- Name: Title3 (24/32/600), `colorNeutralForeground1`. It wraps to at most 2 lines and is selectable.
- Job title: Body1, `colorNeutralForeground2`. Omitted if empty.
- Company and department: Body1, `colorNeutralForeground3`, joined with " · " (estimate). Omitted if both are empty.
- Top-right buttons:
  - "Edit" is a subtle button with icon and text. It is the "Edit contact" entry point [S3][S4]. It is hidden for read-only sources [S1].
  - The star toggle has tooltip "Add to favorites" or "Remove from favorites".
  - "More options" (`...`) opens: "Add to list" ▸, "Categorize" ▸, "Manage sources" [S11], "Export contact (.vcf)" (Axentra decision), separator, "Delete".

#### 2.7.2 Action buttons

There are 4 buttons, each 64 x 56 px with a 20 px icon above a Caption1 label and radius 4 px. They are subtle buttons with hover `colorSubtleBackgroundHover` (estimate). The web People hub advertises quick call, message and email actions for contacts [S50]. Axentra provides:

| Label | Icon | Action | Disabled when / tooltip |
|---|---|---|---|
| "Email" | `Mail` | Opens a new message to the primary email. If there are 2 or more addresses, a menu lets the user pick one | No email: "No email address" |
| "Chat" | `Chat` | Opens the contact's first IM address (`imAddresses`) with the desktop's registered URI handler (`xdg-open`). Supports `sip:`, `xmpp:`, `im:` and `msteams:` | No IM address or no handler: "No chat app available" (Axentra decision) |
| "Call" | `Call` | Opens a `tel:` URI. If there are 2 or more numbers, a menu shows "Mobile", "Work" and "Home" with the numbers | No phone: "No phone number" |
| "Video" | `Video` | Opens the configured meeting app with the contact's address | No meeting app configured: "No video app available" (Axentra decision) |

Disabled buttons use `colorNeutralForegroundDisabled`. They still get keyboard focus (`aria-disabled="true"`) so the tooltip can be read.

#### 2.7.3 Sections

Sections are stacked cards (estimate), each with a Subtitle2 header and 16 px padding. A field row is a Caption1 label in `colorNeutralForeground3` above a Body1 value. Rows are 16 px apart (estimate). Empty fields are hidden, and a section with no fields is hidden. Each value has a "Copy" icon button that appears on hover or focus (estimate).

| Section header | Fields, in display order | Value behaviour |
|---|---|---|
| "Contact information" | "Email" (up to 3), "Mobile", "Work phone" (each business phone), "Home phone" (each), "IM address", "Home address", "Work address", "Other address", "Birthday", "Website", "Nickname", "Spouse/partner", "Children" | Emails are links in `colorBrandForegroundLink` that open compose. Phones are `tel:` links. Addresses span several lines; an "Open map" link is optional (estimate). Birthday uses the locale long date, without a year if none was given. Website opens in the browser after confirming the scheme is `http` or `https` |
| "Organization" | "Company", "Job title", "Department", "Office location", "Manager", "Assistant", "Profession" | Plain text. For directory contacts, Manager links to that person's card |
| "Notes" | Personal notes, as multiline plain text | Shows 6 lines, then a "Show more" toggle (estimate) |
| "Recent emails" (optional, phase 2) | The last 5 messages to or from this person, from the local index | Clicking one opens it in Mail. This mirrors the web's Conversations/Emails tab [S3][S4] |

**LinkedIn is deliberately absent.** Classic Outlook and the web show a "LinkedIn" tab or section [S1][S3][S4]. Axentra ships no LinkedIn integration and has no placeholder for it. Classic and web also show "Files" [S3][S4]. Axentra omits it too, because it needs OneDrive/SharePoint.

The contact list variant of the pane shows a square avatar, the list name, "N members" and the description. Its buttons are "Email" (to all members), "Edit" and "Delete". Below that is the member list (32 px avatar, name, email). Clicking a member opens that contact (estimate).

The group variant (Microsoft 365 group) shows a square avatar, the name, the description, and "Member" or "Owner" as the user's role. Its only button is "Email". Read-only (estimate).

### 2.8 New contact / Edit contact form

Shown as a modal dialog: 600 px wide, maximum height 85% of the window, scrollable body, radius 8 px, `shadow64`, over the `colorBackgroundOverlay` scrim (estimate). Title: "New contact" or "Edit contact". The same field layout is used inline when the details pane is in edit mode.

Top of form: a 72 px avatar with a camera button. Its label is "Add photo", or "Change photo" once a photo exists (estimate for both). Its menu has "Upload a photo" [S4] (the older web article says "Upload a new photo" [S3]) and "Remove photo" (estimate; no source documents photo removal). After cropping, the documented confirm button is "Apply" [S3][S4].

Fields in order:

| # | Label | Control | Maps to (Graph contact [S33] / vCard 4.0 [S41]) | Notes |
|---|---|---|---|---|
| 1 | "First name" | Text | `givenName` / `N` given | Autofocus |
| 2 | "Last name" | Text | `surname` / `N` family | |
| 3 | "Email" | Email text plus "Add email" link | `emailAddresses[0..3]`, mirrored by the v1.0 properties `primaryEmailAddress`, `secondaryEmailAddress`, `tertiaryEmailAddress` / `EMAIL` | At most 3, because Exchange contacts have exactly three email slots (primary, secondary, tertiary) [S33]. Validate as an RFC 5322 addr-spec (simplified); show the inline error "Enter a valid email address" (estimate) |
| 4 | "Phone" | Type dropdown ("Mobile", "Work", "Home") plus text, then "Add phone number" | `mobilePhone`, `businessPhones[]`, `homePhones[]` / `TEL;TYPE=cell, work, home` | Keep the text as typed. No forced formatting |
| 5 | "Address" | Type dropdown ("Home", "Work", "Other"), then "Street address", "City", "State/Province", "ZIP/Postal code", "Country/Region", then "Add address" | `homeAddress`, `businessAddress`, `otherAddress` (fields: street, city, state, postalCode, countryOrRegion) / `ADR` | One address of each type |
| 6 | "Birthday" | Month dropdown, Day dropdown, Year (optional) | `birthday` (DateTimeOffset, UTC) [S33] / `BDAY` | No-year birthdays: vCard 4.0 uses the reduced form `BDAY:--MMDD` [S41]. vCard 3.0 has no such form; Apple clients write year 1604 plus an `X-APPLE-OMIT-YEAR=1604` parameter (unverified). Graph has no "no year" flag; storing year 1604 there copies the Apple convention and is (unverified) for Outlook. Always send the Graph value as midnight UTC on the chosen date and read back only the date part, so it never shifts a day across time zones |
| 7 | "Company" | Text | `companyName` / `ORG` component 1 | |
| 8 | "Job title" | Text | `jobTitle` / `TITLE` | |
| 9 | "Department" | Text | `department` / `ORG` component 2 | |
| 10 | "Notes" | Multiline, 4 rows, grows to 12 | `personalNotes` / `NOTE` | Plain text |
| 11 | "More fields..." | Disclosure link | Reveals the fields below | The web uses "Add more" for this concept [S1][S4] |

Fields revealed by "More fields..." (estimate order): "Middle name" (`middleName`), "Nickname" (`nickName` / `NICKNAME`), "Title" (prefix, `title`), "Suffix" (`generation`), "Phonetic first name", "Phonetic last name" and "Phonetic company" (`yomiGivenName`, `yomiSurname`, `yomiCompanyName`), "Office location" (`officeLocation`), "Manager" (`manager`), "Assistant" (`assistantName`), "Profession" (`profession` / `ROLE`), "Website" (`businessHomePage` / `URL`), "IM address" (`imAddresses` / `IMPP`), "Spouse/partner" (`spouseName` / `RELATED;TYPE=spouse`), "Children" (`children` / `RELATED;TYPE=child`), "File as" (`fileAs`).

vCard detail: `RELATED` takes a URI by default, so a plain name must be written as `RELATED;TYPE=spouse;VALUE=text:Jane Doe` [S41]. `RELATED`, `KIND` and `MEMBER` exist only in vCard 4.0 (see 2.15 for vCard 3.0 servers).

Graph detail: Graph may overwrite `displayName` with a generated value when other name fields change. Always send the current `displayName` in every contact update [S33].

Footer: the primary button is "Save" [S1] and the secondary is "Cancel". The current new Outlook and web instructions both end with "Save" [S1]. The older web article and the Outlook.com article say "Create" for a new contact [S3][S4], and classic Outlook differs again (2.16). Axentra uses "Save" for both new and edit, following new Outlook.

Rules:

- Save stays disabled until at least one of first name, last name, company or email is filled in (estimate).
- `Ctrl+S` saves and `Esc` discards [S7].
- If the form has changes, `Esc` or "Cancel" opens the confirmation "Discard changes?", with buttons "Discard" and "Keep editing" (estimate wording).
- If a contact with the same primary email already exists, an inline banner reads "A contact with this email already exists." with an "Open contact" link. Saving is still allowed (estimate).
- Contacts created from a message go to the default "Contacts" folder. The web help adds that such a contact can't be moved to another folder later [S1]. Axentra allows the move (Axentra decision), because Graph and CardDAV both support it.
- Contacts from read-only sources can't be edited. The web help says Edit and Delete are missing for contacts that come from Skype for Business or a connected social network account [S1]; Axentra treats directory (GAL) entries the same way. For organisation-enriched contacts, saving creates an auto-linked contact, and notes or birthdays the user adds are visible only to them [S2].

### 2.9 Contact lists

A contact list is a personal list of people, used to email everyone at once. Classic Outlook calls these "Contact Groups" [S5].

Editor dialog (same frame as 2.8). The title is "New contact list" or "Edit contact list" [S1][S5]:

| # | Label | Control | Notes |
|---|---|---|---|
| 1 | "List name" (estimate label; the help only says to enter a name [S5][S10]) | Text, required | Save stays disabled while it is empty |
| 2 | "Add email addresses" [S10] | People picker that accepts a name or a raw email | Suggests saved contacts, recent recipients and the directory. `Enter`, `,` or `;` commits a chip (estimate; not documented) |
| 3 | "Description" (estimate label) | Multiline, optional | A description field is documented [S5][S10] |
| 4 | Member list | Rows with a 32 px avatar, name and email, plus a remove "x" button labelled "Remove {name}" | Removal uses the "x" beside each entry [S3][S4] |

Footer: "Create" for a new list or "Save" when editing [S3][S5][S10], plus "Cancel".

Behaviour:

- Lists are stored in the default "Contacts" folder [S4].
- In new Outlook the documented way to start a list is to select a contact, choose "Add to list", then "New contact list" [S5]; the "New contact" split button route is documented for Outlook.com and the web [S3][S4][S10]. Axentra offers both.
- "Add to list" on a contact toggles membership without opening the editor [S4]. Only contacts that have an email address can be added [S4].
- Deleting a list never deletes the member contacts [S5].
- Members that are raw emails (not saved contacts) show initials coloured by the email key.
- Duplicate addresses are ignored, and a toast reads "{email} is already in this list" (estimate).
- Limits are not documented. Axentra warns at 500 members (estimate).

### 2.10 Favorites

- Toggled from the ribbon, the context menu, the hover star or the details header [S2][S4].
- The "Favorites" view orders entries by user drag order. Entries never reordered are appended alphabetically (estimate). Dragging inside Favorites shows a 2 px `colorCompoundBrandStroke` insertion line.
- No public API carries Outlook's favourite flag (the Graph `contact` resource has no favourite property in v1.0 or beta [S33][S62]). Store it in Axentra's local store keyed by the contact's immutable ID. To roam it between Axentra installs on Microsoft 365 accounts, also write it to a Graph open extension on the contact (for example `com.axentra.people` with `{ "favorite": true, "order": n }`) [S33]; Outlook ignores that extension, so favourites set in Axentra won't appear in Outlook and vice versa. Microsoft lists "User Configuration objects" on its Graph parity roadmap for Q4 2026 [S51]; if that ships it may expose Outlook's own favourites. Re-check then.

### 2.11 Groups

- Shown only for work or school Exchange accounts. Child nodes: "Members" and "Owners" [S3].
- Rows show a square avatar, the group name and its email.
- Creating, editing and managing groups is out of scope for v1. The details pane offers "Email" only (estimate).

### 2.12 Suggested (Axentra decision)

- This node is populated from the local mail index. It lists people the user has sent mail to or received replies from in the last 180 days who are not saved as contacts (estimate thresholds). Rows are sorted by interaction count.
- Each row has an inline "Add to contacts" button, which opens 2.8 pre-filled, and a "Dismiss" button (`x`) that hides the person for good.
- Mailing lists (`List-Id` header present) and no-reply senders are excluded.
- Privacy: the data stays on the device, and a "Suggested contacts" toggle under People settings controls the node. **Default: off** (Axentra decision, resolving the earlier open question). Neither new Outlook nor the web documents an equivalent node (the web's "Frequently contacted" view comes from a third-party KB [S9]), so users have no expectation that Axentra mines their mail for people; showing it unasked on a shared screen could expose correspondents. On first visit to People, show a one-time, dismissible tip: "Turn on Suggested contacts to see people you email who aren't saved yet." (Axentra wording).

### 2.13 Import, export, manage

Import [S6]:

1. Clicking "Import contacts" opens a dialog with a "Browse" button [S6]. The dialog title "Import contacts" is (estimate); the help names no title. Browse uses the XDG desktop portal `FileChooser` with filters "CSV (*.csv)" and "vCard (*.vcf)". Microsoft documents CSV only; vCard is an Axentra addition.
2. Outlook shows a preview of the file before importing; the help tells users to cancel and re-save as UTF-8 if the preview text looks wrong [S6]. Axentra's preview shows the first 10 rows. The column-mapping table and the "Destination folder" dropdown are Axentra additions (estimate).
3. The "Import" button starts the import [S6], which shows a progress bar. The final toast reads "Imported N contacts" plus "M skipped" when duplicates were found (Axentra wording).

Encoding: Microsoft recommends UTF-8 for new Outlook because it covers all scripts [S6][S53]. Axentra detects a UTF-8 BOM and otherwise tries UTF-8. If decoding fails it falls back to Windows-1252 and warns "Some characters may not display correctly" (estimate).

Duplicates: new Outlook hides exact duplicates automatically [S11]. Axentra skips a row when a contact already has the same normalised display name **and** the same primary email (estimate).

Export [S6]:

1. "Export contacts" opens a dialog with the dropdown "Contacts from this folder", whose options are "All contacts", "Contacts" and each folder.
2. "Export" writes `contacts.csv` (or `.vcf` if chosen) to `$XDG_DOWNLOAD_DIR`. New Outlook saves to the Downloads folder [S6].
3. A toast reads "Exported N contacts" with an "Open folder" action.

CSV header row: Outlook-compatible headers. The import/export article points to a separate Microsoft article on creating CSV files for Outlook, which offers a downloadable sample file and advises starting from a file exported by Outlook and keeping the header row intact [S6][S53]. The header names inside that sample were not readable during the fact-check, so the list below stays an estimate. Before release, export a contact set from new Outlook and from Outlook on the web and replace this list with the real header row (QA task). Import must match headers case-insensitively and accept both "E-mail" and "Email" spellings (estimate), because classic Outlook, new Outlook and Outlook.com exports are not known to share one template: `First Name, Middle Name, Last Name, Title, Suffix, Nickname, E-mail Address, E-mail 2 Address, E-mail 3 Address, Mobile Phone, Business Phone, Home Phone, Company, Department, Job Title, Office Location, Business Street, Business City, Business State, Business Postal Code, Business Country/Region, Home Street, Home City, Home State, Home Postal Code, Home Country/Region, Birthday, Web Page, Notes, Categories` (estimate). Quote fields per RFC 4180 and write a UTF-8 BOM so spreadsheet apps detect the encoding (estimate).

"Manage sources": for unified, linked contacts, this lists each source (Personal, Directory, other account) with "Unlink" and "Edit" for the sources the user may change [S11].

### 2.14 Delete and confirmations

| Target | Dialog title | Body (Axentra wording) | Buttons |
|---|---|---|---|
| 1 contact | "Delete contact?" | "{Name} will be deleted from your contacts." | "Delete" (primary), "Cancel" |
| N contacts | "Delete {N} contacts?" | "These contacts will be deleted from your contacts." | "Delete", "Cancel" |
| Contact list | "Delete contact list?" | "The people in this list won't be deleted, only the list." [S5] | "Delete", "Cancel" |
| Folder | "Delete folder?" | "{Folder} and the {N} contacts in it will be deleted." | "Delete", "Cancel" |

- Focus starts on "Cancel" (estimate). `Enter` triggers the focused button.
- On Exchange, use Graph `DELETE /me/contacts/{id}` (scope `Contacts.ReadWrite`) [S60]. Graph also has a separate `permanentDelete` action that purges the item into Recoverable Items [S33], which implies plain `DELETE` is the recoverable path; the delete page itself does not say where the item goes, so "moves to Deleted Items" is (unverified). Never call `permanentDelete` from the UI. On CardDAV it is a hard `DELETE`. The web cannot restore deleted contacts [S1], while classic Outlook can [S1]. Axentra shows an "Undo" toast for 8 s (estimate) that re-creates the item from a local snapshot (the re-created item gets a new ID; re-key local metadata).
- After a delete, selection moves to the next row, or to the previous one if the deleted row was last.

### 2.15 Data model and sync mapping

| UI concept | Exchange via Graph | CardDAV / vCard | Notes |
|---|---|---|---|
| Contact | `contact` [S33] | vCard 4.0 [S41] or 3.0 in a CardDAV address book [S42] (see below) | Graph IDs change when an item moves folder unless you send `Prefer: IdType="ImmutableId"` on **every** request, including delta and subscription requests [S33][S52]. **Always request immutable IDs.** Immutable IDs are case-sensitive, and still change if the item moves to an archive mailbox or is exported and re-imported [S52]. Migrate any stored regular IDs with `translateExchangeIds` [S52] |
| Folder | `contactFolder` | Address-book collection | Container IDs are already stable [S52] |
| Incremental sync | `GET /me/contactFolders/{id}/contacts/delta` (per folder) [S33] | `sync-collection` REPORT (RFC 6578) where offered, else ctag/etag polling (unverified for each server) | Graph contacts also support change notifications [S33] |
| Contact list | **No Graph API in v1.0 or beta** as of this check [S33][S62]. Microsoft's EWS-retirement page lists "Contact Lists" (personal contact lists in the mailbox) as a Graph parity gap with a target of Q3 2026, and EWS is being switched off from October 2026 (fully by April 2027) [S51] | vCard 4.0: `KIND:group` plus `MEMBER:mailto:` or `MEMBER:urn:uuid:`; `MEMBER` is only allowed when `KIND` is `group` [S41]. vCard 3.0 servers: Apple's `X-ADDRESSBOOKSERVER-KIND:group` / `X-ADDRESSBOOKSERVER-MEMBER` convention (unverified, not in any RFC) | See "Open questions and resolutions" |
| Favourite | Not exposed [S33][S62]. Roam Axentra's own flag in an open extension [S33] | `X-AXENTRA-FAVORITE:1` or local store | Outlook will not see Axentra favourites |
| Category | `categories` (Outlook category display names) [S33] | `CATEGORIES` [S41] | |
| Photo | `contact/photo` [S33] | `PHOTO` [S41] | |
| Recently added | `createdDateTime` [S33] | `REV`/local first-seen time | |

Graph permission scopes for this module: `Contacts.ReadWrite` (delegated, work/school and personal) [S60]. Directory (GAL) lookups need a separate directory scope, which this file does not specify (see the Mail/Directory spec).

CardDAV version: a CardDAV server **must** support vCard 3.0 and only **should** support vCard 4.0. Read the collection's `supported-address-data` property; if it is absent the server accepts only vCard 3.0, and a server may reject (415) a vCard 4.0 upload [S42]. Axentra writes vCard 4.0 only where advertised, and otherwise writes 3.0 and maps 4.0-only properties (`KIND`, `MEMBER`, `RELATED`, reduced-accuracy `BDAY`) to the 3.0 conventions above or keeps them local.

### 2.16 Classic Outlook differences (for reference)

- The command is "Create" rather than "Save". The New menu has "New contact list" and "New group" [S1].
- Contact groups are created with "New Contact Group", "Add Members" and "Save & Close" [S5].
- "Link contacts" / "Unlink" are explicit commands [S1].
- Profile tabs are "Files", "Email" and "LinkedIn" [S1].
- Deleted contacts can be restored [S1].
- `Ctrl+3` opens Contacts.

---

## 3. To Do module

### 3.1 Entry points

| Entry | Behaviour | Source |
|---|---|---|
| App rail icon "To Do" (check mark in a rounded square, estimate) | Opens the To Do module on the last list used. The help only says the To Do logo on the side margin opens the task pane where you pick a list or choose "New list" [S14]; the first-run default of "My Day" is (estimate) | [S14] |
| `Ctrl+5` | Go to To Do. Neither new Outlook nor the web documents a To Do navigation shortcut; their documented module keys stop at People (`Ctrl+4` / `Ctrl+Shift+4`) [S7]. `Ctrl+5` therefore collides with nothing in the documented table and stays an Axentra decision. Also accept `Ctrl+Shift+5` for symmetry with the web-style keys (Axentra decision). Whether new Outlook binds `Ctrl+5` to something undocumented is (unverified) | [S7] |
| "Open To Do" / "Manage all tasks" in the My Day pane | Opens the module on "My Day" | [S13] |

### 3.2 Layout

```
+-------------------+-------------------------------------------+--------------------+
| SIDEBAR           | LIST HEADER                               | DETAIL PANE        |
| [search]          | My Day                  [bulb] [Sort] [...]| (o) Task title  [*]|
| (sun) My Day    3 | Monday, October 19                        |  (o) step 1        |
| (star) Important 1| [Sorted by due date ^ x]                  |  + Add step        |
| (cal) Planned   5 | +  Add a task                             | (sun) Add to My Day|
| (person) Assigned | ----------------------------------------- | (bell) Remind me   |
| (flag) Flagged    | (o) Pay invoice                     [*]   | (cal) Add due date |
| (home) Tasks    7 |     Tasks . Due Today . 1 of 3            | (rep) Repeat       |
| ----------------- | (o) Call Ada                        [ ]   | Pick a category    |
| (list) Groceries 2|     Groceries                             | Add file           |
| > Work (group)    | v Completed 2                             | [ Add a note     ] |
| ----------------- | (x) Book flights  (struck through)        | [|]  Created today [trash]|
| + New list  [grp] |                                           |                    |
+-------------------+-------------------------------------------+--------------------+
```

| Region | Default width | Min / max | Notes |
|---|---|---|---|
| Sidebar | 280 px (estimate) | 220 / 400 px (estimate) | Collapsible to a 48 px icon rail (estimate) |
| Task list | Remaining width | 360 px minimum (estimate) | Max content width 960 px, centred (estimate) |
| Detail pane | 360 px (estimate) | 300 / 520 px (estimate) | Opens when a task is selected and closes with `Esc` [S27] |

### 3.3 Sidebar

#### 3.3.1 Smart lists (fixed order)

| # | Label | Icon (Fluent System Icons) | Contents | Count | Hideable | "Add a task" here | Source |
|---|---|---|---|---|---|---|---|
| 1 | "My Day" | `WeatherSunny` | Tasks added to My Day for today, plus incomplete tasks due today, which Outlook adds automatically (3.11) | Incomplete count | No (estimate) | Creates the task in "Tasks" and adds it to My Day | [S13][S14][S16] |
| 2 | "Important" | `Star` | Tasks with `importance = high` that are not completed | Incomplete count | Yes | Creates the task in "Tasks" with importance high | [S15][S16] |
| 3 | "Planned" | `CalendarLtr` | Incomplete tasks with a due date or a reminder | Incomplete count | Yes | Creates the task in "Tasks" due today (estimate) | [S16][S18] |
| 3a | "All" (optional; off by default, estimate) | `Infinity` (estimate) | Every incomplete task | Incomplete count | Yes | Creates the task in "Tasks" | [S15][S16] |
| 3b | "Completed" (optional; off by default, estimate) | `CheckmarkCircle` | Every completed task | none | Yes | No add bar | [S15][S16] |
| 4 | "Assigned to me" | `Person` | Planner tasks assigned to the user [S16], and tasks assigned to the user in shared To Do lists [S27] | Incomplete count | Yes (estimate; S16 does not say) | No add bar | [S16][S27] |
| 5 | "Flagged email" | `Flag` | Tasks created from flagged messages (3.12). S16 capitalises it "Flagged Email"; Graph, S22 and the To Do app use "Flagged email" [S29] | Incomplete count | Yes (Settings, "Connected apps") [S16] | No add bar (estimate) | [S16][S22][S29] |
| 6 | "Tasks" | `Home` | The default list (Graph `wellknownListName = defaultList`) | Incomplete count | No | Normal add | [S16][S29] |

Notes:

- Tasks added inside a smart list are stored in "Tasks" and linked to the smart list being viewed [S14][S15].
- A task can appear in several smart lists at once. For example, a starred task with a due date shows in both Important and Planned [S16].
- "Assigned to me" needs a Planner-capable work account. Graph's `GET /me/planner/tasks` needs `Tasks.Read` and does not support personal Microsoft accounts [S59]. Graph v1.0 `todoTask` has no assignee property [S28], so tasks assigned inside shared To Do lists [S27] can't be identified through Graph; Axentra shows Planner tasks only and hides the node for accounts without Planner (Axentra decision).
- "Flagged email" works only on Microsoft-hosted accounts in Microsoft To Do [S16][S22]. For IMAP accounts, Axentra builds the same list locally from `\Flagged` messages (Axentra decision; see 3.12).
- The support pages list the smart lists in different orders [S15][S16]. S16 lists My Day, Important, Planned, All, Completed, Assigned to me, Tasks, Flagged Email; S15 ends with Flagged Email and Due Today. The order in the table is the on-screen sidebar order Axentra ships (estimate: Flagged email above Tasks). "All" and "Completed", when enabled, go between Planned and Assigned to me, matching S16's listing order [S16].
- "All" = all incomplete tasks; "Completed" = tasks marked complete [S16]. Whether they are visible by default is undocumented; Axentra ships both hidden (estimate), with the "Completed" section collapsed by default in every list (3.7, estimate).
- Some To Do help mentions an optional "Due Today" smart list [S15]. Axentra covers it with the My Day setting instead.

#### 3.3.2 Sidebar item anatomy and states

- Height 36 px (estimate), padding 0 12 px, 20 px icon, 12 px gap.
- Label: Body1, `colorNeutralForeground1` (estimate).
- Count: right-aligned, Caption1, `colorNeutralForeground3`. Hidden when 0 (estimate). Values over 999 display "999+" (estimate).
- Icons use `colorNeutralForeground2` at rest. When selected, the icon switches to the filled variant in `colorBrandForeground1` (estimate).

| State | Background | Notes |
|---|---|---|
| Rest | transparent | |
| Hover | `colorSubtleBackgroundHover` | |
| Pressed | `colorSubtleBackgroundPressed` | |
| Selected | `colorSubtleBackgroundSelected` | Label in Body1Strong. 3 x 16 px brand pill at the left edge (estimate) |
| Focused | | 2 px `colorStrokeFocus2` inset outline |
| Drop target (task dragged over) | `colorBrandBackground2` | 1 px `colorCompoundBrandStroke` border (estimate) |

A 1 px divider in `colorNeutralStroke2`, with 8 px vertical margin, separates the smart lists from the custom lists (estimate).

#### 3.3.3 Custom lists and groups

- **List row**: if the name starts with an emoji, that emoji replaces the icon and is removed from the displayed label [S26]. Otherwise the icon is `TextBulletListLtr` (estimate). Shared lists add a trailing `People` icon [S16]. The incomplete count works as in 3.3.2.
- **Groups**: a header row with a chevron and the group name in Body1Strong (estimate). Clicking it expands or collapses the group [S45]. Member lists are indented 16 px (estimate). Groups don't show a count, and selecting a group doesn't show an aggregated list [S45].
- **Footer** (sticky, 48 px, estimate):
  - "New list" with a `Add` icon [S14][S16]. `Ctrl+L` [S21]. It adds "Untitled list" in rename mode with the text selected (estimate). `Enter` commits. `Esc` keeps the default name (estimate).
  - Beside it, an icon-only "Create group" button (`TabAdd`, estimate) [S25][S45]. It adds a group named "Untitled group" in rename mode [S25].
- Lists are dragged into or out of groups and reordered by drag [S25][S45].

#### 3.3.4 Sidebar context menus

| Target | Items, in order | Source |
|---|---|---|
| Custom list | "Rename list" (estimate), "Share list" [S16], "Move list to..." ▸ groups (estimate), "Duplicate list", "Print list", separator, "Delete list" | S16 documents right-click actions named share, duplicate, print and delete, but writes them as "Duplicate", "Print" and "Delete" in prose, so the "... list" suffixes are (estimate). Renaming is documented only through the "List options" menu, not this context menu [S16]. Deleted lists can't be recovered [S16] |
| Shared list | adds "Sharing options" [S16]. Sharing uses "Create invitation link", then "Copy link" or "Invite via email"; "Limit access to this list" stops the link working; invitees need a Microsoft account [S16]. Sharing is out of scope for v1 (Axentra decision): show "Sharing options" read-only | |
| Group | "Rename group", "Ungroup lists" [S25] | Ungrouping keeps the lists and moves them to the top level [S25] |
| Smart list (hideable) | "Hide from sidebar" (estimate) | |
| "Tasks", "Flagged email" | No rename or delete [S29] | |

Delete list dialog: title "Delete list?", body ""{name}" and all its tasks will be permanently deleted. This can't be undone." (Axentra wording), buttons "Delete list" (primary) and "Cancel".

#### 3.3.5 Settings (gear icon in the module header)

| Section | Setting label | Default | Source |
|---|---|---|---|
| General | "Confirm before deleting" | On (estimate). The label is documented; in the To Do app it is a checkbox reached from the user name or profile picture [S20] | [S20] |
| General | "Add new tasks on top" | Off (estimate) | Third-party [S46] |
| General | "Move starred tasks to top" | Off (estimate) | Third-party [S46] |
| General | "Play completion sound" | On (estimate) | Third-party [S46] |
| General | "Turn on reminder notifications" | On (estimate) | |
| Smart lists | "Important", "Planned", "All", "Completed", "Assigned to me" (one toggle each) | On, On, Off, Off, On (estimate) | [S15][S16] |
| Smart lists | "Auto-hide empty smart lists" | Off (estimate) | Third-party |
| Smart lists | "Show tasks due today in My Day" (estimate label) | On. Outlook's documented behaviour is that My Day fills with incomplete tasks due that day automatically, and S16 documents no switch for it [S16]; the toggle is an Axentra addition whose default matches Outlook | [S16] |
| Connected apps | "Flagged email" | On for Microsoft-hosted accounts (estimate default). The section name "Connected apps" is documented [S16]; a screenshot caption in S22 calls the area "Connections" (unverified which wording is current) | [S16][S22] |

### 3.4 Task list header

- Height 88 px when there's a subtitle, 64 px without (estimate). Padding 24 px horizontal (estimate).
- Title: the list name in Title3 (24/32/600) (estimate). For custom lists, clicking the title enters rename mode (estimate). Renaming is documented through the list options menu [S16].
- Subtitle, My Day only: today's date in the locale long form without the year, e.g. "Monday, October 19" (estimate format). Caption1, `colorNeutralForeground3`. It updates at local midnight.
- Right-side toolbar (subtle icon buttons, 32 x 32 px):
  - "Suggestions" (`Lightbulb`), shown only in My Day [S16].
  - "Sort" (`ArrowSort`) (estimate: a separate toolbar button is not documented; S17 reaches "Sort by" through the "..." menu at the top right of the list).
  - "List options" (`MoreHorizontal`) [S16][S17][S26].
- "Sort by" menu items, in order: "Importance", "Due date", "Added to My Day", "Alphabetically", "Creation date" [S17]. Sorting is per list [S17].

| Sort | Default direction (estimate) | Tie-break |
|---|---|---|
| Importance | Important first | Manual order |
| Due date | Earliest first; tasks without a due date last | Importance |
| Added to My Day | Tasks in My Day first, most recently added first | Manual order |
| Alphabetically | A to Z (ICU collation) | Creation date |
| Creation date | Newest first | Title |

- While a sort is active, a chip row appears under the title: "Sorted by {option lowercased}", e.g. "Sorted by due date". It contains a chevron button "Reverse sort order" and an `x` button "Remove sort order". Removing the sort returns to manual order (estimate; the docs don't describe reversing or removing [S17]).
- Sort is persisted per list.
- Manual drag-reorder is turned off while a sort is active (estimate).
- "List options" menu for a custom list: "Rename list", "Sort by" ▸, "Change theme" ▸ (colour swatches and images [S26]), "Hide completed tasks" / "Show completed tasks" [S47], "Duplicate list", "Print list", separator, "Delete list". Smart lists get only "Sort by" ▸, "Change theme" ▸ and "Hide/Show completed tasks" (estimate).
- **Theme**: a list theme tints the header title and the "Add a task" plus icon with the theme colour. Its background fills the list canvas behind the cards (estimate). Axentra ships 12 colour themes and no photo themes in v1 (Axentra decision; no Microsoft artwork). Each theme's colour must pass 4.5:1 contrast against the canvas in both light and dark.

### 3.5 "Add a task" input

```
Rest:     [+]  Add a task
Focused:  [o]  |caret                                                  
          ----------------------------------------------------------- 
          [cal] [bell] [repeat]                              [ Add ] 
```

- Sits at the top of the task list, under the header. It is 52 px tall at rest (estimate) and grows by a 40 px toolbar row when focused (estimate). Radius 4 px and `shadow2` (estimate).
- At rest: a 20 px `Add` icon and the placeholder "Add a task" [S14][S15][S20], both in `colorBrandForeground1` (estimate). Accessible name: "Add a task in {list name}" [S27].
- Focused: the plus becomes an empty 20 px completion circle (estimate). The toolbar shows three icon buttons and one text button:

| Button | Tooltip / name | Opens |
|---|---|---|
| `CalendarLtr` | "Add due date" | Same menu as the detail pane (3.10) |
| `Alert` | "Remind me" | Same menu as the detail pane |
| `ArrowRepeatAll` | "Repeat" | Same menu as the detail pane |
| Text button | "Add" | Commits [S13]. Disabled while the trimmed text is empty (estimate) |

- Once a quick value is set, its icon becomes a chip with the value, e.g. "Due Tomorrow", with an `x` to clear it (estimate).
- `Enter` commits [S14][S20] and keeps focus in the input, ready for the next task. `Esc` clears the text, then blurs on a second press (estimate).
- New tasks go to the bottom of manual order [S20], or to the top when "Add new tasks on top" is on [S46].
- Hashtags such as `#work` in a title become tag links. Clicking one runs a search for the tag [S15][S19].
- Parsing natural-language dates in the title is phase 2. Microsoft documents a version for the To Do Windows app [S18].
- Pasting multiple lines asks: "Add {N} tasks?" with buttons "Add {N} tasks" and "Add as one task" (Axentra decision).
- Title length limit: 255 characters (estimate). The counter appears at 230.

### 3.6 Task row anatomy

```
[o]  Task title text that may wrap to two lines ...                        [star]
     Tasks  .  (sun) My Day  .  1 of 3  .  (cal) Due Tue, Oct 20  .  (bell)  .  (rep)  .  (clip)  .  (note)  .  [Category]
```

| Part | Spec |
|---|---|
| Container | Card with minimum height 52 px (estimate), padding 8 px 16 px (estimate), radius 4 px, `shadow2` on `colorNeutralBackground1`, and 2 px gap between cards (estimate) |
| Completion circle | 20 px hit area 32 px (estimate). Ring 1.5 px `colorNeutralStrokeAccessible` (estimate). Accessible name "Mark as completed" [S13][S27]; the unchecked-back label "Mark as not completed" is (estimate). The Windows To Do app announces it as a "Complete task" checkbox instead [S27]. Role `checkbox` |
| Title | Body1, `colorNeutralForeground1`, up to 2 lines then ellipsis (estimate) |
| Subtitle | Caption1, `colorNeutralForeground3`, single line. Items are separated by a 2 px dot with 6 px spacing on each side (estimate). An item is shown only when it has a value |
| Star | 20 px (estimate). Tooltip "Mark as important" [S13]; S16 words the toggle "Mark task as important" / "Remove importance" [S16]. Axentra ships "Mark as important" / "Remove importance". Role `button` with `aria-pressed` |

Subtitle items, in this order (estimate order; the step counter and category labels are documented [S15][S19]):

| # | Item | When shown | Format |
|---|---|---|---|
| 1 | List name | In smart lists, or when the task belongs to a different list than the one shown | "Tasks", "Groceries" |
| 2 | My Day | Task is in My Day and the current view is not My Day | `WeatherSunny` 12 px + "My Day" |
| 3 | Steps | At least 1 step | "{done} of {total}", e.g. "1 of 3" |
| 4 | Due date | Due date set | `CalendarLtr` 12 px + label (3.10.1). Overdue is shown in `colorPaletteRedForeground1`; due today in `colorBrandForeground1` (estimate) |
| 5 | Reminder | Reminder set and in the future | `Alert` 12 px. Tooltip shows the reminder date and time |
| 6 | Repeat | Recurrence set | `ArrowRepeatAll` 12 px |
| 7 | Files | Has attachments | `Attach` 12 px |
| 8 | Note | Note body is not empty | `Note` 12 px |
| 9 | Categories | Categories set | Pill: 12 px height, the category colour as background tint, Caption1 text. At most 2 pills, then "+N" (estimate) |

States:

| State | Visual |
|---|---|
| Rest | as above |
| Hover | Background `colorNeutralBackground1Hover`. Shadow becomes `shadow4` (estimate). The circle shows a faint check: the glyph in `colorBrandForeground1` at 60% opacity, ring in `colorBrandForeground1` (estimate) |
| Pressed | Background `colorNeutralBackground1Pressed` |
| Selected (detail pane open) | Background `colorBrandBackground2` (estimate) |
| Focused | 2 px `colorStrokeFocus2` outline, radius 4 px. The circle and star are separate tab stops, and arrow keys move between rows |
| Completed | Circle filled with `colorBrandBackground`, check in `colorNeutralForegroundOnBrand`. Title struck through in `colorNeutralForeground3`. Subtitle unchanged (estimate) |
| Dragging | Lifted ghost with `shadow16` at 95% opacity, and the source slot at 40% opacity (estimate) |
| Read-only (Assigned to me, shared without edit) | Star and circle still work. Edits to fields that aren't allowed are disabled in the detail pane |

Interactions:

- Clicking the row body selects the task and opens the detail pane.
- Clicking the circle toggles completion (3.8). Clicking the star toggles importance. Neither opens the detail pane.
- Double-clicking the title opens the detail pane with the title in edit mode (estimate).
- Right-click menu, in order: "Add to My Day" / "Remove from My Day" [S16][S27], "Mark as important" / "Remove importance", "Mark as completed" / "Mark as not completed", separator, "Due today", "Due tomorrow", "Pick a date", "Remove due date", separator, "Move task to..." ▸ lists (S16 words it "Move tasks to ..." [S16]), separator, "Delete task". The Windows To Do app labels the single-task delete item "Delete selected task" [S20]; Axentra uses "Delete task", matching the web trash button name [S27]. Item order and the other labels are estimates.
- `Shift+click` selects a range [S20]. `Ctrl+click` toggling one task is (estimate); the help mentions only `Shift+click`. With several selected, the context menu acts on all of them, and its delete item reads "Delete tasks" [S20].
- Reordering: drag by the row, or by the 6-dot handle that appears on hover at the left edge (estimate). The My Day pane uses explicit drag dots [S13].

### 3.7 Completed section

- Appears below the incomplete tasks whenever the list has at least 1 completed task.
- Header row, 40 px (estimate): a 12 px chevron, then the label "Completed" and the count in Body1Strong, e.g. "Completed 3". Clicking it toggles collapsed or expanded. The chevron rotates over 150 ms.
- Expanded or collapsed is persisted per list. The default is collapsed (estimate).
- Completed tasks are ordered by `completedDateTime`, newest first (estimate).
- "Hide completed tasks" in List options removes the whole section [S47].
- In "My Day", completed tasks stay in the section until the midnight reset. In "Important" and "Planned", completed tasks are not shown unless "Show completed tasks" is on (estimate).
- Clearing a completed task's circle moves it back to the incomplete list at its previous position, or at the end if that position is unknown (estimate).

### 3.8 Completion animation and sound

Sequence for marking a task complete (all timings are estimates, built from Fluent motion tokens):

| t (ms) | Event |
|---|---|
| 0 | The circle fills with `colorBrandBackground`: scale 0.6 to 1.0 over 100 ms on `curveDecelerateMid`. The completion sound starts |
| 0-200 | The check stroke draws in (SVG `stroke-dashoffset`) over 200 ms on `curveDecelerateMid`. The title strikethrough draws left to right over 200 ms |
| 350 | The row collapses: height to 0 and opacity to 0 over 250 ms on `curveAccelerateMid` |
| 600 | The row is inserted at the top of "Completed". The "Completed" count increments with a 150 ms fade |

- If the user unchecks the task within the first 600 ms, cancel the sequence and restore the row in place.
- `prefers-reduced-motion` skips the animation; the state changes at once and the sound still plays.
- On the web, the To Do app plays a sound cue and moves keyboard focus to the next task's checkbox [S27]. The Windows app also plays a sound cue; its focus behaviour is not documented [S27]. Axentra follows the web.
- Sound: a short, original Axentra chime of about 250 ms, peaking at -12 dBFS (estimate). **Do not ship Microsoft's sound asset.** Play it through the webview `<audio>` element, preloaded. It is controlled by "Play completion sound" [S46]. Mute it when the system is in Do Not Disturb, if the portal exposes that state (estimate).
- Completing a repeating task creates the next instance (3.10.3).
- Completing a flagged-email task updates the message flag (3.12).

### 3.9 Task detail pane

The pane has `colorNeutralBackground3` behind stacked cards of `colorNeutralBackground1`. Each card has radius 4 px and `shadow2`. Cards are 8 px apart and the pane has 16 px padding (estimate). Task titles and notes save automatically [S27]. In the Windows app, a custom reminder or due date picked from the calendar needs an explicit "Save" in the picker [S27], which matches the "Save" / "Cancel" buttons in 3.10. Everything else saves at once with no Save button (estimate).

| # | Card | Contents and behaviour | Labels / sources |
|---|---|---|---|
| 1 | Title and steps | Row: 20 px circle, an editable title in Subtitle1 (20/28/600) that wraps freely, and a star. Below: the steps list. Each step row is 40 px with a 16 px circle, text (struck through when done) and a hover menu `...` with "Mark as completed", "Promote to task" and "Delete step" (estimate labels). Then the "Add step" input with a `Add` icon. After the first step exists, its placeholder becomes "Next step" (estimate). `Enter` saves a step and keeps focus there for the next one [S15][S19]. Steps reorder by drag (estimate) | "+ Add step" [S15][S19]. Accessible name of the title: "Rename task" (web) [S27] |
| 2 | My Day | Single row: `WeatherSunny` + "Add to My Day". Once set, it reads "Added to My Day" (estimate) in `colorBrandForeground1` with an `x` labelled "Remove from My Day" [S27]. The Windows app words these "Add task to My Day" / "Remove task from My Day" [S27] | [S16][S27] |
| 3 | Dates | Three rows, separated by a 1 px `colorNeutralStroke3` hairline: "Remind me" (`Alert`), "Add due date" (`CalendarLtr`), "Repeat" (`ArrowRepeatAll`). Each row opens a menu (3.10). Once set, the row shows the value in `colorBrandForeground1` (overdue due dates in red) and an `x` to remove it. The `x` labels are "Remove reminder", "Remove due date" and "Remove recurrence" (web) [S27]; the Windows app uses "Never repeat" for the last [S27] | [S15][S18][S27] |
| 4 | Category | "Pick a category" (`Tag`) opens a multi-select menu of the account's Outlook categories, each with a colour swatch. Chosen categories show as pills with `x` [S15][S19]. New categories are created under Settings > Accounts > Categories [S15], so the menu ends with a link "Manage categories" (estimate) | [S15][S19] |
| 5 | Files | "Add file" (`Attach`) opens the portal file chooser, one file at a time, with a 25 MB per-task limit [S15][S27]. File rows show the type icon, name, size and a `x` labelled "Delete file" [S27]. Over the limit, the error reads "Files must be 25 MB or smaller" (estimate wording). Sync: files under 3 MB use one POST; larger files need an upload session [S32] | [S15][S27][S32] |
| 6 | Linked email (flagged-email tasks only) | Sender avatar (24 px), sender name, subject and a 3-line body preview. The link is "Open email" (Axentra wording; the documented label is "Open in Outlook" [S22]). It opens the message in Mail | [S22][S31] |
| 7 | Note | Multiline rich text with placeholder "Add a note" [S15][S19]. (Screen readers hear "Write a note" on the web and "Add note" in the Windows app [S27].) Supports bold, italic, underline and numbered lists [S19]. Shortcuts `Ctrl+B`, `Ctrl+I`, `Ctrl+U`, which Microsoft documents for To Do on the web only [S19]. Saves on blur and after 1 s idle (estimate). Shows "Updated {relative time}" under the note in Caption1 (estimate). Graph: an update accepts the body only as HTML [S61] | [S15][S19][S27] |
| 8 | Footer (sticky, 48 px) | Left: icon button "Dismiss detail view" (`PanelRightContract`, estimate icon), the same as `Esc`. The name comes from the Windows app's screen-reader output [S27]; the web name is not documented. Centre: Caption1 `colorNeutralForeground3` text "Created today", "Created yesterday" or "Created on {weekday, Month day}". Completed tasks show "Completed on {...}" instead (estimate format). Right: trash icon button "Delete task" in the bottom corner [S14][S20][S27] | |

Delete task:

- With "Confirm before deleting" on, show a dialog with title "Delete task?" and body ""{title}" will be permanently deleted." (Axentra wording). Buttons "Delete" (primary) and "Cancel". The trash button is named "Delete task" and the confirming button "Delete" [S27].
- With confirmation off, delete at once and show an "Undo" toast for 8 s (Axentra decision).
- Exchange keeps deleted tasks in Deleted Items, and classic Outlook can restore them [S20]. Axentra's Undo re-creates the task from a local snapshot.

For tasks from Planner ("Assigned to me"), the pane adds an "Assigned to" row showing the assignee, and editing is limited to what the source allows (estimate). In shared To Do lists the documented controls are "Assign to" and "Remove assignment", and a task can only be assigned to someone who has joined the list [S27]; Axentra does not offer assignment in v1 because Graph exposes no assignee on `todoTask` [S28].

### 3.10 Date, reminder and repeat rules

#### 3.10.1 Due date

Menu items [S18]: "Today", "Tomorrow", "Next week", a separator, "Pick a date". Each item shows its resolved weekday on the right in Caption1, e.g. "Tue" (estimate).

- "Next week" means the next first day of the week for the locale, e.g. Monday (estimate).
- "Pick a date" opens a month calendar popover with `shadow16`, the week starting on the locale's first day, and "Save" / "Cancel" buttons (estimate).
- Store the due date as a date with no time. With Graph, send `dueDateTime` as local midnight plus a `timeZone`. Graph's `timeZone` field accepts Windows time zone names (for example "Pacific Standard Time") and only a fixed list of about 100 IANA names; create and update calls may not accept every zone [S57]. Map the system IANA zone to its Windows name with the CLDR `windowsZones` table and send that. Graph returns `dueDateTime` normalised to UTC (for example 04:00 UTC for a New York midnight) [S61]; convert back to the user's zone before taking the date part, or send `Prefer: outlook.timezone="<Windows name>"` on reads (unverified for To Do endpoints).

Label rules, used in the row and the pane (estimate):

- Due today: "Today" in the pane, "Due Today" in the row.
- Due tomorrow: "Tomorrow" / "Due Tomorrow".
- Due yesterday: "Yesterday" / "Due Yesterday", shown in red.
- Earlier this year: "Due Mon, Oct 12" in red.
- Other dates this year: "Due Tue, Oct 20".
- A different year: "Due Tue, Oct 20, 2027".

#### 3.10.2 Reminder

Menu items [S18]: "Later today", "Tomorrow", "Next week", a separator, "Pick a date & time".

- "Later today" means now plus 3 hours, rounded up to the hour. It is hidden after 21:00 (estimate).
- "Tomorrow" means 09:00 tomorrow (estimate). "Next week" means 09:00 on the locale's first weekday next week (estimate).
- "Pick a date & time" shows the calendar plus a time combobox with 30-minute steps and free typing (estimate).

Once set, the pane shows "Remind me at {time}" on line 1 and "Today", "Tomorrow" or "{Weekday, Mon d}" on line 2 (estimate).

The reminder fires a desktop notification through `org.freedesktop.Notifications`. The notification is titled with the task title, body "Reminder", and has the actions "Complete" and "Snooze 10 minutes" (estimate). Reminders must fire even when the To Do module isn't open: schedule them in the core service, not in the UI.

#### 3.10.3 Repeat

Menu items [S18]: "Daily", "Weekdays", "Weekly", "Monthly", "Yearly", a separator, "Custom".

"Custom" opens a dialog (estimate). The first row is "Repeat every", a number (1-999) and a unit ("days", "weeks", "months" or "years"). For weeks, a row of 7 toggle chips follows, ordered from the locale's first day. The buttons are "Save" and "Cancel".

- Setting a repeat on a task with no due date also sets the due date to today (estimate).
- When a repeating task is completed, the completed instance stays in "Completed" and a new incomplete copy is created with the next due date. The copy keeps steps (unchecked), note, categories, importance and My Day membership only if its next due date is today (estimate).
- The row's repeat icon tooltip spells out the rule, e.g. "Weekly on Mon, Wed" (estimate).

### 3.11 My Day and Planned behaviour

My Day:

- My Day is cleared at local midnight every night. The tasks stay in their own lists and are not deleted [S13].
- Implement the reset as a timer for the next local midnight. Re-arm it after suspend/resume, time-zone changes and DST changes. If a launch finds the stored date is older than today, run the reset at once.
- When "Show tasks due today in My Day" is on, incomplete tasks due today appear in My Day automatically [S16]. They are marked so that "Remove from My Day" hides them for the day (estimate).
- The **Suggestions** flyout (lightbulb) is a right-side panel, 320 px (estimate). Its heading is "Suggestions" and it has a "Close" button [S16]. Sections, in order (estimate headings):
  - "Overdue"
  - "Due today"
  - "Upcoming" (due in the next 7 days)
  - "Earlier" (tasks that were in My Day on a past day and aren't completed)
  - "Recently added" (created in the last 2 days)

  These match the documented kinds of suggestion [S16]. Each row is a compact task row with a "+" button labelled "Add to My Day" [S16]. Once added, the row disappears from Suggestions.

Planned:

- Grouped, collapsible sections with counts: "Earlier" (overdue), "Today", "Tomorrow" [S18], then "This week", "Later" and "No due date (reminder only)" (estimate for the last three).
- Within a section, tasks are ordered by due date, then by reminder time.

### 3.12 Flagged email

Documented behaviour [S16][S22]:

- Flagging a message creates a task in "Flagged email" whose title is the message subject. The flag shortcut is `Insert`; pressing it on a flagged message marks the flag complete [S7].
- Supported accounts: Outlook.com, Hotmail.com, Live.com and custom domains hosted by Microsoft; To Do must be signed in with the same account as the mailbox [S22].
- On first enable, the list is backfilled with the 100 most recently flagged messages from the last 30 days.
- Only the primary mailbox syncs. Shared mailboxes and shared folders don't. Flagged calendar items are excluded.
- The task can be renamed, given a due date and a reminder, added to My Day and marked important.
- The task detail shows a preview of the message with a link to open it.
- Categories already on the flagged message sync to the task [S15][S19]. This applies to mail flagged on or after 27 April 2020; older flagged mail must be unflagged and reflagged [S19].
- Microsoft does **not** document what happens to the message when the task is completed, or to the task when the message is unflagged [S22]. Everything in the table below is Axentra behaviour.
- Built-in list, `wellknownListName = flaggedEmails`, which can't be renamed or deleted [S29].

Axentra rules (estimate unless cited):

| Event | Effect |
|---|---|
| Task marked completed | Message flag set to `complete` (Graph `followupFlag.flagStatus`, values `notFlagged`, `flagged`, `complete` [S48]; IMAP: see below). Community reports say Microsoft To Do completes the flag [S49] |
| Message unflagged (`notFlagged`) | The task is removed from "Flagged email" and from Axentra's local views |
| Message flag marked complete in Mail | The task is marked completed |
| Message deleted | The task stays. Its linked-email card shows "This email is no longer available" |
| Task due date changed | Message `followupFlag.dueDateTime` updated. Graph returns `400 Bad Request` unless `startDateTime` is sent with it [S48] |

Row addition: in smart lists, the subtitle's list item reads "Flagged email". The row also shows a 12 px `Mail` icon before the title (estimate).

**IMAP accounts** (Axentra decision; Microsoft's version needs Microsoft-hosted accounts [S16][S22]):

- Scope: build the list from `\Flagged` messages in **all subscribed mail folders except Trash, Junk, Drafts and Sent** (identified by RFC 6154 special-use attributes where the server offers them, unverified per server), not INBOX only. Users flag mail after filing it, and Microsoft's help limits its own version only by mailbox (primary mailbox yes, shared mailboxes and shared folders no), not by folder [S22]; whether it really includes every folder is (unverified). A per-folder opt-out lives in account settings. Keep the cost down with CONDSTORE/QRESYNC (RFC 7162) where offered, so only changed flags are fetched (unverified per server).
- Completion: IMAP has only the `\Flagged` system flag ("flagged for urgent/special attention") and no completed state [S55]. Keep `\Flagged` and add the keyword `$Completed` so other clients still see the message as flagged. `$Completed` is **not** in the IANA IMAP/JMAP keyword registry [S54] (registered keywords include `$Important`, `$Junk`, `$Forwarded`, `$followed` and Apple's `$MailFlagBit0`-`2` colour bits, none meaning "done"), so treat it as an Axentra private keyword and document it. Only write it when the folder's `PERMANENTFLAGS` includes `\*` (client may create keywords) [S55]; otherwise keep completion in the local store and leave the message flag untouched.
- Unflagging in another client (`\Flagged` removed) removes the task, as in the table above. Removing `$Completed` reopens the task.

### 3.13 Drag and drop

| Drag source | Drop target | Result | Source |
|---|---|---|---|
| Mail message(s) in the message list | My Day pane, **To Do** tab | Creates one task per message: title = subject, a linked resource pointing at the message, in "Tasks" and added to My Day (estimate for the stored fields) | [S23] |
| Mail message(s) | The To Do app-rail icon | Creates tasks in "Tasks" and shows the toast "Task created" with "Open". This mirrors classic Outlook's drag-to-Tasks (Axentra decision) | |
| Task in the My Day **To Do** tab | Calendar grid (day, week or work week) | Creates an event at the drop slot: title = task title, 30 min (estimate), body = the task note. The task is not changed (estimate) | [S24][S13] |
| Task row | Another task position in the same list | Reorder (manual sort only) | [S13] |
| Task row | Custom list or "Tasks" in the sidebar | Move the task to that list. This re-keys the ID (see 3.15) | [S16] (move), drag estimate |
| Task row | "My Day" in the sidebar | Add to My Day | [S16] |
| Task row | "Important" in the sidebar | Mark important (estimate) | |
| Task row | "Planned" in the sidebar | Set due today (estimate) | |
| Files from the file manager (`text/uri-list`) | Task detail pane | Attach the files, enforcing the 25 MB limit | [S15], drag estimate |

Drag visuals:

- The ghost is a compact card with the title, with a count badge when dragging several items (Caption1Strong on `colorBrandBackground`).
- A valid drop target shows `colorBrandBackground2` and a 1 px `colorCompoundBrandStroke` outline. An invalid target shows the `not-allowed` cursor.
- Reordering shows a 2 px brand insertion line.
- `Esc` cancels the drag.

All drag and drop runs on HTML5 drag events inside the Dioxus webview. Mail-to-task drags carry a custom MIME type `application/x-axentra-message-ids` (JSON array).

### 3.14 Keyboard

From the To Do help [S21]. That article covers only the Windows 10 To Do app; Microsoft documents no shortcuts for To Do inside new Outlook or on the web [S7]. Inside the module Axentra applies these bindings and overrides the global ones while the module has focus.

| Shortcut | Action |
|---|---|
| `Ctrl+N` | Focus "Add a task" (new task) [S21] |
| `Ctrl+L` | New list [S21] |
| `Ctrl+T` | Add the selected task(s) to My Day [S21]. Removing them when all are already in it is (estimate) |
| `Ctrl+D` | Complete the selected task(s) [S21] |
| `Ctrl+F` | Search tasks [S21]. Search covers tasks, steps, tags and notes [S17] |
| `Ctrl+E` | Also search, for parity with Outlook's global search key (Axentra decision) |
| `Ctrl+R` | Sync now [S21] |
| `Ctrl+V` | Paste (listed by the To Do app [S21]); multi-line paste follows 3.5 |
| `Enter` / `Space` on the circle | Toggle complete. Focus then moves to the next task [S27] |
| `Esc` | Close the detail pane, cancel rename, or cancel a drag [S27] |
| `Up` / `Down` | Move between tasks (estimate) |
| `Ctrl+Up` / `Ctrl+Down` | Move the focused task up or down in manual order (estimate) |
| `Delete` | Delete the selected task(s), with confirmation if that setting is on (estimate) |
| `F2` | Rename the focused task or list (estimate) |
| `Shift+click`, `Ctrl+click` | Multi-select [S20] |
| `Ctrl+1` (To Do app) | Not mapped: in Axentra it means "Go to Mail" [S7]. The To Do app uses it for the minified sidebar [S21] |

Screen-reader names come from Microsoft's web To Do labels [S27]: "Add a task", "Mark as completed", "Add to My Day" / "Remove from My Day", "Remind me" / "Remove reminder", "Add due date" / "Remove due date", "Repeat" / "Remove recurrence", "Add file" / "Delete file", "Delete task", "Rename task". "Mark as important" is the My Day pane tooltip [S13], not an S27 label. After a task is added, announce "Task added" through an `aria-live="polite"` region; Narrator and NVDA announce this in the Windows app [S27].

### 3.15 Data model and sync mapping

| UI concept | Graph (`/me/todo`) | CalDAV VTODO (RFC 5545 [S43], RFC 4791 [S44]) | Notes |
|---|---|---|---|
| List | `todoTaskList`: `displayName`, `isOwner`, `isShared`, `wellknownListName` (`none`, `defaultList`, `flaggedEmails`, `unknownFutureValue`) [S29] | Calendar collection whose `supported-calendar-component-set` includes `VTODO` | Treat `unknownFutureValue` and any unrecognised value as a read-only built-in list. Groups of lists don't exist in Graph [S29]; store them locally and mirror them in a list-level open extension (see below) |
| Title | `title` [S28] | `SUMMARY` | |
| Note | `body` (itemBody) [S28]; updates accept HTML only [S61]. `bodyLastModifiedDateTime` tells you when the note changed [S28] | `DESCRIPTION` (plain text). For rich text, the standard property is `STYLED-DESCRIPTION` (RFC 9073), with `DESCRIPTION` either absent or a plain-text rendering marked `DERIVED=TRUE` [S56]. `X-ALT-DESC;FMTTYPE=text/html` is a Microsoft extension, not a standard; read it, but write `STYLED-DESCRIPTION` | |
| Importance | `importance`: `low`, `normal`, `high` [S28] | `PRIORITY`: 0 = undefined, 1-4 = high, 5 = medium, 6-9 = low [S43]. Write 1 for high, 0 (or omit) for normal; read any 1-4 as high | The UI exposes only normal and high. Graph `low` and CalDAV 6-9 display as normal but are preserved on write-back |
| Completed | `status = completed`, `completedDateTime` [S28] | `STATUS:COMPLETED`, `COMPLETED` (must be a UTC date-time [S43]), `PERCENT-COMPLETE:100` | Other Graph statuses (`notStarted`, `inProgress`, `waitingOnOthers`, `deferred`) display as not completed and are preserved |
| Start date | `startDateTime` [S28] | `DTSTART` | Not shown in the UI; preserve on write-back |
| Due date | `dueDateTime` [S28] (see 3.10.1 for time zones) | `DUE;VALUE=DATE` | If a `DTSTART` is present it must use the same value type as `DUE` (RFC 5545, unverified section reference) |
| Reminder | `isReminderOn`, `reminderDateTime` [S28] | `VALARM` with `ACTION:DISPLAY`, a `DESCRIPTION` (required for DISPLAY alarms) and `TRIGGER;VALUE=DATE-TIME` written in UTC (required for absolute triggers) [S43] | |
| Repeat | `recurrence` (patternedRecurrence) [S28] | `RRULE` | |
| Categories | `categories` (Outlook category display names) [S28] | `CATEGORIES` | |
| Steps | `checklistItems`: `displayName`, `isChecked`, `checkedDateTime`, `createdDateTime` [S30] | **Child VTODOs**, each with `RELATED-TO:<parent UID>` (RELTYPE defaults to `PARENT` [S43]) | Resolves the earlier open question. Child VTODOs are the only standards-based option and other CalDAV task apps can at least see them as tasks; `X-` lines would be invisible to them. Other apps showing them as nested subtasks rather than as top-level tasks is (unverified) per app. Map step order to `X-AXENTRA-ORDER` on each child |
| Files | `attachments` (`taskFileAttachment`: name, contentType, size, contentBytes) [S32]; `hasAttachments` flag on the task [S28] | `ATTACH` (binary or URI) | Graph To Do APIs are not available in the 21Vianet (China) cloud [S58] |
| Linked email | `linkedResources`: `applicationName`, `displayName`, `externalId`, `webUrl` [S31] | `X-AXENTRA-LINKED-MESSAGE` (Message-ID) | |
| Created / modified | `createdDateTime`, `lastModifiedDateTime` [S28] | `CREATED`, `LAST-MODIFIED`, `DTSTAMP` | |
| My Day membership | **Not in the documented todoTask properties** (v1.0) [S28] | `X-AXENTRA-MYDAY;VALUE=DATE` | Local store, plus a Graph **open extension** on the task so it roams between Axentra installs [S28]. Outlook won't see it, and Outlook's own My Day won't appear in Axentra |
| Manual order | Not exposed [S28] | `X-APPLE-SORT-ORDER`-style integer (Axentra uses `X-AXENTRA-ORDER`) | Local store plus open extension, as for My Day |

Graph open extensions: `todoTask` and `todoTaskList` both support open extensions [S28][S29]. Axentra writes one extension per object named `com.axentra.todo` holding `{ "myDay": "YYYY-MM-DD" | null, "order": n }` on tasks and `{ "group": "<group id>", "groupName": "...", "order": n }` on lists. This is Axentra's own sync channel for My Day, manual order and list groups (resolving the open question): no extra server, roams across Axentra installs, ignored by Microsoft clients. Local SQLite stays the source of truth while offline.

Critical: a Graph task `id` changes when the task moves to another list [S28], and immutable IDs do **not** cover To Do tasks (they cover messages, events, contacts and the legacy `outlookTask`) [S52]. Graph v1.0 also has **no move operation** for tasks: the methods are list, create, get, update and delete [S28], and update takes no parent-list field [S61]. Implement "Move task to..." as: create the task in the target list, re-create its checklist items, linked resources, attachments and open extension, then delete the original. Re-key all local metadata (My Day, manual order, expanded state) in the same local transaction, and roll back if any step fails. Flagged-email tasks must not be moved (estimate: Outlook does not document moving them).

Sync: use `GET /me/todo/lists/delta` for lists and `GET /me/todo/lists/{id}/tasks/delta` per list for tasks [S58]. Both need delegated `Tasks.ReadWrite` (the task delta also allows `Tasks.Read`) for work/school and personal accounts [S58]. Attachments use `POST .../tasks/{id}/attachments` under 3 MB and an upload session above that [S32].

Graph permission scopes for this module: `Tasks.ReadWrite` (To Do), `Tasks.Read` for Planner "Assigned to me" (work/school only) [S59], and a mail scope for writing message flags in 3.12 (`Mail.ReadWrite`, unverified here; see the Mail spec).

### 3.16 Empty states (Axentra wording; no Microsoft artwork)

| List | Title | Body |
|---|---|---|
| My Day | "Nothing planned for today" | "Add tasks here or open Suggestions. My Day starts fresh every night." |
| Important | "No important tasks" | "Star a task to see it here." |
| Planned | "Nothing planned" | "Tasks with a due date or reminder show up here." |
| Assigned to me | "Nothing assigned to you" | "Tasks assigned to you show up here." |
| Flagged email | "No flagged email" | "Flag a message in Mail and it shows up here as a task." |
| Flagged email (turned off) | "Flagged email is turned off" | Button: "Turn on" (opens Settings > Connected apps) |
| Tasks / custom list | "No tasks yet" | "Use Add a task above to get started." |
| Search, no hits | "No results for "{query}"" | none |

### 3.17 Classic Outlook differences

- Classic has a separate Tasks module (`Ctrl+4`) and the To-Do Bar. My Day in new Outlook is the counterpart of the To-Do Bar [S13].
- Classic tasks don't support My Day or steps. Recovering deleted To Do tasks is done in classic Outlook through Deleted Items [S20].

---

## 4. My Day side pane (new Outlook header)

### 4.1 Header button

- A toggle button in the app header (title-bar row), on the right, next to Notifications [S13]. Icon `CalendarCheckmark` (estimate), 20 px in a 32 x 32 px subtle button. Tooltip and accessible name "My Day". `aria-pressed` reflects the open state.
- Available in Mail, Calendar, People and Groups [S13][S15]. In the To Do module it is still visible but disabled with the tooltip "My Day is already open in To Do" (estimate).
- States: rest is transparent; hover `colorSubtleBackgroundHover`; pressed (open) `colorSubtleBackgroundSelected` with the icon in `colorBrandForeground1` (estimate).

### 4.2 Pane anatomy

```
+--------------------------------------+
| My Day                           [x] |
| [ Calendar ] [ To Do ]               |   <- tab list, 3 px brand underline on the active tab
|--------------------------------------|
| (tab content, scrolls)               |
|--------------------------------------|
| footer link (per tab)                |
+--------------------------------------+
```

- Docked at the right edge of the module content. It pushes the content rather than overlaying it (estimate). Width 320 px (estimate), resizable from 280 to 480 px (estimate). Background `colorNeutralBackground1` with a 1 px `colorNeutralStroke2` left border.
- Header: "My Day" in Subtitle2 and a close button "Close My Day" (estimate label).
- Tabs: "Calendar" and "To Do" [S13]. Fluent TabList style: 44 px tall (estimate), Body1 labels, and a 3 px `colorCompoundBrandStroke` underline under the selected tab that slides over 200 ms on `curveDecelerateMid` (estimate). The pane remembers the last tab.
- Open state and width persist per module (estimate).
- Opening the pane with `F6` cycling puts focus on the selected tab. `Esc` with focus inside the pane closes it (estimate).

### 4.3 Calendar tab

| Element | Behaviour | Source |
|---|---|---|
| Date header | Shows the selected date, e.g. "Today" or "Tue, Oct 20" (estimate format). Clicking it opens a mini calendar to choose a date. Prev/next chevrons on each side step one day (estimate) | [S13] |
| "New event" | Opens a quick-create event form for the selected date. It defaults to the next half hour, 30 min (estimate) | [S13] |
| "Switch views" (`...`) | Menu: radio items "Agenda" and "Day", then a "Calendars" section with a checkbox per calendar to choose which show in the pane | [S13] |
| Agenda view | Events grouped by day ("Today", "Tomorrow", "{Weekday, Mon d}"). Each row: 4 px calendar-colour bar, time range in Caption1, title in Body1, location in Caption1 (estimate). All-day events come first | [S13] |
| Day view | A vertical timeline of the selected day with 48 px per hour (estimate), a red now-line, and the current time scrolled into view (estimate) | [S13] |
| Event details | Clicking an event shows its details inside the pane, with "Back" and "View full event in new window" | [S13] |
| Online meetings | Shows "Join Online" on the row while the meeting is joinable, from 10 min before the start until the end (estimate window). Clicking opens the meeting URL. Microsoft documents this button for new Outlook only (Teams meetings) [S13] | [S13] |
| Drop target | Dragging a mail message onto this tab switches to the To Do tab after hovering 600 ms (estimate). The documented flow drops mail on the To Do tab [S23] | |

### 4.4 To Do tab

| Element | Behaviour | Source |
|---|---|---|
| "Add a task" | The same input as 3.5 in compact form: no quick-date toolbar, and an "Add" button. `Enter` adds the task to "Tasks" and to My Day | [S13] |
| Task rows | Compact rows, 44 px (estimate): circle "Mark as completed", title, star "Mark as important". Subtitle shows only the list name and the due date (estimate) | [S13] |
| Reorder | A 6-dot drag handle beside the circle reorders tasks | [S13] |
| Task click | Opens the task in the To Do module's detail pane (navigates to To Do > My Day) | [S13] |
| Completed | Completed tasks collapse into "Completed N" at the bottom, as in 3.7 (estimate) | |
| Footer | "Open To Do" link (`Open` icon). Its tooltip is "Manage all tasks". S13 gives the two names as alternatives ("Open to do" or "Manage all tasks"), so which one is the visible label and which the tooltip is (estimate) | [S13] |
| Drop target | Accepts mail messages (creates tasks) | [S23] |
| Drag source | Tasks can be dragged onto the Calendar grid to create events | [S24] |

Empty state: "No tasks for today", then "Add a task or drag an email here." (Axentra wording).

### 4.5 Synchronisation between the pane and the module

The pane and the To Do module share one state store. Completing a task in one updates the other without a refresh. The midnight reset (3.11) applies to both.

---

## 5. Implementation notes (must not miss)

1. **Avatar colour hashing iterates UTF-16 code units** (`encode_utf16`), with the index of the UTF-16 unit used in `i % 8`. Iterating Rust `char`s gives different colours for every non-BMP name. Keep the palette order exactly as listed (30 entries), or every avatar changes colour.
2. **Dark-mode avatar colours are not a CSS filter.** They are specific palette shades, including the dark-red and plum exceptions (shade20). Ship both tables as CSS custom properties `--avatar-{n}-bg` and `--avatar-{n}-fg` under `[data-theme]`.
3. **Initials follow Fluent `getInitials`**: 3 words take initials from the 1st and 3rd, and 4 or more words give a single initial. CJK, Arabic and Hangul leading characters, and phone-number names, fall back to the person icon. Write unit tests with the examples in 2.6.2.
4. **Request immutable IDs from Graph for contacts** (`Prefer: IdType="ImmutableId"` on every request, including delta and subscriptions) [S33][S52]. Immutable IDs don't cover To Do tasks [S52]. **Re-key To Do metadata when a task moves between lists**, because the Graph task ID changes [S28], and implement the move as create, copy children, delete, since Graph has no move call [S28][S61].
5. **My Day membership, manual task order, list groups and contact favourites have no documented Graph property.** Keep them in Axentra's local SQLite store keyed by stable IDs, and roam them between Axentra installs through Graph open extensions on the task, list and contact [S28][S29][S33]. Never silently drop them on resync. In the UI, say they sync between Axentra devices but not to Outlook.
6. **The My Day midnight reset** must survive suspend, time-zone change and DST. Run the check on resume and on startup, not only from a timer.
7. **Reminders run in the core service**, not in the Dioxus UI, so they fire with the To Do view closed. Use the freedesktop notification spec with action buttons.
8. **Smart lists are views, not storage.** A task added in My Day, Important or Planned is stored in "Tasks" with the matching attribute set [S14][S15]. Never create a backing list named after a smart list.
9. **"Tasks" and "Flagged email" can't be renamed or deleted** [S29]. Hide those menu items rather than disabling them.
10. **Flagged email is two-way.** Flag, unflag, complete and due-date changes must loop between Mail and To Do without echo storms. Debounce, and compare against the last known server state.
11. **Context-scoped shortcuts:** `Ctrl+N`, `Ctrl+L` and `Ctrl+D` mean different things in Mail, People and To Do. Route shortcuts through a single keymap with module scopes, and show the active scope in the `Shift+?` overlay.
12. **Virtualise every list** (contacts, tasks, Suggestions). Sticky alphabetical headings must keep working with virtualisation; render headings as virtual rows.
13. **ICU collation and grapheme segmentation** for sorting, index headings and type-ahead. Never use byte order or `to_uppercase` on the first byte.
14. **Every animation honours `prefers-reduced-motion`**, including the chevron rotation, the completion sequence and tab underline slides. The completion sound still plays and is governed only by its setting.
15. **No Microsoft assets.** No completion sound, illustrations, list theme photos or icons taken from Outlook. Use icons from the MIT-licensed Fluent UI System Icons set by name only where licensing allows, or Axentra's own glyphs.
16. **No LinkedIn and no Files tab** in the contact details pane. Don't leave empty placeholders.
17. **Read-only contacts** (directory, linked sources) hide Edit and Delete; they don't disable them [S1]. Organisation-enriched edits are private overlays [S2].
18. **Contact list deletion never deletes members** [S5]. Write a test that asserts it.
19. **CSV import:** prefer UTF-8 and detect the BOM [S6]. Show a preview and the column mapping before any write. Make the import atomic per batch of 100, so that a failure doesn't leave half-imported duplicates.
20. **Attachment upload:** use direct POST under 3 MB and an upload session from 3 MB to 25 MB [S15][S32]. Reject larger files before reading them fully into memory.
21. **Focus management:** after completing a task, focus moves to the next task's circle [S27]. After deleting a contact, selection moves to the next row. Dialogs trap focus and return it to the invoking control.
22. **Accessible names** must match the labels in 3.14 and 2.4 exactly so that screen-reader users' muscle memory carries over. Announce "Task added" and "Task completed" through a polite live region.
23. **Estimates are tunable.** Every value tagged `(estimate)` must be a named design token or constant in `axentra-ui/tokens.rs`, never a magic number in a component, so design QA can adjust it in one place.
24. **Drag and drop:** use one MIME contract (`application/x-axentra-message-ids`, `application/x-axentra-task-ids`) across Mail, Calendar, To Do and the My Day pane. Test dropping onto every target in 3.13.
25. **Do not build on EWS.** Exchange Online starts disabling EWS in October 2026 and switches it off completely in April 2027 [S51]. Everything Exchange-side in this file goes through Graph.
26. **Graph time zones:** send Windows time zone names (or one of Graph's listed IANA names) in `dateTimeTimeZone`; map from the system IANA zone with CLDR `windowsZones` [S57]. Convert UTC values back to the local zone before taking a date part (3.10.1).
27. **CardDAV is vCard 3.0 first.** Check `supported-address-data` before writing vCard 4.0, and degrade 4.0-only properties for 3.0 servers [S42].
28. **Normalise avatar colour keys to NFC** before hashing (2.6.3).
29. **iCalendar UTC rules:** `COMPLETED` and absolute `TRIGGER` values must be UTC, and DISPLAY alarms need a `DESCRIPTION` [S43]. Map `PRIORITY` 1-4 to high on read [S43].

### Open questions and resolutions (fact-check 2026-10-19)

1. **Personal contact lists on Microsoft 365 / Outlook.com.** Not EWS: it starts being disabled in October 2026 and is fully off in April 2027 [S51]. Graph v1.0 and beta have no contact-list API [S33][S62], but Microsoft lists "Contact Lists" (and "Additional Contact Properties") as Graph parity gaps with a Q3 2026 target [S51], which had passed without a visible API at the time of this check. **Resolution for v1:** read nothing from the server; let users create and edit lists on the device only, labelled "On this device", and add a Graph adapter behind a feature flag once Microsoft publishes the API. Re-check [S51] each release. **Still open:** whether lists created in Outlook can be shown at all before that API exists.
2. **My Day, manual order, list groups, contact favourites.** No Graph v1.0 property exists [S28][S29][S33]. **Resolution:** local SQLite store as the source of truth, roamed between Axentra installs through Graph open extensions on `todoTask`, `todoTaskList` and `contact` [S28][S29][S33]; CalDAV uses `X-AXENTRA-*` properties. No Axentra server is needed. **Still open:** Microsoft's "User Configuration objects" roadmap item (Q4 2026 [S51]) may expose Outlook's own settings objects; re-evaluate then.
3. **To Do navigation shortcut.** **Resolution:** `Ctrl+5` (and `Ctrl+Shift+5`), with `Ctrl+3` left unassigned. Neither client documents a To Do key; the documented module keys stop at People [S7]. **Still open:** whether new Outlook binds `Ctrl+5` to something undocumented.
4. **"Suggested" default.** **Resolution:** off by default with a one-time tip (2.12).
5. **CSV export headers.** Microsoft's CSV article offers a sample file [S53] whose header row could not be read during this check. **Still open:** capture the real header row by exporting from new Outlook and from the web (QA task in 2.13).
6. **Default visibility of "All" and "Completed", and the collapsed state of "Completed" sections.** Undocumented [S16]. **Resolution (estimate):** both smart lists hidden by default; "Completed" sections collapsed by default.
7. **Steps and My Day on CalDAV.** **Resolution:** steps as child VTODOs linked with `RELATED-TO` (default RELTYPE `PARENT` [S43]); My Day and order as `X-AXENTRA-*` properties, which other clients may ignore. **Still open:** how individual CalDAV apps (Nextcloud Tasks, Thunderbird, tasks.org, Apple Reminders) display child VTODOs, which needs hands-on testing.
8. **IMAP flagged email scope and completion.** **Resolution:** all subscribed mail folders except Trash, Junk, Drafts and Sent; completion keeps `\Flagged` and adds the private keyword `$Completed`, which is not IANA-registered [S54], only when `PERMANENTFLAGS` allows new keywords [S55] (3.12).
9. **Icons and list theme colours.** Microsoft documents only that a list theme can be a colour or an image, and that a leading emoji becomes the list icon [S26]. Fluent System Icon names and the 12-colour, no-photo theme set remain Axentra decisions. **Still open:** design sign-off.

---

## Sources

- [S1] https://support.microsoft.com/office/create-view-and-edit-contacts-and-contact-lists-in-outlook-ca469928-4b1c-47e4-ae85-15b6345ce043
- [S2] https://support.microsoft.com/en-us/outlook/people/manage-contacts-in-outlook
- [S3] https://support.microsoft.com/en-US/outlook/using-contacts-people-in-outlook-on-the-web
- [S4] https://support.microsoft.com/office/create-view-and-edit-contacts-and-contact-lists-in-outlook-com-5b909158-036e-4820-92f7-2a27f57b9f01
- [S5] https://support.microsoft.com/en-au/outlook/people/create-edit-or-delete-a-contact-list-or-contact-group-in-outlook
- [S6] https://support.microsoft.com/office/285a3b55-8d93-4ac8-93df-43fffd13b2f1
- [S7] https://support.microsoft.com/en-us/accessibility/outlook/keyboard-shortcuts-for-outlook
- [S8] https://support.microsoft.com/en-us/accessibility/outlook/use-a-screen-reader-to-explore-and-navigate-the-people-app
- [S9] https://kb.wisc.edu/microsoft365/page.php?id=70283 (third-party: University of Wisconsin KB, Featured people views)
- [S10] https://td.byui.edu/TDClient/79/ITHelpCenter/KB/PrintArticle?ID=11751 (third-party: BYU-Idaho KB, new Outlook contacts and lists)
- [S11] https://mc.merill.net/message/MC927963 (Microsoft 365 Message Center archive: People Hub improvements in new Outlook)
- [S12] https://mc.merill.net/message/MC1199768 (Microsoft 365 Message Center archive: new People Hub on the web)
- [S13] https://support.microsoft.com/office/use-my-day-with-to-do-in-outlook-6de6ee52-751b-4405-b389-850572b15306
- [S14] https://support.microsoft.com/topic/40deff88-941a-4fc0-aba1-7d929d947795
- [S15] https://support.microsoft.com/en-us/topic/6e8a991b-ea62-4009-a7f7-62b70a57ec18
- [S16] https://support.microsoft.com/en-us/outlook/create-and-manage-task-lists-with-my-day-in-outlook
- [S17] https://support.microsoft.com/en-us/todo/sort-and-search-in-lists
- [S18] https://support.microsoft.com/en-us/todo/add-due-dates-and-reminders-in-microsoft-to-do
- [S19] https://support.microsoft.com/en-us/todo/add-steps-importance-notes-tags-and-categories-to-your-tasks
- [S20] https://support.microsoft.com/ToDo/create-edit-delete-and-restore-tasks
- [S21] https://support.microsoft.com/en-us/office/using-keyboard-shortcuts-in-microsoft-to-do-ab4197e1-2922-463e-b6cc-e7f6fc8033f3
- [S22] https://support.microsoft.com/en-us/todo/using-microsoft-to-do-with-flagged-email-from-outlook
- [S23] https://support.microsoft.com/en-us/outlook/calendar/drag-a-message-to-create-a-task
- [S24] https://support.microsoft.com/en-US/Outlook/calendar/drag-a-task-to-your-calendar-with-to-do-in-outlook
- [S25] https://support.microsoft.com/ToDo/organize-your-lists
- [S26] https://support.microsoft.com/en-us/todo/customize-your-lists
- [S27] https://support.microsoft.com/en-us/accessibility/todo/use-a-screen-reader-to-work-with-tasks-in-to-do
- [S28] https://learn.microsoft.com/en-us/graph/api/resources/todotask?view=graph-rest-1.0
- [S29] https://learn.microsoft.com/en-us/graph/api/resources/todotasklist?view=graph-rest-1.0
- [S30] https://learn.microsoft.com/en-us/graph/api/resources/checklistitem?view=graph-rest-1.0
- [S31] https://learn.microsoft.com/en-us/graph/api/resources/linkedresource?view=graph-rest-1.0
- [S32] https://learn.microsoft.com/en-us/graph/api/resources/taskfileattachment?view=graph-rest-1.0 and https://learn.microsoft.com/en-us/graph/api/todotask-post-attachments?view=graph-rest-1.0
- [S33] https://learn.microsoft.com/en-us/graph/api/resources/contact?view=graph-rest-1.0
- [S34] https://github.com/microsoft/fluentui/blob/master/packages/react-components/react-avatar/library/src/components/Avatar/useAvatar.tsx
- [S35] https://github.com/microsoft/fluentui/blob/master/packages/react-components/react-avatar/library/src/utils/getInitials.ts
- [S36] https://github.com/microsoft/fluentui/blob/master/packages/react-components/react-avatar/library/src/components/Avatar/useAvatarStyles.styles.ts
- [S37] https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/colors.ts, https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/alias/lightColorPalette.ts, https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/alias/darkColorPalette.ts
- [S38] https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/alias/lightColor.ts, https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/alias/darkColor.ts, https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/brandColors.ts
- [S39] https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/fonts.ts, https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/spacings.ts, https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/borderRadius.ts, https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/durations.ts, https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/curves.ts, https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/utils/shadows.ts
- [S40] https://fluent2.microsoft.design/components/web/react/core/avatar/usage
- [S41] https://www.rfc-editor.org/rfc/rfc6350 (vCard 4.0)
- [S42] https://www.rfc-editor.org/rfc/rfc6352 (CardDAV)
- [S43] https://www.rfc-editor.org/rfc/rfc5545 (iCalendar, VTODO)
- [S44] https://www.rfc-editor.org/rfc/rfc4791 (CalDAV)
- [S45] https://windowscentral.com/microsoft-do-now-has-list-groups-all-users and https://onmsft.com/how-to/how-to-create-groups-of-lists-in-microsoft-to-do/ (third-party: list groups)
- [S46] https://windowscentral.com/you-can-now-add-tasks-microsoft-do-straight-top-windows-10 and https://techcommunity.microsoft.com/discussions/to-do/microsoft-to-do-problems-with-the-bell-sound/2264104/replies/3679171 (third-party / community: add on top, starred to top, completion sound setting)
- [S47] https://windowsreport.com/microsoft-to-do-hide-completed-tasks/ (third-party: Hide completed tasks)
- [S48] https://learn.microsoft.com/en-us/graph/api/resources/followupflag?view=graph-rest-1.0
- [S49] https://techcommunity.microsoft.com/discussions/to-do/flagged-email-behaviour/1106854 (community report: completing a flagged-email task)
- [S50] https://petri.com/outlook-on-the-web-contact-management/ (third-party: People hub favorites and quick call/message/email)
- [S51] https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/deprecation-of-ews-exchange-online (EWS disablement from October 2026, full shutdown April 2027; Graph parity roadmap incl. Contact Lists, Additional Contact Properties, User Configuration objects)
- [S52] https://learn.microsoft.com/en-us/graph/outlook-immutable-id (immutable IDs: supported resource types, lifetime, delta and subscription use, translateExchangeIds)
- [S53] https://support.microsoft.com/topic/4518d70d-8fe9-46ad-94fa-1494247193c7 (Create or edit .csv files to import into Outlook; sample file, UTF-8 advice)
- [S54] https://www.iana.org/assignments/imap-jmap-keywords/imap-jmap-keywords.xhtml (IANA IMAP and JMAP keyword registry)
- [S55] https://www.rfc-editor.org/rfc/rfc9051 (IMAP4rev2: system flags, keywords, PERMANENTFLAGS)
- [S56] https://www.rfc-editor.org/rfc/rfc9073 (iCalendar event publishing extensions: STYLED-DESCRIPTION)
- [S57] https://learn.microsoft.com/en-us/graph/api/resources/datetimetimezone?view=graph-rest-1.0 (accepted time zone names)
- [S58] https://learn.microsoft.com/en-us/graph/api/todotasklist-delta?view=graph-rest-1.0 and https://learn.microsoft.com/en-us/graph/api/todotask-delta?view=graph-rest-1.0 (To Do delta endpoints, permissions, national-cloud availability)
- [S59] https://learn.microsoft.com/en-us/graph/api/planneruser-list-tasks?view=graph-rest-1.0 (Planner tasks assigned to the user; Tasks.Read; no personal accounts)
- [S60] https://learn.microsoft.com/en-us/graph/api/contact-delete?view=graph-rest-1.0 (contact delete; Contacts.ReadWrite)
- [S61] https://learn.microsoft.com/en-us/graph/api/todotask-update?view=graph-rest-1.0 (todoTask update: writable properties, HTML-only body, UTC normalisation in the response)
- [S62] https://learn.microsoft.com/en-us/graph/api/resources/contact?view=graph-rest-beta (beta contact resource: no contact-list or favourite property)

Fact-check note (2026-10-19): Fluent token hex values in 1.1 (via the alias-to-ramp mappings in [S38] and the `brandWeb` ramp), the shadow patterns, the 30-entry avatar palette including the dark-red and plum shade20 exceptions [S37], the initials font and icon size rules [S36], the `getInitials` rules [S35] and the hash test vectors were all re-checked against the `microsoft/fluentui` master sources and match. Support articles were read through a summarising fetch tool, so quoted UI labels from them should still get a final visual check in a live new Outlook session before release.
