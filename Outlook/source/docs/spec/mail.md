# Axentra Outlook: Mail spec (folder pane, message list, reading pane, compose)

Status: Draft 1 (2026-10-19), independently fact-checked 2026-10-19 (see "Fact-check log" at the end)
Audience: Rust engineers building the Dioxus (HTML/CSS) desktop client
Reference product: new Outlook for Windows and Outlook on the web (OWA), Fluent 2 era (2024-2026). Where classic Outlook for Windows behaves differently, a "Classic:" note says so.

---

## 0. Conventions used in this document

| Marker | Meaning |
|---|---|
| **[MS]** | Stated in Microsoft public documentation (support.microsoft.com, learn.microsoft.com, Message Center notices). See Sources. |
| **[FL]** | Value taken from Fluent 2 design tokens in the MIT-licensed `microsoft/fluentui` repo (`packages/tokens`, `react-nav`). |
| **[3P]** | Reported by reputable third-party write-ups (university IT knowledge bases, how-to sites). Treat as likely, not guaranteed. |
| **[REF]** | Measured from the rendered Outlook on the web page and recorded in `docs/spec/reference-measurements.md` (October 2026). That file wins over this one for geometry; this file wins for behaviour. |
| **(estimate)** | Our best estimate, consistent with Fluent 2 guidance. Not documented anywhere. Must not be presented to users or testers as Outlook fact. |
| **(unverified)** | A claim that was previously tagged as sourced, but whose cited source does not support it (or no longer loads). Treat as an estimate until someone confirms it. |
| **(Axentra)** | Deliberate Axentra-specific decision or original copy text (not a claim about Outlook). |

- All UI copy in quotes is the exact label to render, in sentence case unless noted.
- "Brand" means the Axentra accent ramp. We reuse the Fluent web brand ramp values so the app reads as Fluent; no Microsoft logos or product artwork are used anywhere.
- Icons: use Fluent UI System Icons (MIT licence, `microsoft/fluentui-system-icons`). Icon names given here are suggestions and must be checked against the icon set version we vendor. Regular style at rest, Filled style for selected or toggled-on states.
- Sizes are CSS px at 100% zoom. The app must respect the GTK/desktop scale factor and the in-app zoom.

---

## 1. Shared foundations for the mail surface

### 1.1 Colour tokens (light and dark)

Values below resolve the Fluent alias tokens through the Fluent global grey and brand ramps. **[FL]** unless tagged.

| Axentra token | Fluent alias | Light | Dark | Used for |
|---|---|---|---|---|
| `--fg-1` | colorNeutralForeground1 | `#242424` | `#ffffff` | Primary text, sender, subject |
| `--fg-2` | colorNeutralForeground2 | `#424242` | `#d6d6d6` | Folder names, secondary labels |
| `--fg-3` | colorNeutralForeground3 | `#616161` | `#adadad` | Preview text, timestamps (read) |
| `--fg-4` | colorNeutralForeground4 | `#707070` | `#999999` | Placeholder, tertiary meta |
| `--fg-disabled` | colorNeutralForegroundDisabled | `#bdbdbd` | `#5c5c5c` | Disabled menu items and buttons |
| `--fg-on-brand` | colorNeutralForegroundOnBrand | `#ffffff` | `#ffffff` | Text on Send button |
| `--bg-1` | colorNeutralBackground1 | `#ffffff` | `#292929` | Message list and reading pane surface |
| `--bg-1-hover` | colorNeutralBackground1Hover | `#f5f5f5` | `#3d3d3d` | Row hover |
| `--bg-1-pressed` | colorNeutralBackground1Pressed | `#e0e0e0` | `#1f1f1f` | Row pressed |
| `--bg-1-selected` | colorNeutralBackground1Selected | `#ebebeb` | `#383838` | Neutral selection (menus) |
| `--bg-2` | colorNeutralBackground2 | `#fafafa` | `#1f1f1f` | Collapsed conversation rows |
| `--bg-3` | colorNeutralBackground3 | `#f5f5f5` | `#141414` | Attachment card rest |
| `--bg-4` | colorNeutralBackground4 | `#f0f0f0` | `#0a0a0a` | Folder pane / app canvas (Fluent Nav rest background) |
| `--bg-4-hover` | colorNeutralBackground4Hover | `#fafafa` | `#1f1f1f` | Folder row hover (Fluent Nav `backgroundColorHover`) |
| `--bg-4-pressed` | colorNeutralBackground4Pressed | `#f5f5f5` | `#000000` | Folder row pressed (Fluent Nav `backgroundColorPressed`) |
| `--bg-4-selected` | colorNeutralBackground4Selected | `#ffffff` | `#1a1a1a` | Available for a selected folder row; Fluent Nav itself sets no selected background |
| `--subtle-hover` | colorSubtleBackgroundHover | `#f5f5f5` | `#383838` | Icon button hover |
| `--subtle-pressed` | colorSubtleBackgroundPressed | `#e0e0e0` | `#2e2e2e` | Icon button pressed |
| `--subtle-selected` | colorSubtleBackgroundSelected | `#ebebeb` | `#333333` | Toggled icon button |
| `--stroke-1` | colorNeutralStroke1 | `#d1d1d1` | `#666666` | Input borders, card borders |
| `--stroke-2` | colorNeutralStroke2 | `#e0e0e0` | `#525252` | Row dividers, pane dividers |
| `--stroke-3` | colorNeutralStroke3 | `#f0f0f0` | `#3d3d3d` | Hairlines inside cards |
| `--brand-bg` | colorBrandBackground | `#0f6cbd` | `#115ea3` | Send button rest |
| `--brand-bg-hover` | colorBrandBackgroundHover | `#115ea3` | `#0f6cbd` | Send button hover |
| `--brand-bg-pressed` | colorBrandBackgroundPressed | `#0c3b5e` | `#0c3b5e` | Send button pressed |
| `--brand-bg-2` | colorBrandBackground2 | `#ebf3fc` | `#082338` | Info banners, chips selected |
| `--brand-fg-1` | colorBrandForeground1 | `#0f6cbd` | `#479ef5` | Unread subject, unread count, links |
| `--brand-fg-2` | colorBrandForeground2 | `#115ea3` | `#62abf5` | Link hover |
| `--brand-stroke` | colorCompoundBrandStroke | `#0f6cbd` | `#479ef5` | Unread accent bar, tab indicator |
| `--nav-indicator` | colorCompoundBrandForeground1 | `#0f6cbd` | `#479ef5` | Folder-pane selection pill. Fluent Nav uses this token, not colorCompoundBrandStroke; the values are the same today but keep the separate name. |
| `--nav-icon-selected` | colorNeutralForeground2BrandSelected | `#0f6cbd` | `#479ef5` | Filled icon of the selected folder (Fluent Nav) |
| `--shadow-ambient` | colorNeutralShadowAmbient | `rgba(0,0,0,0.12)` | `rgba(0,0,0,0.24)` | Ambient layer of every shadow (1.3) |
| `--shadow-key` | colorNeutralShadowKey | `rgba(0,0,0,0.14)` | `rgba(0,0,0,0.28)` | Key layer of every shadow (1.3) |
| `--focus-outer` | colorStrokeFocus2 | `#000000` | `#ffffff` | Keyboard focus ring |
| `--row-selected` | brand[150] / brand[40] | `#cfe4fa` | `#0c3b5e` | Selected message row (estimate) |
| `--row-selected-hover` | brand[140] / brand[50] | `#b4d6fa` | `#0e4775` | Selected row under pointer (estimate) |
| `--danger-fg` | red shade10 / red tint30 | `#bc2f32` | `#e37d80` | Validation errors, invalid chip (estimate of mapping) |
| `--flag-fg` | red primary / red tint30 | `#d13438` | `#e37d80` | Flag icon, high importance icon (estimate of mapping) |
| `--complete-fg` | green primary / green tint30 | `#107c10` | `#54b054` | Flag complete check (estimate of mapping) |
| `--pin-fg` | brand-fg-1 | `#0f6cbd` | `#479ef5` | Pinned icon (estimate) |
| `--draft-fg` | red shade10 / red tint30 | `#bc2f32` | `#e37d80` | "[Draft]" prefix in list (estimate) |
| `--highlight-search` | marigold tint40 / marigold shade30 | `#f9e2ae` | `#835b00` | Search term highlight (estimate) |

All alias-to-hex values in this table were re-checked on 2026-10-19 against `lightColor.ts`, `darkColor.ts` and the global grey ramp in `colors.ts`; they match. Dark-theme measurements in `reference-measurements.md` agree as well (pane/row `#292929`, secondary text `#adadad`, brand foreground `#479ef5`, disabled `#5c5c5c`) **[REF]**.

Brand ramp used (Fluent web brand, **[FL]**): 10 `#061724`, 20 `#082338`, 30 `#0a2e4a`, 40 `#0c3b5e`, 50 `#0e4775`, 60 `#0f548c`, 70 `#115ea3`, 80 `#0f6cbd`, 90 `#2886de`, 100 `#479ef5`, 110 `#62abf5`, 120 `#77b7f7`, 130 `#96c6fa`, 140 `#b4d6fa`, 150 `#cfe4fa`, 160 `#ebf3fc`.

### 1.2 Typography

Fluent web type ramp **[FL]**: sizes 10/12/14/16/20/24/28/32, line heights 14/16/20/22/28/32/36/40, weights 400/500/600/700.

| Role | Token | Size / line height / weight |
|---|---|---|
| Caption (badges, chips, group headers) | caption1 / caption1Strong | 12 / 16 / 400 or 600 |
| Body (rows, folders, menus) | body1 / body1Strong | 14 / 20 / 400 or 600 |
| Message list folder title | subtitle1 | 20 / 28 / 600. Measured as 20px semibold **[REF]** (was subtitle2 16/22 in Draft 1). |
| Reading pane subject | subtitle1 | 20 / 28 / 600 (estimate of role) |
| Empty state title | subtitle1 | 20 / 28 / 600 (estimate of role) |

- UI font stack: Fluent's base stack starts with `'Segoe UI'` **[FL]**; the measured Outlook page uses `"Segoe UI", "Segoe UI Web (West European)", -apple-system, BlinkMacSystemFont, Roboto, "Helvetica Neue", sans-serif` **[REF]**. Segoe UI is not redistributable and not present on Linux. Axentra stack, aligned with `reference-measurements.md`: `"Segoe UI Variable Text", "Segoe UI", Selawik, Inter, "Noto Sans", sans-serif` (Axentra). Selawik is Microsoft's open-source (OFL) fallback that is metric-compatible with Segoe UI, so bundle it; then check text metrics with Noto Sans as well, since that is what users without Selawik will see. Note: `calendar.md` 2.2 still appends `'Noto Sans', 'Cantarell', 'Ubuntu'` without Selawik; the two specs should share one stack constant.
- Compose default body font: the new Outlook web default is reported as Aptos 12 pt, replacing Calibri 11 pt **[3P]**. Microsoft's own default-font page still says Calibri **[MS]**. Aptos cannot be bundled. Axentra default: the body font family list `Aptos, Calibri, Carlito, "Noto Sans", sans-serif` at 12 pt (Axentra). Carlito is metric-compatible with Calibri.

### 1.3 Spacing, radius, elevation, motion

- Spacing scale **[FL]**: 2, 4, 6, 8, 10, 12, 16, 20, 24, 32.
- Corner radius **[FL]**: small 2, medium 4, large 6, x-large 8, circular. Axentra uses medium (4) for rows, buttons and inputs; x-large (8) for cards, menus and dialogs (estimate).
- Shadows **[FL]** (each is two layers, an ambient one and a key one): shadow4 = `0 0 2px` + `0 2px 4px`; shadow8 = `0 0 2px` + `0 4px 8px`; shadow16 = `0 0 2px` + `0 8px 16px`; shadow28 = `0 0 8px` + `0 14px 28px`. The shadow helper takes its colours from the alias tokens `colorNeutralShadowAmbient` and `colorNeutralShadowKey`: alpha 0.12 and 0.14 in light mode, 0.24 and 0.28 in dark mode **[FL]** (`--shadow-ambient` / `--shadow-key` in 1.1; Draft 1 wrongly called these an estimate). Menus and flyouts use shadow16, dialogs shadow28, drag ghost shadow8 (estimate of assignment).
- Motion: hover background transitions 100 ms linear (matches Fluent Nav `durationFaster`/`curveLinear` **[FL]**); menus fade+slide 150 ms ease-out (estimate). Honour `prefers-reduced-motion`.
- Focus ring: 2px solid `--focus-outer`, inset by 2px on rows (Fluent Nav uses `strokeWidthThick` with negative offset **[FL]**). Only show on keyboard focus (`:focus-visible`).

### 1.4 Overall mail layout

```
+-----+----------------+----------------------+------------------------------------+
| App | Folder pane    | Message list         | Reading pane                       |
| bar | (Favorites,    | (header, pivot,      | (conversation / compose)           |
|     |  accounts)     |  grouped items)      |                                    |
+-----+----------------+----------------------+------------------------------------+
```

- The ribbon above this layout and the app bar on the far left are specified in `ribbon.md` / `shell.md`. This document references their mail commands only.
- Surfaces **[REF]**: the ribbon is a rounded (8px) card floating on the canvas; the folder pane has no surface of its own and sits directly on the canvas (`--bg-4`); the message list region starts immediately to its right (measured x = 264 for a folder pane spanning x = 50 to 266, i.e. no visible gap) and has a 4px top-right radius. Draft 1's "radius 8, gap 4 to 8 between panes" is superseded. The reading pane surface treatment was not measured; use `--bg-1` with the same 4px outer radius (estimate).
- Reading pane position options **[MS]**: Settings > Mail > Layout gives "Right", "Bottom", "Fill screen", "Popout only". The ribbon View tab gives "Show on the right", "Show on the bottom", "Hide" **[MS]**. Default: Right (estimate; Microsoft does not state a default).
- After moving or deleting the open item **[MS]**: "Open the next item" / "Open the previous item" / "Return to the message list". Microsoft documents this for the new Outlook only; the Outlook on the web page does not list it. Default: Open the next item (estimate).
- Responsive behaviour (estimate): window width < 1100 px makes the folder pane auto-collapse; < 760 px uses single-column navigation (list, then reading pane with a back button).

| Pane | Default width | Min | Max | Resizable |
|---|---|---|---|---|
| Folder pane | 216 **[REF]** (measured at a 1440px window; the Fluent Nav drawer default of 260 **[FL]** is not what Outlook uses) | 180 (estimate) | 400 (estimate) | Yes, splitter |
| Message list (pane on right) | 380 (estimate; not measured, the reference capture had the reading pane hidden) | 300 (estimate) | 55% of window (estimate) | Yes, splitter |
| Reading pane | fills the rest | 420 (estimate) | n/a | via neighbours |

- Vertical frame **[REF]**: app header 48px; ribbon card from y = 48, 81px tall (tab row 31px, command row 40px); the folder tree starts about 14px below the ribbon card and the message list header about 8px below it. Owned by `shell.md` / `ribbon.md`; repeated here so mail layout code does not hard-code different numbers.

Splitters: 1px visible `--stroke-2` line with an 8px invisible hit area, `col-resize` cursor. Double-click a splitter to reset to the default width (Axentra). Widths are saved per window.

---

## 2. Folder pane

### 2.1 Structure (top to bottom)

1. **Collapse toggle**: in the new Outlook this is the hamburger button at the far left of the ribbon card, left of the tabs, 40x32 **[REF]**. Its label reads "Hide navigation pane" **[REF]**; the "Show navigation pane" label for the collapsed state is an estimate. Collapsing hides the folder pane entirely; the app bar stays. State persists.
2. **Favorites** section (can be pinned, see 2.3).
3. One **account section** per account, in the order the accounts were added. Each has a header and a folder tree.
4. Optional extra sections that other specs own: Groups, shared mailboxes, Search folders.
5. Optional footer row: "Add account" (estimate; shell spec owns it).

The pane scrolls vertically. With Favorites pinned, only the region below Favorites scrolls **[MS]**.

### 2.2 Row anatomy and states

Based on the Fluent Nav item **[FL]**: padding 10px top and bottom, 8px right, 10px left, body1 text, radius 4, 20px icon slot, and a 16px flex gap (`spacingVerticalL`) on the item root. The Fluent default gives a 40px row; the Fluent "small" variant (4px vertical padding) gives about 28px.

Measured Outlook folder rows are **36px tall, radius 4, 14px text, inset 8px from the pane's left edge** (row x = 58 in a pane starting at x = 50; row width 190 in a 216px pane) **[REF]**. The capture was taken at the user's own density setting, so Compact density may be shorter (unverified). Axentra uses 36px for the default density (Draft 1 used 32px).

```
[8 inset][indent][chevron 16][icon 20][8][Folder name ........ ][count]
```

The 8px icon-to-text gap is an estimate; Fluent Nav's root gap is 16px **[FL]**. Compare against a reference screenshot before tuning.

| Element | Spec |
|---|---|
| Indent | 16px per nesting level (estimate) |
| Chevron | 16px, `chevron_right` / `chevron_down`. Only on folders with children. Clicking it toggles without selecting the folder. |
| Icon | 20px Fluent System Icon, `--fg-2`. Filled variant in `--nav-icon-selected` when selected (Fluent Nav: filled icon on selection **[FL]**). |
| Name | body1 `--fg-2`. body1Strong `--fg-1` when the folder has unread items or is selected (estimate for unread; selected uses body1Strong in Fluent Nav **[FL]**). Single line, end ellipsis, full name in a tooltip. |
| Count | See 2.6. Right aligned, 8px from the right edge, in brand colour **[REF]**. |
| Selected indicator | 4px x 20px pill, circular radius, `--nav-indicator`, vertically centred, drawn 16px before the content start (Fluent `indicatorOffset`) **[FL]**. Fades in over 100 ms. |

| State | Background | Text |
|---|---|---|
| Rest | transparent over `--bg-4` | `--fg-2` |
| Hover | `--bg-4-hover` **[FL]** (Fluent Nav; Draft 1 had `--subtle-hover`) | `--fg-1` |
| Pressed | `--bg-4-pressed` **[FL]** | `--fg-1` |
| Selected | Fluent Nav defines no selected background **[FL]**; Axentra adds `--subtle-selected` (estimate; check against the reference before shipping) plus the indicator pill | body1Strong `--fg-1` |
| Drop target (during drag) | `--brand-bg-2` with a 1px `--brand-stroke` inset border (estimate) | `--fg-1` |
| Keyboard focus | 2px focus ring inset | unchanged |
| Disabled (offline and the folder is not cached) | transparent | `--fg-disabled` (estimate) |

Hovering a row shows a "More options" (`more_horizontal`) icon button at the right, which replaces the count while hovered. This matches the documented "hover over it and select More options" path **[MS]**. The button opens the same menu as right-click.

### 2.3 Favorites section

- Header row: chevron and the label "Favorites", caption1Strong `--fg-2`, 28px tall (estimate). Its hover "More options" menu (2.8) holds "Pin favorites" / "Unpin favorites" **[MS]** and "Hide favorites" **[MS]**.
- Default contents: re-checked 2026-10-19, the two Microsoft pages still disagree, and the difference is per product rather than an error in either. The new Outlook Favorites article says Inbox and Deleted Items **[MS]**. The Outlook on the web folders article says Inbox, Sent Items and Deleted Items **[MS]**. Axentra seeds Inbox, Sent Items, Deleted Items for the first account (Axentra), which matches the web, not the new Outlook. If strict parity with the new Outlook for Windows is the goal, drop Sent Items from the seed.
- Favorites are not exposed by any documented Microsoft Graph API, so Axentra stores them locally per account (Axentra). They will not roam to or from Outlook.
- Items that can be added **[MS]**: mail folders (default or custom), shared folders the user can open, contacts or frequent contacts (they show that person's mail), categories (they show all items with that category), search folders.
- Add: right-click a folder and choose "Add to Favorites", or drag a folder into the Favorites section **[MS]**. Favoriting a category from Manage categories (star) also adds it here **[3P]**.
- Remove: right-click a favorite and choose "Remove from Favorites". This removes only the shortcut, never the folder **[MS]**.
- Reorder: drag within Favorites **[MS]**, or use "Move up" / "Move down" in the context menu **[MS]** (OWA label).
- Show again after hiding: View > Layout > Folder pane > "Show favorites" **[MS]**. Axentra puts this under the View tab of the ribbon.
- Pin from the ribbon: View > Folder pane > Favorites > "Pin or Unpin favorites" **[MS]**. Pinning keeps Favorites fixed at the top while the rest of the folder list scrolls (rolled out to the new Outlook and the web December 2024 to January 2025, MC926191) **[MS]**.
- Unread counts on a favorite mirror the target folder's count.
- Multiple accounts: when two favorites have the same name (for example two Inboxes), add the account's display name or email address as a caption1 `--fg-3` suffix (Axentra).

### 2.4 Account section

- Header row: chevron, then the account's email address **[MS]** (Microsoft's create-folder steps for the new Outlook say to right-click "your email address" in the folder pane), body1Strong `--fg-1`, 36px to match the folder rows. Hover shows "More options" **[MS]**. Right-click (or More options) gives "Create new folder" **[MS]**; type the name and press Enter **[MS]**.
- In OWA and Outlook.com the top-level node is "Folders" and its right-click also gives "Create new folder" **[MS]**. Axentra follows the new Outlook (account address).
- Below the tree: a "New folder" text button, `add` icon, `--brand-fg-1` (unverified: Draft 1 cited Microsoft for a "New folder" button at the bottom of the OWA folder list, but the current create-folder article does not mention it; keep it as an Axentra affordance).

### 2.5 Default folders: names, order, icons

Display names come from the server (they are localised server-side). Identify folders by role, never by name: Microsoft Graph well-known names **[MS]**, or IMAP SPECIAL-USE attributes (RFC 6154).

| Order | Display name (en-US) | Graph well-known name | IMAP special-use | Icon (suggested) | Shown when |
|---|---|---|---|---|---|
| 1 | Inbox | `inbox` | (INBOX) | `mail_inbox` | Always |
| 2 | Drafts | `drafts` | `\Drafts` | `drafts` | Always |
| 3 | Sent Items | `sentitems` | `\Sent` | `send` | Always |
| 4 | Snoozed / Scheduled | `scheduled` (mapping unverified) | n/a (local) | `clock_snooze` | After the first snooze. Microsoft says a folder named "Snoozed" appears **[MS]**. Graph's `scheduled` well-known name is documented only as the folder for the Outlook for iOS "Schedule" feature **[MS]**; that it is the same folder as "Snoozed" is our inference (unverified). Show the server's display name. |
| 5 | Deleted Items | `deleteditems` | `\Trash` | `delete` | Always |
| 6 | Junk Email | `junkemail` | `\Junk` | `mail_prohibited` | Always |
| 7 | Archive | `archive` | `\Archive` | `archive` | If it exists, or after the first Archive action |
| 8 | Notes | n/a | n/a | `note` | If the server has it. Axentra shows it read-only (Axentra). Draft 1 cited Microsoft for "read-only"; no current source says so (unverified). Graph access to Exchange notes (`IPM.StickyNote`) is on Microsoft's EWS-parity roadmap for Q3 2026 **[MS]**. |
| 9 | Conversation History | `conversationhistory` | n/a | `history` | If the server has it. Graph describes it as the folder where Skype saves IM conversations **[MS]** (Draft 1 said Skype/Teams; Teams is not mentioned). |
| 10 | Outbox | `outbox` | n/a | `mail_arrow_up` | Only while it holds unsent items (estimate) |
| 11 | RSS Feeds / RSS Subscriptions | n/a | n/a | `rss` | If the server has it. Treat as a plain mail folder; Axentra does not fetch RSS (Axentra). |
| 12+ | User folders | n/a | n/a | `folder` | A to Z, case-insensitive, locale collation |
| last | Search folders | `searchfolders` | n/a | `folder_search` | If any are defined (owned by search spec) |

- Order note: rows 1 to 9 follow the order the reference task asked for. Microsoft's Outlook on the web folders article lists the starting folders as Inbox, Junk Email, Drafts, Sent Items, Deleted Items, in that sequence **[MS]**, but it does not say that this is the display order. Outlook.com is commonly seen with Junk Email directly under Inbox; work accounts in the new Outlook are not confirmed either way (unverified). Keep the order in a single constant, with a per-provider override (consumer Outlook.com: Junk Email second), so it can change in one place.
- Never show these (Graph names **[MS]**): `syncissues`, `conflicts`, `localfailures`, `serverfailures`, `recoverableitemsdeletions`, `msgfolderroot`, `clutter` (the folder of the retired Clutter feature **[MS]**; Draft 1 called it classic-only, which Graph does not say), and any folder with `isHidden = true`.
- Graph omits hidden folders from `GET /me/mailFolders` unless the request adds `includeHiddenFolders=true` **[MS]**. Do not add that parameter for the folder pane. `isHidden` can only be set at creation **[MS]**.
- Default folders cannot be renamed or deleted (examples given: Inbox, Drafts, Deleted Items, Sent Items) **[MS]**. Hide Rename and Delete folder for them.
- IMAP accounts may have no `\Archive` (or other) special-use folder. When the server advertises `CREATE-SPECIAL-USE` (RFC 6154 section 3), create Archive with the `USE (\Archive)` attribute; otherwise create a plain folder and remember its name locally. See `protocols.md` for the capability list.
- Classic: classic Outlook also shows Outbox, Sync Issues (optionally), RSS Feeds, and Search Folders as a tree node.

### 2.6 Count badges

| Folder | Number shown | Style |
|---|---|---|
| Inbox, user folders, Archive, Deleted Items, Junk Email | Unread count (`unreadItemCount` **[MS]**) | `--brand-fg-1`, right-aligned **[REF]**; body1Strong weight (estimate) |
| Drafts | Total item count (`totalItemCount` **[MS]**). Drafts are not "unread". | body1 `--fg-3` (estimate) |
| Outbox, Snoozed/Scheduled | Total item count (estimate) | body1 `--fg-3` (estimate) |
| Notes, Conversation History, Sent Items | No count (estimate) | n/a |

- Hide the badge when the count is 0.
- Format: integer with no thousands separator up to 9999, then "9999+" (estimate).
- A collapsed parent folder does not add its children's counts (estimate; classic behaves this way).
- With Focused Inbox on, Inbox shows the total unread across Focused and Other (estimate).

### 2.7 Collapse / expand

- Favorites, each account section, and each parent folder can collapse. Remember the state per node per account.
- Keyboard **[MS]**: Right arrow expands, Left arrow collapses. Left on an expanded node collapses it; Left on a collapsed or leaf node moves focus to its parent (Axentra, standard tree behaviour).
- While dragging, hovering a collapsed parent for 700 ms auto-expands it (estimate).

### 2.8 Folder context menus

Order matters. A separator is shown as "---". Items marked "(classic)" are classic Outlook only and are not built in v1.

User (custom) folder, right-click or "More options":

1. "Create new subfolder" **[MS]**
2. "Rename" **[MS]**
3. "Delete folder" **[MS]**: confirmation dialog, then the folder and its contents move to Deleted Items **[MS]**
4. ---
5. "Mark all as read" **[MS]**
6. "Delete all" **[MS]** (OWA label; moves every item to Deleted Items). Classic label: "Empty folder".
7. ---
8. "Add to Favorites" / "Remove from Favorites" **[MS]**
9. "Move" (opens the folder picker; Axentra addition, since OWA moves folders by drag **[MS]**)
10. "Permissions" (Exchange accounts only; classic **[MS]**; v2)

Default folder: the same menu without Rename, Delete folder and Move. Additions:

- Deleted Items: "Empty folder" (estimate of label; permanent delete, needs confirmation). Recovery of items already removed from Deleted Items: Microsoft documents a "Recover" control at the top of the Deleted Items message list, after which the user selects items and chooses "Restore" **[MS]**. Axentra's label "Recover items deleted from this folder" is our wording (estimate). Exchange only; on Graph the items live in `recoverableitemsdeletions`.
- Junk Email: "Delete all" **[MS]** (Microsoft's junk-filter article names this command for clearing Junk Email; Draft 1 used "Empty folder").

Favorites item: "Remove from Favorites" **[MS]**, "Move up" **[MS]**, "Move down" **[MS]**, ---, "Mark all as read", "Create new subfolder" (only for a real folder target).

Favorites header: "Pin favorites" / "Unpin favorites" **[MS]**, "Hide favorites" **[MS]**.

Account header: "Create new folder" **[MS]**, ---, "Mark all as read" (all folders; estimate), "Account settings" (opens settings, owned by settings spec).

"Mark all as read" backends: Graph v1.0 has no single "mark folder as read" call; Microsoft lists it as an EWS-parity gap with a Q4 2026 target **[MS]**. Until it ships, page through `GET .../messages?$filter=isRead eq false&$select=id` and send `PATCH {isRead:true}` in JSON batches of up to 20 requests, updating the local count optimistically. IMAP: `UID STORE 1:* +FLAGS.SILENT (\Seen)` on the selected folder.

### 2.9 Create, rename, move, delete folders

- Create: an inline text field opens as a new row in place, either under the parent (subfolder) or at the top level (estimate of placement). Placeholder "New folder" (Axentra). Enter commits **[MS]**, Esc cancels, losing focus commits if the field is not empty (estimate).
  - Validation (Axentra copy): an empty name is not allowed (no error; the row is simply removed). A duplicate sibling name shows the inline error "A folder with this name already exists." Names containing the account's IMAP hierarchy delimiter are rejected with "Folder names can't contain \"/\"." (show the actual delimiter character).
  - Optimistic insert, then reconcile with the server id. On failure, remove the row and show a toast: "Couldn't create the folder. Try again." (Axentra).
- Rename: the field opens inline with the name preselected. Same validation.
- Move: drag a folder onto another folder **[MS]**, or use "Move" (picker). Moving a folder into itself or into its own descendant is not allowed (the drop cursor shows not-allowed).
- Delete: dialog title "Delete folder?" with body "\"{name}\" and everything in it will be moved to Deleted Items." and buttons "Delete" (primary) and "Cancel" (Axentra copy). A folder that is already inside Deleted Items is deleted permanently, and the body text says so.

---

## 3. Message list

### 3.1 Header

```
[Folder name (subtitle1)] [star]          [Select] [Jump to] [Filter] [By Date v]
[Focused | Other]  (Inbox only, when Focused Inbox is on)
```

| Element | Spec |
|---|---|
| Folder title | subtitle1 (20px semibold **[REF]**), `--fg-1`, one line with ellipsis. In search mode it is replaced by "Results" (estimate). |
| Favorite star | 20px icon button next to the title **[REF]**, "Add to Favorites" / "Remove from Favorites" (labels: estimate) |
| Select | "Select" control, right-aligned group **[REF]**. Icon `checkbox_checked` / `checkbox_unchecked` (estimate of icon). Turning it on shows checkboxes on every row and a header "Select all" checkbox. |
| Jump to | "Jump to" control **[REF]**. Behaviour not documented; Axentra opens a date picker that scrolls the list to the first item on or before the chosen date when sorted by date, and a letter picker when sorted by From or Subject (estimate). |
| Filter button | Icon button `filter`, label "Filter". At the top of the message list **[MS]**, also seen there **[REF]**. Shows a dot badge or filled icon while a non-default filter is active (estimate). |
| Sort button | Drop-down showing the current sort, e.g. "By Date" with a chevron **[REF]**. See 3.3. |
| Focused/Other pivot | Tab list: "Focused" and "Other" **[MS]**. Inbox only **[MS]**. Tabs are body1, selected tab body1Strong, with a 3px `--brand-stroke` underline indicator (estimate per Fluent TabList). Height 40 (estimate). |
| Header height | About 44 for the title row **[REF]**, plus 40 for the pivot (estimate) |

Focused Inbox **[MS]**: the label is "Sort messages into Focused and Other". Microsoft's Focused Inbox article (current) places it under Settings > Mail > Message list for both the new Outlook and Outlook on the web, and in the new Outlook you choose the account there first; Microsoft's mailbox-appearance article still says Settings > Mail > Layout. Turning it off is also worded "Don't sort my messages" **[MS]**. It is per account in the new Outlook **[MS]**. It maps to Graph `inferenceClassification` = `focused` or `other`.

When Other gets new mail while Focused is shown, show a one-line info row under the pivot: "New messages in Other" with a link "View" (Axentra copy; Microsoft says the user is informed but gives no wording **[MS]**).

### 3.2 Filter menu

Opened from the Filter button. Single-select radio group. Default "All".

| Label | Behaviour | Source |
|---|---|---|
| "All" | No filter | estimate of label |
| "Unread" | `isRead = false` | [3P] |
| "Flagged" | `flag.flagStatus = flagged` | [MS] (Filter, then flagged) |
| "To me" | User is in To or Cc | [3P] |
| "Has files" | `hasAttachments = true` (non-inline attachments) | [3P] / "Has files" sort label [MS, Mac context] |
| "@ Mentions me" | Message mentions the user. Graph `mentionsPreview.isMentioned` is beta-only, which `protocols.md` bans in shipped code, so use a local check (the user's own address in a `mailto:` link in the body) until it reaches v1.0. Mentions work only for Exchange 2016/2019, Exchange Online and Outlook.com mailboxes, and the @ indicator only on Exchange Online and Outlook.com **[MS]**. | [MS] |

- Active filter: show a dismissible chip under the header, for example "Filter: Unread  ✕" (Axentra).
- An empty result shows the filter empty state (3.13).
- Filters and sort settings are kept per folder (estimate).

### 3.3 Sort options

Microsoft's sort article says sorting is reached through the "Filter" control at the top of the message list in the new Outlook and on the web **[MS]**. The October 2026 reference capture shows a separate sort drop-down ("By Date" with a chevron) to the right of Filter **[REF]**. Axentra follows the capture: a separate sort button, with the same "Sort by" items also listed at the end of the Filter flyout so both paths work. Fields Microsoft documents: date, category, sender, size, importance, subject, "and more" **[MS]**. The labels below (other than the field list) are estimates, except that the sort button's resting label format "By Date" is **[REF]**.

| Sort by label | Field | Order labels (toggle) | Group headers |
|---|---|---|---|
| "Date" (default) | received date (sent date in Sent Items/Drafts) | "Newest on top" / "Oldest on top" (estimate) | Date buckets (3.4) |
| "From" | sender display name, falling back to address | "A to Z" / "Z to A" (estimate) | Sender name |
| "Category" | first category name | "A to Z" / "Z to A" (estimate) | Category name; "No category" last |
| "Size" | size in bytes | "Largest on top" / "Smallest on top" (estimate) | Size buckets (below) |
| "Importance" | high > normal > low | "High on top" / "Low on top" (estimate) | "High", "Normal", "Low" |
| "Subject" | subject without Re:/Fw: prefixes | "A to Z" / "Z to A" (estimate) | First letter |
| "Flag status" | flagged > complete > none | "Flagged on top" (estimate) | "Flagged", "Completed", "Not flagged" (estimate) |

Size buckets (classic Outlook ranges; estimate for the new Outlook): Enormous (> 5 MB), Huge (1 to 5 MB), Very large (500 KB to 1 MB), Large (100 to 500 KB), Medium (25 to 100 KB), Small (10 to 25 KB), Tiny (< 10 KB).

Pinned messages float to the top only when sorting by date **[3P]**.

### 3.4 Date group headers

Documented: Microsoft's classic Outlook grouping article names the default Inbox groups Today, Yesterday, Last Week, Last Month and Older **[MS]**. For the new Outlook, a Microsoft moderator answer mentions sections "such as Yesterday and This week" **[MS, Q&A]**, and the October 2026 reference capture shows "Yesterday" and "This Month" headers **[REF]**. Axentra bucket rules, evaluated in the user's time zone with the locale's first day of the week:

| Order | Header | Rule |
|---|---|---|
| 0 | "Pinned" | Pinned items, only when sorted by date [3P] |
| 1 | "Today" | Same calendar day as now. Future-dated items also go here (estimate). |
| 2 | "Yesterday" | Previous calendar day [REF] |
| 3 | "This week" | In the current week and earlier than yesterday [MS, Q&A] |
| 4 | "Last week" | In the previous calendar week (classic [MS]; assumed for the new Outlook) |
| 5 | "This month" | Current month, older than all of the above. Seen as "This Month" (title case) in the capture **[REF]**; render it as captured until a sentence-case sighting exists. Replaces Draft 1's "Earlier this month". |
| 6 | "Last month" | Previous calendar month (classic [MS]; assumed for the new Outlook) |
| 7 | "Older" | Everything else (classic [MS]; assumed for the new Outlook) |

- Draft 1 also had "Two weeks ago" and "Three weeks ago". Those are classic Outlook arrangement groups (unverified) and nothing shows them in the new Outlook, so they are dropped.
- A bucket that has no items is not rendered. When "Yesterday" falls in the previous week, it still wins over "Last week" (rules are evaluated top-down).
- Header style: caption1Strong `--fg-2`, 28px tall, 12px left padding, sticky while its group is scrolled (estimate). In the columns layout the headers are full-width rows with a collapse chevron **[REF]**, so the chevron moves from v2 to v1.
- Can be turned off **[MS, Q&A]**: Settings > Mail > Layout > Date header > "Don't show date header in message list". (The source is an accepted moderator answer, not a support article.)

### 3.5 Item anatomy

Density is a View-tab setting (View > Density) **[MS]**: "Roomy" shows sender images, attachment previews and preview text; "Cozy" shows preview text only, with some spacing; "Compact" shows preview text only, with minimal spacing **[MS]**. There are no separate toggles for sender image or attachment preview; they come only with Roomy **[MS]**. Microsoft states no default for new installs; Axentra default: Roomy (estimate). The message list format setting (Settings > Mail > Layout > Message list format) is "Sender name first" / "Subject first" **[MS]**.

Hover actions **[MS]** (Message Center MC1041465): in the new Outlook and on the web, hover actions moved into the Sender/From column, freeing the Received column. Microsoft says this mainly affects the layouts with the reading pane at the bottom or hidden, i.e. the single-line columns layout. Rollout: targeted release mid-July to mid-August 2025, general availability mid-September to early November 2025. In the three-line layouts, hover actions sit on the right of line 1 (estimate).

**Roomy (three lines, reading pane on the right):**

```
|bar| [avatar 32] | Sender name .................. [hover actions] |
|   |             | Subject ........ [icons] [time]                 |
|   |             | Preview text .............. [category pills]    |
|   |             | [attachment preview chips]  (optional row)      |
```

| Part | Spec |
|---|---|
| Row padding | 12 top/bottom, 12 left (after the bar), 12 right (estimate) |
| Unread bar | 3px wide **[REF]** (measured in the columns layout), at the row's left edge, `--brand-stroke`. Full height minus 8px top/bottom inset and radius 2 are estimates. |
| Avatar | 32px circle (estimate). Shows the sender's photo when available. Otherwise initials: the first letter of the first and last words of the display name, or the first two letters of a single word, or the first letter of the address local part. Background and foreground come from the Fluent colorful avatar palette, hashed from the address (Fluent persona tokens: Background2 = tint40 / Foreground2 = shade30 in light, shade30 / tint40 in dark **[FL]**). On hover, or in select mode, the avatar is replaced by a 20px checkbox centred in the same 32px slot. |
| Line 1 | Sender display name, body1 `--fg-1`, ellipsis. Drafts show the recipients instead. Sent Items shows "To: {recipients}" (estimate). A conversation shows participant first names separated by "; " (estimate). |
| Line 2 | Subject, body1 `--fg-1`, ellipsis. Then the status icons (16px, gap 4). Time is right-aligned, caption1 `--fg-3`. |
| Line 3 | Preview, body1 `--fg-3`, one line, ellipsis. Uses the first non-quoted text of the body (Graph `bodyPreview`, up to 255 characters **[MS]**). Category pills are right-aligned (3.7). |
| Attachment chips | Up to 3 chips, 24px tall, file-type icon plus name (max width 140, ellipsis), then "+N" (estimate). Shown only in Roomy **[MS]**. Not shown for inline images. |
| Height | 84px, or 114px with the attachment row (estimate) |
| Divider | None between rows. Rows are separated by padding only (estimate). |

**Cozy:** the same three lines, no avatar (the checkbox slot appears on hover, 20px), padding 8 top/bottom, about 76px tall (estimate).

**Compact:** two lines, padding 6, about 52px tall, no avatar (estimate). Line 1: sender, then hover actions or time. Line 2: subject, an em-space, then the preview inline in `--fg-3`, then icons (estimate).

**Single-line columns layout (reading pane on the bottom or hidden, list width ≥ 900px):**
measured **[REF]**: rows 37px; column headers "From | Subject | Received ⌄" above the rows; a checkbox column at the left; the Subject column shows the subject followed by the preview text on the same line in secondary colour; date group headers are full-width rows with a collapse chevron; in dark mode row background `#292929` and text `#d6d6d6`.
Axentra column widths (estimate): `[checkbox 32][From 220, hover actions overlay its right end][icons 64][Subject + preview (flex)][categories][Received 110]`. Hover actions live in the From column per MC1041465 **[MS]**, not in a separate column (Draft 1 had a separate column). Clicking a column header sorts by it (estimate).

**Draft rows:** the line-2 subject is prefixed by "[Draft]" in `--draft-fg` (estimate of styling). A scheduled draft shows "Scheduled: {date time}" in caption1 `--brand-fg-1` (estimate).

**Group-by-conversation rows:** see 3.10.

### 3.6 Status icons (16px, order left to right on line 2)

| Icon | Fluent icon (suggested) | Colour | Condition |
|---|---|---|---|
| High importance | `important` (exclamation) | `--flag-fg` | `importance = high` |
| Low importance | `arrow_down` | `--brand-fg-1` | `importance = low` |
| @ mention | `mention` | `--brand-fg-1` | User is @mentioned. Shown automatically in the new Outlook **[MS]**. |
| Meeting invite / response | `calendar` / `calendar_checkmark` / `calendar_cancel` | `--fg-3` | Item is a meeting request, accept, decline or cancellation |
| Attachment | `attach` (paperclip) | `--fg-3` | `hasAttachments` **[MS]** |
| Replied / Forwarded | `arrow_reply` / `arrow_forward` | `--fg-3` | Last verb executed (estimate that the list shows this) |
| Encrypted / Signed | `lock_closed` / `ribbon` | `--fg-3` | S/MIME or other protection |
| Pinned | `pin` (Filled) | `--pin-fg` | Pinned (always shown, not only on hover) |
| Flag | `flag` (Filled) / `checkmark` | `--flag-fg` / `--complete-fg` | Flagged / complete (always shown) |

When hover actions overlap pinned or flag icons, the hover action with the same meaning replaces the icon in place, so nothing jumps (estimate).

### 3.7 Category pills

- Pill: height 20, horizontal padding 6, radius 4 (estimate), caption1 text. Colours come from section 9.
- Show at most 2 pills, then "+N" (estimate). The tooltip lists all categories.
- Clicking a pill filters to that category. In Inbox the filter spans all folders; in other folders it covers that folder only **[3P]**.

### 3.8 Time format rules (message list)

Locale-aware through ICU skeletons; the en-US examples are below. Use 12h or 24h from the OS locale.

| Item age | Format | Example |
|---|---|---|
| Today | time | "3:45 PM" (seen as "10:30 AM" **[REF]**) |
| Within the last 6 days (excluding today) | short weekday + time | "Mon 3:45 PM" (seen as "Sun 8:16 PM" **[REF]**) |
| Older, same calendar year | three-line layouts: short weekday + M/D, "Thu 9/26" (estimate). Columns layout: M/D/YYYY **[REF]** | |
| Previous years | M/D/YYYY | "9/26/2023" (columns layout **[REF]**; three-line layouts estimate) |

- Tooltip on the time: full date and time, for example "Monday, October 14, 2024 3:45 PM" (estimate).
- The list shows received time; Drafts and Sent Items show last-modified and sent time (estimate).
- Re-render the labels at midnight and when the time zone changes.

### 3.9 Row states

| State | Background | Text changes |
|---|---|---|
| Rest, read | `--bg-1` | Sender body1 `--fg-2`, subject body1 `--fg-2`, time `--fg-3`. Dark mode measured `#d6d6d6` (= `--fg-2`) for row text and `#adadad` (= `--fg-3`) for time labels **[REF]**; Draft 1 used `--fg-1`. Light mode not measured; `--fg-2` assumed for consistency. |
| Rest, unread | `--bg-1` + 3px unread bar **[REF]** | Columns layout: sender and subject both body1Strong in `--brand-fg-1` **[REF]**. Three-line layouts: sender body1Strong `--fg-1`; subject body1Strong `--brand-fg-1`; time caption1Strong `--brand-fg-1` (estimate of exact colouring). |
| Hover | `--bg-1-hover` | Hover actions appear; the avatar becomes a checkbox |
| Pressed | `--bg-1-pressed` | n/a |
| Selected (single, reading pane shows it) | `--row-selected` | Same text rules |
| Selected + hover | `--row-selected-hover` | n/a |
| Multi-selected | `--row-selected` + checked checkbox | n/a |
| Keyboard focus | 2px inset focus ring (in addition to any of the above) | n/a |
| Being dragged | opacity 0.5 (estimate) | n/a |
| Pending (optimistic move or delete in flight) | opacity 0.6, then collapses the row height over 150 ms (estimate) | n/a |
| Disabled (offline, not cached) | `--bg-1` | All text `--fg-disabled` |

### 3.10 Hover quick actions

- Default set, according to a third-party Outlook on the web write-up **[3P]**: Delete, a read/unread toggle, Flag, Pin. Axentra ships this set in this order: Delete, Mark read/unread, Flag, Pin (estimate of order). The same source covers the web only, not the new Outlook for Windows.
- "Archive" is not in the default set. Users can choose it under Settings > Mail > Customize actions **[3P]**. That page has 4 drop-downs for the message list hover actions, with a preview **[3P]**.
- Options documented by that source **[3P]**: Delete, read/unread toggle, Flag, Archive, Move to folder, Pin, and "Nothing" (empty slot). Draft 1 also listed Snooze, Categorize, Report junk and Sweep; none of these is documented as a hover action (unverified). Axentra may offer them as an extension, but keep the documented list first. Maximum 4.
- Since the 2025 change, the first hover action slot temporarily shows a gear button that opens these settings; it disappears after the first click **[MS]**. Axentra: show the gear once for new installs (Axentra).
- Buttons: 28x28 subtle icon buttons, 20px icons, gap 2 (estimate). Tooltips show the label plus the shortcut, for example "Delete (Delete)".
- Clicking a hover action never changes the selection or opens the item. It applies to the hovered row only, even when other rows are multi-selected (estimate).

### 3.11 Conversation threading

- Setting **[MS]**: Settings > Mail > Layout > Message organization ("How do you want your Message List organized?"). Group by conversation, or show individual messages. The ribbon also has View > Messages > "Conversations" **[MS]**. Microsoft states no default for new installs; Axentra default: by conversation (estimate).
- Conversation identity: Graph `conversationId`; IMAP `THREAD=REFERENCES` (RFC 5256) or a local References/In-Reply-To graph (RFC 5322); JMAP `threadId` (RFC 8621).
- Header row: a chevron (16px) to the left of the avatar slot, shown on hover and on rows with more than one message (estimate). The time is the newest message's time. The row is unread if any message in the folder is unread. Status icons are the union across the messages (any attachment, any flag).
- Expanding (chevron click, or Right arrow; collapse with Left) shows child rows indented by 32 (estimate). Each child shows sender, a one-line preview and time, about 44px tall (estimate). Child rows can include messages from other folders, which show the folder name as a caption1 `--fg-3` tag ("Sent Items", "Deleted Items") (estimate).
- Clicking the header opens the whole conversation in the reading pane. Clicking a child opens the conversation scrolled to and expanded on that message (estimate).
- Only one conversation is expanded at a time; expanding another collapses the first (estimate).
- Actions on a header row (delete, move, categorize, flag) apply to all of the conversation's messages in the current folder (estimate). Delete never touches the copies in Sent Items (estimate).
- Option **[MS]**: "Do you want to see deleted items in your conversations?" Show or hide.

### 3.12 Selection, multi-select, keyboard

| Input | Result |
|---|---|
| Click | Select one item and open it in the reading pane. Marks it read according to the mark-as-read setting (3.15). |
| Ctrl+click | Toggle the item in the selection **[MS]** |
| Shift+click | Select the range from the anchor |
| Shift+Up / Shift+Down | Extend the selection **[MS]** |
| Ctrl+A | Select all loaded items (and lazily all items in the folder, see 3.14) |
| Checkbox click (hover or select mode) | Toggle, same as Ctrl+click |
| Up / Down | Move focus and selection (Axentra; standard listbox) |
| Ctrl+Up / Ctrl+Down | Move focus without changing the selection (Axentra) |
| Space | Toggle the focused item's selection in select mode (Axentra) |
| Home / Ctrl+Home | Select the first message **[MS]** |
| End | Select the last loaded message (Axentra) |
| Enter | Open in the reading pane, or the popout if the reading pane is hidden (Axentra) |
| Shift+Enter | Open in a new window **[MS]** |

- Multi-selection (≥ 2): the reading pane shows a summary card with "{n} items selected" (subtitle1), action buttons (Delete, Archive, Move, Mark as read, Flag, Categorize) and a "Deselect all" link (Axentra copy; estimate of layout).
- The "Select all" header checkbox in select mode is tri-state (none / some / all).
- When items that are selected get removed by sync, drop them from the selection silently.

### 3.13 Drag and drop

- Drag one or more selected rows. A ghost pill follows the pointer: an envelope icon and "{n} item(s)" (estimate), with shadow8.
- Drop on a folder in the folder pane (including a Favorites shortcut) to move. Hold Ctrl while dropping to copy (classic behaviour; estimate for the new Outlook). The cursor badge shows "+" for copy.
- Not valid as targets: the current folder, Notes, Conversation History, Outbox, search folders, category favorites. Exception: dropping onto a category favorite applies that category (estimate). Invalid targets show the not-allowed cursor.
- Hovering a collapsed parent for 700 ms auto-expands it (estimate). Hovering near the top or bottom 32px of the folder pane auto-scrolls it.
- Dragging outside the app window exports the messages as `.eml` files (RFC 5322) through the platform DnD protocol (Axentra; v2).
- After the drop: optimistic removal, plus a toast "Moved {n} item(s) to {folder}" with "Undo" (Axentra copy).

### 3.14 Loading, infinite scroll, virtualization

- Virtualize: render only the visible rows plus about 10 overscan rows. Rows have fixed heights per density, except the Roomy attachment row and expanded conversations; keep a height cache for those.
- Page size: 50 items per server request (estimate). Prefetch the next page when the viewport is within 15 rows of the end.
- Show a "Loading more…" row (spinner and caption1 text) at the end while fetching. On error, show "Couldn't load more messages." with "Retry" (Axentra copy).
- First load: show skeleton rows (grey bars at line positions) for up to 8 rows. Do not show skeletons for cached folders.
- New mail arriving while the user has scrolled down must not move the viewport. Show a pill "New messages ↑" at the top that scrolls up when clicked (Axentra).
- Keep the scroll position and selection per folder within a session.

### 3.15 Mark-as-read behaviour

- Default: an item becomes read when the selection moves to another item. This is the classic Outlook default **[3P]**; Microsoft's own classic page words it as marking read on selection, with the "Mark item as read when the selection changes" check box **[MS, classic]**. The default in the new Outlook and on the web is not documented (estimate).
- Settings > Mail > Message handling > "Mark as read" **[3P]**. Only "Don't automatically mark items as read" is confirmed as a label **[3P]**; the other two options are estimates: "Mark displayed items as read when the selection changes" / "Mark displayed items as read after {n} seconds". (Classic's equivalents are "Mark items as read when viewed in the Reading Pane" with "Wait _ seconds…", and "Mark item as read when the selection changes" **[MS, classic]**.)
- Ctrl+Q marks read and Ctrl+U marks unread **[MS]**.

### 3.16 Empty states (Axentra copy)

Centred vertically in the list. Line art in `--fg-4` (our own artwork, no Microsoft imagery), 96px. Title in subtitle1 `--fg-1`, body in body1 `--fg-3`, optional button.

| Context | Title | Body | Action |
|---|---|---|---|
| Inbox (no Focused) or Focused | "All clear" | "New messages will show up here." | n/a |
| Other | "Nothing in Other" | "Less important messages will be kept here." | n/a |
| Drafts | "No drafts" | "Messages you start but don't send are saved here." | n/a |
| Sent Items | "Nothing sent yet" | "Messages you send will be listed here." | n/a |
| Deleted Items | "Deleted Items is empty" | "Items you delete stay here until this folder is emptied." | n/a |
| Junk Email | "No junk email" | "Messages that look like junk are kept here for a limited time." | n/a |
| Archive | "Nothing archived" | "Archive messages to keep them without cluttering your Inbox." | n/a |
| Snoozed | "Nothing snoozed" | "Snoozed messages wait here until it's time to bring them back." | n/a |
| Any other folder | "This folder is empty" | n/a | n/a |
| Filter has no matches | "Nothing matches this filter" | n/a | "Clear filter" |
| Search has no results | "No results found" | "Try other keywords or change your filters." | "Search all folders" (only when the scope is narrower) |
| Load error | "We couldn't load this folder" | "Check your connection and try again." | "Try again" |
| Offline, not cached | "You're offline" | "This folder will load when you're back online." | n/a |

---

## 4. Message context menu (right-click in the message list)

Opens at the pointer. Shift+F10 or the Menu key opens it at the focused row. Width is fit-to-content (min 220), radius 8, shadow16, items 32px tall, 20px icons, shortcut text right-aligned in caption1 `--fg-3` (estimate of metrics). Submenus open on hover after 250 ms, or on Right arrow.

Microsoft documents several of these items being reachable by right-click: Categorize **[MS]**, the Focused/Other moves **[MS]**, Restore in Deleted Items **[MS]**, and Ignore on the web **[3P]**; Snooze, Pin and Flag are documented from the ribbon rather than the context menu. The exact order and grouping in the new Outlook are not documented; the order below is ours (estimate). Re-checked 2026-10-19: no Microsoft page confirms "Copy" (to folder) or a "Forward as attachment" submenu in the new Outlook's context menu; both stay estimates.

| # | Item | Shortcut | Submenu / notes |
|---|---|---|---|
| 1 | "Reply" | Ctrl+R | n/a |
| 2 | "Reply all" | Ctrl+Shift+R | n/a |
| 3 | "Forward" | Ctrl+F | Submenu: "Forward", "Forward as attachment" (estimate) |
| --- | | | |
| 4 | "Delete" | Delete | Shift+Delete deletes permanently after confirmation **[MS]** |
| 5 | "Archive" | E (OWA **[MS]**), Backspace (classic; estimate) | See 10.2 |
| 6 | "Move" | Ctrl+Shift+V (classic; estimate for the new Outlook) | Submenu: up to 5 recent folders, "Inbox", ---, "Move to a different folder…" (picker), "Create new folder" (estimate) |
| 7 | "Copy" | n/a | Same submenu as Move, ending in "Copy to a different folder…" (estimate) |
| --- | | | |
| 8 | "Categorize" | n/a | Submenu: recent categories with check states **[MS]**, ---, "New category" **[MS]**, "Clear all categories" **[3P]**, "Manage categories" **[MS]** (see 9.3) |
| 9 | "Mark as read" / "Mark as unread" | Ctrl+Q / Ctrl+U | Label follows the item's state; for a mixed selection show both (estimate) |
| 10 | "Flag" | Insert | Submenu (section 8): "Today", "Tomorrow", "This week", "Next week", "No date" **[MS]**, "Custom…" (Axentra extension), ---, "Mark complete", "Clear flag" (labels estimate) |
| 11 | "Pin" / "Unpin" | n/a | n/a |
| 12 | "Snooze" | n/a | Submenu (10.7): "Later today", "Tomorrow", "This weekend", "Next week" **[3P]**, ---, "Choose a date" (label unverified) |
| --- | | | |
| 13 | "Move to Other inbox" / "Move to Focused inbox" | n/a | Inbox with Focused on only **[MS]**. Microsoft describes them as reached from Move on the right-click menu of the Focused or Other tab. |
| 14 | "Always move to Other inbox" / "Always move to Focused inbox" | n/a | **[MS]**. Creates a sender override (Graph `inferenceClassificationOverride`). Classic uses "Move to Other" / "Always Move to Other" **[MS]**. |
| 15 | "Ignore" / "Stop ignoring" | n/a | Conversation-level (10.6). Present on the web **[3P]**; not confirmed in the new Outlook for Windows (unverified). |
| 16 | "Report" | n/a | Submenu: "Report phishing", "Report junk" **[3P]**; a university write-up lists Junk, Phishing and Not junk for the web **[3P]**. In Junk Email, replaced by "Not junk" (estimate of label). |
| 17 | "Block" | n/a | Submenu: "Block sender"; "Never block sender", "Never block sender's domain", "Never block this group or mailing list" (classic labels **[MS]**, where Microsoft capitalises them "Never Block Sender", "Never Block the Group or Mailing List"; the domain item and the presence of the "Never block" items in the new Outlook are unverified) |
| 18 | "Sweep…" | n/a | Hidden in Junk Email, Drafts, Sent Items, Deleted Items **[MS]** |
| 19 | "Rules" | n/a | Submenu: "Create rule…" (prefilled from sender), "Manage rules" (estimate) |
| --- | | | |
| 20 | "View" | n/a | Submenu: "View message details" **[MS]** (headers dialog; Microsoft's header article gives More actions > View > View message details for the new Outlook and the web), "View message source" (unverified: Draft 1 cited Microsoft, but the cited page documents only "View message details") |
| 21 | "Save as" | n/a | Saves an `.eml` file **[3P]** |
| 22 | "Print" | Ctrl+P (estimate) | n/a |
| 23 | "Open in new window" | Shift+Enter | **[MS]** |

Rules for the menu:

- Multi-selection: disable Reply, Reply all, Forward submenu items other than "Forward as attachment", Create rule, View, Save as (allowed only for one item), Open in new window. All other items apply to the whole selection.
- Drafts folder: show "Edit" (opens compose) at the top instead of the Reply group (estimate). Hide Report and Block.
- Sent Items: hide Report, Block, Sweep, Focused/Other.
- Deleted Items and Junk Email: add "Restore" **[MS]**. On the web it returns the item to its original folder when possible, otherwise mail goes to Inbox **[MS]** (Draft 1 tagged this [3P]). Microsoft does not state the destination for the new Outlook; use the same rule (estimate). Graph exposes no "original folder" property, so record the source folder id locally at delete time; when unknown, restore to Inbox (Axentra).
- Junk Email: add "Not junk" at the top (estimate of placement).
- Disabled items stay visible in `--fg-disabled` and are announced as disabled (not hidden), except for the context-specific hides listed above.
- Classic: classic Outlook has many more items (Quick Steps, Find Related, Follow Up > Add Reminder, Junk > Junk Email Options). Not in scope.

---

## 5. Reading pane

### 5.1 Structure (conversation view, newest on top by default)

```
+--------------------------------------------------------------------+
| Subject (subtitle1, wraps to 2 lines)        [conversation actions] |
| [category pills]                                                    |
+--------------------------------------------------------------------+
| [info banners]                                                      |
| +----------------------------------------------------------------+ |
| | [avatar 40] Sender Name <addr>      [Reply][Reply all][Fwd][...]| |
| |             To: A; B; +3                         Mon 10/14/2024 | |
| |             Cc: C                                     3:45 PM   | |
| | [attachments well]                                              | |
| | Body…                                                           | |
| | [Reply] [Reply all] [Forward]                                   | |
| +----------------------------------------------------------------+ |
| | collapsed older message row                                     | |
| | collapsed older message row                                     | |
+--------------------------------------------------------------------+
```

- Conversation ordering setting **[MS]** (older wording): "Newest on top", "Newest on bottom", or show each message separately. Newer wording asks whether to show all messages in a conversation or only the selected one **[MS]**.
- Pane padding: 20 left/right, 16 top (estimate). Each message is a card: `--bg-1`, a 1px `--stroke-3` border, radius 8, padding 16 (estimate).

### 5.2 Subject header

- Subject: subtitle1 `--fg-1`, selectable text, wraps to at most 2 lines then ellipsis, full text in a tooltip. Empty subject shows "(No subject)" in `--fg-3` (estimate of label).
- Category pills (section 9) sit under the subject. Clicking one filters by that category.
- Conversation actions, right-aligned, 32px subtle icon buttons (estimate of set):
  - "Switch background" (sun/moon). Only in dark mode. Toggles a light background for message bodies (Axentra label; estimate of feature parity).
  - "Open in new window" (`open`), Shift+Enter.
  - In Fill screen or popout mode only: "Close" (`dismiss`) or a back arrow.

### 5.3 Message header (expanded message)

| Element | Spec |
|---|---|
| Avatar | 40px (estimate). Same initials and colour rules as the list. Clicking it opens the contact card (people spec). |
| Sender line | Display name, body1Strong `--fg-1`, then `<address>` in body1 `--fg-3` (estimate). If the From and Sender headers differ, show "{Sender} on behalf of {From}". Name and address are selectable. |
| To line | "To: " in `--fg-3`, then recipient names separated by "; ". Collapsed to one line; past one line it truncates with a "+{n} others" link that expands all (estimate of label). |
| Cc line | Same as To. Hidden when there are no Cc recipients. Bcc is shown only on the user's own sent messages. |
| Timestamp | Full format "Mon 10/14/2024 3:45 PM" (ddd M/D/YYYY h:mm a, locale-aware; estimate of format). Right-aligned under the action buttons, caption1 `--fg-3`. Tooltip shows the long date. |
| Action buttons | Top-right: "Reply" (`arrow_reply`), "Reply all" (`arrow_reply_all`), "Forward" (`arrow_forward`) as 32px icon buttons with tooltips, then "More actions" (`more_horizontal`) **[MS]** (the "..." next to the reply buttons; Microsoft's header article names it). |

"More actions" menu (estimate of order): Reply, Reply all, Forward, ---, Delete, Mark as unread, Flag ▸, Categorize ▸, Pin, ---, Report ▸, Block ▸, ---, View ▸ ("View message details" **[MS]**, "View message source" (unverified)), Save as **[3P]**, Print, ---, Open in new window. The Block entry in "More actions" is documented **[3P]**. The "More actions" name and the View ▸ "View message details" path are **[MS]**.

### 5.4 Info banners (stacked between header and body)

Banner: 36px minimum height, radius 4, padding 8/12, 16px icon, body1 text, buttons as text links (estimate of metrics). Colours: info uses `--brand-bg-2` with `--fg-1`; warning uses marigold tint60 `#fefbf4` / marigold shade40 `#463100` in dark with `--fg-1`; danger uses red tint60 `#fdf6f6` / red shade40 `#3f1011` (estimate of mapping).

| Banner | Copy (Axentra wording) | Actions |
|---|---|---|
| External content blocked | "Pictures and other content from the internet weren't downloaded for this message." (Axentra copy, reworded in this fact-check so it does not track Microsoft's old OWA sentence) | "Show content" (Axentra; Draft 1's "Show blocked content" [3P] has no surviving source, unverified); "Always show content from {sender address}" (Axentra; no source shows an equivalent in the new Outlook, unverified) |
| Junk | "This message is in Junk Email. Links and attachments may be unsafe." | "It's not junk" (estimate of label) |
| Phishing / spoof suspected (server verdict) | "This message might not be from who it claims. Be careful with links and attachments." | n/a |
| Replied / forwarded | "You replied to this message on {Mon 10/14/2024 3:12 PM}." / "You forwarded…" | Clicking the date jumps to that message (estimate) |
| Flagged | "Follow up. Start by {date}. Due by {date}." / "Completed on {date}." | n/a |
| Importance | "This message was sent with High importance." / "…Low importance." | n/a |
| Read receipt requested | "{Sender} requested a read receipt." | "Send receipt", "Don't send" (only when the policy is "Ask me"; see 6.12) |
| Mailing list | "Unsubscribe" link next to the sender line (RFC 2369 `List-Unsubscribe`; prefer RFC 8058 one-click POST) | Confirmation dialog before sending |
| Meeting request | Handled by the calendar spec (RSVP bar) | n/a |

External images:

- OWA historically blocked external content in HTML mail and showed a banner to re-enable it **[MS]** (2013 archived blog). Outlook.com can load external images through an image proxy; the choice is "Always use the Outlook service to load images" / "Don't use the Outlook service to load images" under Settings > General > Privacy and data **[MS]**. Microsoft's page does not state which is the default (Draft 1 said "by default"; unverified). No current Microsoft page documents an "external content blocked" banner or a per-sender "always show" option for the new Outlook (unverified).
- Axentra has no proxy. It blocks remote resources by default for any sender not in Safe senders. Remote content loads automatically only for senders in Safe senders or senders the user chose "always show" for (Axentra). (Draft 1's sentence said it also blocks "when the user has chosen always show", which contradicts the action; fixed.)
- Remote resources include `<img src=http…>`, CSS `url()`, `background` attributes, `srcset`, and `<link>`.
- `cid:` (RFC 2392) and `data:` images are always allowed.

### 5.5 Attachments well

- Position: under the message header, above the body **[MS]**.
- Layout: a wrap row of cards, gap 8. When there are more than 6 attachments, show 6 and a "Show all {n} attachments" link (estimate).
- With 2 or more attachments, show the action "Download all" **[MS]**, which saves a `.zip`. The zip format is **[3P]**.
- Card anatomy (estimate of metrics): 220 x 52, `--bg-3`, a 1px `--stroke-2` border, radius 4.
  - 32px file-type icon, or a 40x40 thumbnail for images.
  - Name: body1 `--fg-1`, middle-ellipsis so the extension stays visible.
  - Size: caption1 `--fg-3`, binary units ("245 KB", "3.2 MB").
  - Chevron button (`chevron_down`) at the right.
- Card states: hover `--bg-1-hover` with a `--stroke-1` border; pressed `--bg-1-pressed`; focus ring.
- Single click or double-click opens it **[MS]**; depending on account type Outlook opens either its previewer or the file's app **[MS]**. Axentra: an in-app overlay for images, PDF, text and `.eml`; other types fall back to "Open".
- Chevron menu: "Preview" **[MS]**, "Download" **[MS]**, "Open" (system default app via xdg-open; the Axentra equivalent of the new Outlook's per-app items such as "Open in Word" **[MS]**), "Save as…" (Axentra), ---, "Copy" (to clipboard as a file; v2). Not built: "Save to OneDrive" **[MS]**, "Edit in Browser" **[MS]**, "Edit in desktop app" **[MS]** (new Outlook, shown only when Office is installed). "Download all" is the new Outlook and web label; "Save all" is classic only **[MS]**.
- Drag a card to the desktop or file manager to save it **[MS]**.
- Unsafe types: Outlook blocks `.bat`, `.exe`, `.vbs`, `.js` and similar **[MS]**.
  - Axentra keeps a block list that also covers Linux-relevant types: `.sh`, `.run`, `.desktop`, `.AppImage`, `.jar`, `.deb`, `.rpm` (Axentra).
  - A blocked card shows a shield icon and "Blocked for safety". It offers no download, only "Learn more" (Axentra).
- Inline images (`Content-Disposition: inline` with a Content-ID referenced in the HTML body) are not shown in the well.

### 5.6 Body rendering

- HTML bodies render in a sandboxed, script-disabled frame. Sanitize first: strip `<script>`, event handlers, forms, `<iframe>`, `<object>`, `<embed>`, `<base>`, `<meta http-equiv>`, and `javascript:` URLs.
- Links open in the system browser after a hover tooltip shows the real URL. When the visible text looks like a URL that differs from the href, show a warning on click (Axentra).
- Plain text bodies: monospace off, preserve whitespace, linkify URLs and addresses, honour `format=flowed` (RFC 3676).
- Quoted history in replies collapses behind a "…" pill button "Show message history" (estimate of label). Detect it via `<div id="divRplyFwdMsg">`, `<blockquote type=cite>`, or `>`-prefixed lines.
- Dark mode: remap the body's light background and dark text toward the dark palette. Leave images untouched. "Switch background" (5.2) turns this off per message.
- Zoom: Ctrl+wheel or Ctrl+= / Ctrl+- zoom the body only, 50 to 200% (estimate).

### 5.7 Conversation stack (collapsed messages)

- Collapsed row: 48px (estimate), `--bg-2`, radius 8. Contents: 32px avatar, sender body1Strong when unread, a one-line preview in `--fg-3`, time on the right (list time format), small attachment and flag icons.
- Click (or Enter) expands it in place. Expanding several at once is allowed.
- When more than 3 older messages are collapsed between expanded ones, merge them into a single "{n} more messages" row (Axentra; estimate of behaviour).
- On open, the messages that are expanded are the selected one and every unread one. All others start collapsed (estimate).
- Messages that live in Deleted Items appear only when "see deleted items in conversations" is on **[MS]**. They are shown dimmed (opacity 0.7) with a "Deleted Items" tag (estimate).
- Drafts in the conversation render as an inline compose (section 6) at their position.

### 5.8 Bottom reply bar

At the end of the newest expanded message card: three outline buttons, 32px tall, radius 4, 8px gap, icon and label: "Reply", "Reply all", "Forward" (estimate of styling). Clicking one opens inline compose at that position (newest on top: at the top of the stack). The reading pane scrolls the compose into view and focuses the body.

### 5.9 Reading pane empty state (Axentra copy)

Nothing selected: line art, "Select an item to read", "Nothing is selected." in `--fg-3`.

---

## 6. Compose

### 6.1 Surfaces

| Surface | When | Notes |
|---|---|---|
| Inline in reading pane | Reply / Reply all / Forward (default); new message (Ctrl+N / N) fills the reading pane (estimate) | Top-right of the compose has "Open in new window" (`open`) **[MS]**, which moves the draft into a popout without losing state |
| Popout window | "Open in new window", Shift+Enter on a draft, or the reading pane set to "Popout only" **[MS]** | Separate OS window, default 900x700 (estimate), with the full compose ribbon |
| Draft tabs | When the user navigates away from an inline compose that has content | The draft stays open as a tab in a tab strip above the reading pane: "{Subject or (No subject)}", with a close ✕ (estimate of mechanism) |

- Microsoft states that inline compose has no Insert tab; the user opens the draft in a new window ("Open in new window", near the upper right of the message) to get one **[MS]**. Re-checked 2026-10-19: still stated, for both the new Outlook and classic. Inline compose still reaches the common insert commands through the Message tab's Insert group (for example Signature) **[MS]**.
- Axentra decision: show the Insert tab in both surfaces (Axentra). This is a confirmed deviation from Outlook. If strict parity is preferred, drop Insert inline and rely on the Message tab's Insert group (6.4). See open questions.

### 6.2 Field rows (top to bottom)

Each row is 40px tall (estimate). Labels are text buttons on the left, 56px wide (estimate). Rows are separated by a 1px `--stroke-3` line.

| Row | Spec |
|---|---|
| From | Shown only when the user has more than one account or alias (estimate). A drop-down of accounts and aliases, ending with "Other email address…" (estimate). Changing From swaps the signature if it hasn't been edited (6.11). |
| To | "To" label button (opens the address book picker). Chip input (6.3). At the far right: "Cc" and "Bcc" text toggles (estimate of placement). |
| Cc | Appears when toggled on, or prefilled by Reply all. Same chip input. |
| Bcc | Hidden by default. Toggled from the To row, or with Options > "Show Bcc" (Options tab; Bcc control is on the Options tab per classic **[MS]**). |
| Subject | Input with placeholder "Add a subject" (estimate). For inline replies it is hidden and the original subject with "RE: " / "FW: " is used. A "…" button on the compose toolbar reveals it for editing (estimate). |
| Body | Rich text editor (contenteditable). Minimum 200px. Grows; the compose scrolls inside the reading pane. |

Footer bar (inline compose, bottom; popout, top-left) (estimate of placement):
`[Send ▾] [Discard]  ...  "Draft saved at 3:42 PM"`

### 6.3 Recipient chips and autocomplete

- Chip: 24px tall, radius 12 (pill), `--bg-3` background, 16px avatar plus display name in body1 (estimate). Hover shows the address and opens a contact card after 600 ms. A focused chip gets a focus ring.
  - Selected chip: `--brand-bg-2` (estimate).
  - Invalid chip: `--danger-fg` text, a dashed `--danger-fg` border, and an error icon; the tooltip says "This address isn't valid." (Axentra copy).
  - External recipient (another domain, when org policy enables it): a small `globe` icon (estimate).
- Commit typed text into a chip with Enter, Tab, `;`, `,`, or blur. Paste parses an RFC 5322 `address-list`, including `Name <addr>` and group syntax. Invalid fragments stay as invalid chips.
- Backspace in an empty input selects the last chip; a second Backspace removes it. Delete removes the selected chip. Left/Right moves between chips. Double-click a chip, or press Enter on a selected chip, to edit it as text.
- Chip context menu: "Copy", "Edit", "Remove", "Open contact card" (estimate).
- Autocomplete:
  - Sources, merged and ranked: recent recipients (local "nickname cache": frequency × recency), contacts, and the organisation directory for Exchange (GAL search after 3 characters, debounced 250 ms).
  - Suggestion rows are 44px: 24px avatar, name, address.
  - Show at most 8 suggestions; the last row is "Search directory" (Exchange only).
  - Each recent suggestion has an ✕ ("Remove from suggestions", estimate of label) that deletes it from the local cache.
  - Navigate with Up/Down, accept with Enter or Tab, close with Esc.
- Typing "@" plus a name in the body inserts a mention. The name is highlighted and the person is added to To **[MS]**. The mention is an `<a href="mailto:">` styled as a mention. Users can shorten the inserted text (for example to the first name only) **[MS]**.

### 6.4 Ribbon tabs while composing

Tabs appear in the main ribbon (inline) or the popout ribbon. A community answer on Microsoft Q&A (November 2025) lists the compose tabs as Message, Insert, Format text, Draw and Options **[3P]**; Microsoft confirms a Message tab with an Insert group **[MS]**, Insert in the popout **[MS]** and Options > Tracking **[MS]**. Tab set and order: "Message", "Insert" (popout only in Outlook; see 6.1), "Format text", "Draw" (v2), "Options" **[3P]** for the set, estimate for the order. A March 2026 comment on the same thread reports a build showing only File, Home, View and Help while composing, so the compose tabs may vary by build.

**Format text tab** (control order is an estimate):

| Group | Controls (label: behaviour) |
|---|---|
| Font | "Font" combo box (default Aptos or fallback, 1.2); "Font size" combo box (8, 9, 10, 10.5, 11, 12, 14, 16, 18, 20, 24, 28, 36, 48, 72; default 12); "Increase font size" / "Decrease font size" |
| Style | "Bold" Ctrl+B **[MS]**, "Italic" Ctrl+I **[MS]**, "Underline" Ctrl+U **[MS]**, "Strikethrough", "Subscript", "Superscript" |
| Colour | "Highlight" (split: last colour + palette, "No color"); "Font color" (split: last colour + palette, "Automatic"); "Clear formatting" |
| Paragraph | "Bullets" (split: bullet styles); "Numbering" (split: number styles); "Decrease indent"; "Increase indent"; "Align left", "Center", "Align right", "Justify"; "Line spacing" (1.0, 1.15, 1.5, 2.0); "Left-to-right" / "Right-to-left" |
| Styles | "Styles" gallery: "Normal", "Heading 1", "Heading 2", "Heading 3", "Quote", "Code" (estimate of names) |
| Links | "Link" Ctrl+K **[MS]** (dialog: "Display as", "Web address", "Insert"/"Cancel") |
| Format | "Format painter" (v2); "HTML" / "Plain text" toggle (formats: HTML is the default; plain text has no bold, italic or colour **[MS]**) |

The colour palette is a 10x6 grid of standard colours plus "More colors…" (estimate).

**Insert tab** (estimate of order):

| Control | Behaviour |
|---|---|
| "Attach file" (split) | Main: file chooser (XDG portal). Drop-down: up to 10 "Recent files", "Browse this computer" (estimate of labels) |
| "Link" | Same as Ctrl+K |
| "Pictures" | Inserts inline images (`cid:` multipart/related). Limit and downscale prompt over 5 MB (estimate) |
| "Table" | Grid picker up to 10x8, then "Insert table…" (estimate) |
| "Emoji" | Emoji picker flyout with search, recent, categories |
| "Signature" (split) | Lists the account's signatures; "Signatures…" opens settings **[MS]**. Microsoft documents this control in the Insert group of the Message tab **[MS]**; repeating it on the Insert tab is our choice (estimate). |
| "Horizontal line" | Inserts `<hr>` |
| "Date & time" | Inserts the current date in the locale format (estimate) |

**Options tab:**

| Control | Behaviour | Source |
|---|---|---|
| "Show From" / "Show Bcc" | Toggles the rows | classic [MS] (Bcc on Options) |
| "High importance" / "Low importance" | Mutually exclusive toggles. Set the `Importance: high/low` header (RFC 2156) and `X-Priority` 1/5 for compatibility. Toggle icons: red `important`, blue `arrow_down`. | estimate of labels |
| Tracking: "Request a Delivery Receipt", "Request a Read Receipt" | Checkboxes. When they are not visible, Microsoft says to look under "More Options" at the right end of the ribbon [MS]; Axentra collapses them there when the ribbon is narrow (estimate). DSN (RFC 3461) via SMTP `NOTIFY=SUCCESS` when supported, and `Disposition-Notification-To` (RFC 8098 MDN). Graph: `isDeliveryReceiptRequested` / `isReadReceiptRequested` on the message. Outlook does not offer read receipts for IMAP or POP accounts [MS]; Axentra may still offer them over SMTP (Axentra). | [MS] |
| "Save draft" | Forces a save | [MS] (Options > Save draft) |
| "Schedule send" | Same as the Send drop-down | estimate |

**Message tab** (inline default): Bold/Italic/Underline, Bullets, an Insert group with "Attach file", "Link" and "Signature" (the Insert group and its Signature control are **[MS]**; the other members are estimates), "High importance", "Discard" (estimate). This is a curated subset of the other tabs.

### 6.5 Send split button and Discard

- "Send": primary button, `--brand-bg`, white text, 32px tall, radius 4, min width 72 (estimate). Hover, pressed and disabled use the tokens in 1.1. Disabled (`--bg-1-selected` with `--fg-disabled`) only while the draft is being created on the server for the first time (estimate). An empty To field does not disable Send; it is validated on click (6.8).
- The drop-down half (`chevron_down`, 24px wide, separated by a 1px divider in `--fg-on-brand` at 30% alpha) opens a menu with "Send" and "Schedule send" **[MS]**. Alt+Down opens it **[MS]**.
- Ctrl+Enter sends **[MS]**.
- "Discard": a subtle button with the `delete` icon and the label "Discard" (estimate). Behaviour:
  - If the draft was never saved and has no content, close it immediately.
  - Otherwise show the dialog "Discard this draft?" with "Discard" (primary, danger) and "Cancel" (Axentra copy).
  - Discarding deletes the server draft. Microsoft does not document whether a discarded draft goes to Deleted Items (unverified). Recommendation (Axentra): move it to Deleted Items so it stays recoverable, using Graph `POST /messages/{id}/move` with `destinationId: "deleteditems"` or IMAP `UID MOVE` to the `\Trash` folder. Do not use Graph `DELETE` for this: the delete API does not say where the item goes, and `permanentDelete` purges it beyond the user's reach **[MS]**. A draft that was never saved to the server is simply dropped.
- Esc closes the compose **[MS]**. If there is content, the draft is kept (saved) and moves to a draft tab; Esc never discards (Axentra).

### 6.6 Schedule send

- Path: the drop-down next to Send > "Schedule send" **[MS]**, then pick a default option or a custom time, then "Send" **[MS]**.
- Dialog "Schedule send" (estimate of title). Microsoft's article does not name the preset options. Presets, computed in the user's time zone:
  - "Tomorrow morning" at 8:00 AM (unverified: Draft 1 cited a third-party page for this, but that page covers Undo send, not Schedule send).
  - "Tomorrow afternoon" at 1:00 PM (estimate).
  - "Monday morning" at 8:00 AM, shown only when today is Thursday through Sunday (estimate).
  - "Custom time": date picker plus a time combo box in 30-minute steps. Free typing is allowed (estimate).
  - Each preset shows its resolved date and time in caption1 on the right.
- Buttons: "Send" (primary), "Cancel".
- The scheduled message stays in Drafts until it is sent **[MS]**. Its list row shows "Scheduled: {date time}" (estimate). To cancel, open it from Drafts and choose "Cancel send", which turns it back into a normal draft that can be sent at once **[MS]**.
- Graph backend: there is no first-class "schedule send" property on the Graph message resource. The usual approach is to set the MAPI deferred-send-time extended property (`PidTagDeferredSendTime`, `SystemTime 0x3FEF`) on the draft before calling `send` (unverified: community practice, not a Microsoft Graph article). Test against Exchange Online and Outlook.com before relying on it; if it fails, fall back to the local queue below.
- Not available for IMAP or POP accounts in the new Outlook **[MS]**. Axentra:
  - IMAP accounts can schedule, but the client holds the message in a local queue.
  - Delivery needs the app to be running, or the background service (Axentra).
  - When the app was not running at the due time, send on next launch and tell the user in a toast.

### 6.7 Autosave and "Draft saved at"

- Microsoft's "Find draft email messages" article, which applies to the new Outlook and the web, says drafts are saved automatically every three minutes **[MS]**. The 30-second figure in Draft 1 comes from a non-Microsoft source **[3P]**. Either way, Axentra saves much more often (below).
- Axentra: save 2 s after the last edit (debounce), at least every 30 s while there are unsaved changes, and immediately on blur, tab switch, window close, From change, attachment add, or before Send (Axentra).
- Status text at the right of the footer, caption1 `--fg-3`:
  - "Saving…" while a save is in flight.
  - "Draft saved at {h:mm AM/PM}" after a save (the label format is an estimate).
  - "Couldn't save draft" in `--danger-fg`, with "Retry" (Axentra copy).
- Drafts go to the Drafts folder **[MS]**. The Drafts count updates immediately.
- Offline: save to the local store and mark it for sync. Show "Saved on this device" (Axentra copy).
- Conflict (the same draft edited elsewhere): keep both; the incoming version becomes a new draft "(Conflict) {subject}" (Axentra).

### 6.8 Validation on Send (in this order)

| Check | UI (Axentra copy unless tagged) |
|---|---|
| No recipients in To/Cc/Bcc | Inline error under the To row: "Add at least one recipient." Focus moves to To. No dialog. |
| An invalid recipient | Inline error: "Fix or remove the highlighted addresses." The first invalid chip gets focus. |
| Empty subject | Dialog. Title: "Send without a subject?" Body: "This message doesn't have a subject." Buttons: "Send" (primary) / "Don't send". The task brief cites the title; Outlook's exact wording is unverified (estimate). |
| Missing attachment | Outlook reminds the user when the body mentions words like "attached" or "attachment" but nothing is attached **[MS, classic]** (Microsoft's page applies to Outlook for Microsoft 365, 2024 and 2021); a community answer says the new Outlook also matches phrases such as "see file" and "included" **[3P]**. Dialog title: "Did you mean to attach a file?" Body: "You mentioned an attachment, but nothing is attached." Buttons: "Send anyway" / "Attach a file" / "Cancel" (Axentra copy). Detection: a localized keyword list matched against new text only (not the quoted history or the signature). It can be turned off in the new Outlook under Settings > Mail > Attachments by clearing "Warn me when I send a message that may be missing an attachment" **[3P]**. |
| Size over the server limit | Banner in the compose: "This message is too large to send ({size}). The limit for this account is {limit}." Use SMTP SIZE (RFC 1870) or the server's maximum; default 25 MB (estimate). |
| Offline | Not an error. The message goes to Outbox with the toast "You're offline. Your message will be sent when you reconnect." |

### 6.9 Undo send

- Setting path: Settings > Mail > Compose and reply > "Undo send" **[3P]** (Microsoft's own recall article shortens it to Settings > Mail > Compose, then choose a time under "Undo send" and Save **[MS]**).
- After the user sends, an "Undo" prompt appears at the bottom of the message list for the length of time chosen in that setting **[MS]**.
- Duration, re-checked 2026-10-19: Microsoft's recall article says Outlook.com / Outlook on the web can hold a sent message for at most 30 seconds **[MS]**, and lists Undo send for the new Outlook for Windows too. The older 0/5/10 s slider in Draft 1 came from a dated university page **[3P]** and is superseded. Outlook for Mac allows 5 to 120 s in 5 s steps **[MS]** (not relevant to parity). The step values on the web are not documented; one university page lists 5, 10, 15 or 30 s **[3P]**.
- Axentra: options 0 (off), 5, 10, 15, 20 and 30 s; 30 s is the documented maximum. Default 5 s (Axentra). Outlook's default is undocumented; an older third-party page says the web slider starts at 0 (off) **[3P]**.
- Toast: bottom-centre of the message list, 40px tall, `--fg-1` on `--bg-1`, shadow16, radius 8. Content: a spinner, "Sending…", and an "Undo" button, with a thin progress bar draining over the window (Axentra copy and visuals).
  - Undo reopens the draft exactly as it was, with focus in the body.
  - Quit during the window (recommendation, Axentra; Microsoft documents nothing here): the user already pressed Send, so do not hold the message. On a normal quit, flush the pending-send queue immediately and keep the process alive until submission finishes (show "Sending 1 message…" in the quit path, with a "Quit anyway" escape that leaves it queued). If the process dies (crash, logout, power loss), send it on the next start and show a toast "A message you sent before Axentra closed was sent now." with a "View" link. Holding it as a draft is the safer alternative but loses mail the user believes is sent; that trade-off still needs product sign-off (open question 3).
- Classic: classic Outlook for Windows has no undo send. It offers "Do not deliver before" per message (the message waits in Outbox), or a rule to "defer delivery by a number of minutes", up to 120 minutes **[MS]**.

### 6.10 Reply, Reply all, Forward composition rules

- Reply: To = the Reply-To header if present, otherwise From. Reply all: To = Reply-To/From plus the original To minus the user's own addresses; Cc = the original Cc minus the user's own. Bcc is never carried over.
- Subject prefixes: "RE: " and "FW: " (en-US; Outlook's own prefixes). Do not stack prefixes: "RE: RE:" collapses to one "RE: ".
- Set `In-Reply-To` and `References` (RFC 5322). Forward keeps the attachments; Reply drops them, except inline images.
- Quoted header block (en-US): "From: …", "Sent: {full date}", "To: …", "Cc: …", "Subject: …", above the original body, separated by a horizontal rule. This is Outlook's convention (estimate of the exact fields).
- "Forward as attachment" attaches the original as `message/rfc822`.
- The cursor starts at the top of the body, above the signature.

### 6.11 Signatures

- Settings > Accounts > Signatures **[MS]**. The user picks the account, then "Add signature", names it, edits it in a rich editor, and saves with "Save" **[MS]**. Edit an existing one with the pencil next to it **[MS]**.
- Per account defaults: Microsoft's current article describes check boxes under the editor that apply the signature to new messages, to replies and forwards, or to neither **[MS]**. Draft 1's "For new messages" / "For replies/forwards" drop-downs are classic Outlook labels (classic uses "New messages" / "Replies/forwards") and are not confirmed for the new Outlook (unverified). Axentra: two check boxes under the editor, "Use for new messages" and "Use for replies and forwards" (Axentra copy); checking one on a signature clears it on the account's other signatures.
- Insert one manually: Message tab > Insert group > "Signature", then pick a saved signature; "Signatures..." at the end of that drop-down opens settings **[MS]**. Draft 1 said Insert > Signature.
- Wrap the inserted signature in `<div data-axentra-signature="{id}">` so it can be swapped when From changes, unless the user has edited inside it.
- Replies put the signature above the quoted history.
- Classic: classic Outlook keeps signatures locally under File > Options. The new Outlook and the web need signatures recreated **[MS]**; with a Microsoft account, a signature must be created in both Outlook and Outlook on the web **[MS]**. Graph exposes no signature API, so Axentra stores signatures locally per account and cannot import Outlook's (Axentra; Graph limitation to our knowledge).

### 6.12 Read receipts (incoming)

- Policy: Settings > Mail > Message handling > Read receipts **[MS]**. Microsoft's article does not name the options; labels are estimates: "Ask me before sending a response", "Always send a response", "Never send a response". Default: "Ask me…" (estimate).
- There is no way to force a recipient to send a receipt **[MS]**. Axentra must never send an MDN without user consent or policy.
- After answering (or declining), set the IMAP `$MDNSent` keyword (RFC 3503, registered in RFC 5788) so other clients do not ask again. Graph has no documented MDN-sent flag; record it locally.

---

## 7. Search

### 7.1 Search box

- Location **[REF]**: in the 48px app header, 32px tall, radius 4, 14px text, placeholder "Search"; its left edge lines up with the message list's left edge (x = 266 at a 1440px window) and it is 356px wide there. Draft 1's "centred, about 480px wide" is superseded. Background `--bg-1` on the canvas (estimate). Leading `search` icon.
- Focus: Alt+Q (Outlook on the web **[MS]**). The new Outlook for Windows has no documented search-focus shortcut; Ctrl+E is the classic binding (estimate for the new Outlook). Esc cancels the search **[MS]**.
- Scope: a drop-down at the left inside the box, shown once the box is active. "All folders" is the default on Outlook.com **[MS]**; the other labels are estimates: "Current folder", "{Current folder name}" if it is not Inbox, and "Inbox".
  - With multiple accounts, add "All accounts" and per-account entries (estimate).
  - The new Outlook default scope is set in Settings > General > Search **[MS]**, which can change it to the current folder **[MS]**.
- Filters: a `filter` / `options` icon button at the right inside the box opens the filter panel **[MS]** (the "filter button on the right side of the search box").

### 7.2 Suggestions drop-down

- Opens on focus, under the box, at box width, with shadow16 (estimate of metrics).
- Sections (estimate of grouping): recent searches (clock icon) (unverified; the Outlook.com search article does not mention recent searches), suggested people (avatar) **[MS]** (they appear below the box as you type; selecting one shows related mail), and keyword or subject suggestions as the user types.
- Typing `from:` and a few letters suggests matching contacts **[MS]**.
- Up/Down moves through the suggestions; Enter runs the search; Delete on a recent search removes it (Axentra).
- Selecting a person runs `from:"{address}"`.

### 7.3 Filters panel

Fields documented for OWA **[MS]** (from a search-result excerpt; the cited Microsoft page now returns 404, so this list is no longer checkable): "Folders", "From", "To", "Subject", "Keywords", "Date from", "Attachments". Axentra adds "Date to" next to "Date from" (estimate that Outlook has it). Microsoft's current Outlook.com article only says to use the filter in the search box to limit the search to folders **[MS]**.

| Field | Control |
|---|---|
| "Folders" | Drop-down: "All folders", then the folder tree |
| "From" | People picker (single chip) |
| "To" | People picker |
| "Subject" | Text |
| "Keywords" | Text (body and subject) |
| "Date from" / "Date to" | Date pickers |
| "Attachments" | Checkbox "Has attachments" (estimate of label) |
| Buttons | "Search" (primary), "Clear filters" (estimate of labels) |

Filter fields are written back into the query text as keywords (7.4), so the box always shows the effective query.

### 7.4 Keyword syntax

Documented operators **[MS]**, in Microsoft's Outlook search article:

| Keyword | Example | Meaning |
|---|---|---|
| `from:` | `from:"bobby moore"` | Sender name or address |
| `to:` | `to:alex@contoso.com` | To recipient |
| `cc:` / `bcc:` | `cc:bobby` | Cc / Bcc recipient |
| `subject:` | `subject:"budget review"` | Subject contains the phrase |
| `hasattachment:` | `hasattachment:yes` (or `true`) | Has attachments. Some guides also write `hasattachments:` **[3P]**; Axentra accepts both. |
| `category:` | `category:red` | Category name contains the text |
| `received:` | `received:1/15/2026`, `received:"this week"`, `received:today`, `received:"last month"`, ranges with `>` / `<` | Received date. Microsoft notes that day and month names are no longer supported, but attaches that note to classic Outlook for Windows (build 16.0.14314 and later) **[MS, classic]**; the new Outlook's behaviour is unverified, so Axentra accepts them. |
| `sent:` | `sent:yesterday` | Sent date. Listed among Outlook.com's search keywords **[MS]**. |
| `participants:` | `participants:bobby` | Anyone in From, To, Cc or Bcc **[MS]** (Outlook.com keyword list; meaning estimate) |
| `body:` / `content:` | `body:invoice` | Body text only **[MS]** (Outlook.com keyword list) |
| `read:` | `read:no` (or `read:false`) | Unread items |
| `hasflag:` / `followupflag:` / `isflagged:` | `hasflag:true` | Flagged items (`isflagged:` is in the Outlook.com list **[MS]**) |
| `due:` | `due:"last week"` | Flagged items whose due date falls in the range **[MS]** |
| `messagesize:` | `messagesize:>5 MB`, `messagesize:<10 KB` | Size comparison. The named sizes (tiny…enormous) are not supported in the new Outlook **[MS]**. |
| `AND` / `OR` / `NOT` | `budget AND q3` | Must be uppercase **[MS]** |
| `"…"` | `"exact phrase"` | Exact match |

Matching rules **[MS]** (Microsoft's article covers all Outlook versions without separating the new Outlook):

- Prefix matching: "ray" finds "Raymond" and "rays" but not "tray" or "disarray".
- Numbers joined to words count as part of the word, so "365" does not find "Microsoft365".
- Case-insensitive; punctuation is ignored.
- A partial number shorter than 5 digits does not match inside a longer number (searching 9810 does not find 98101). Draft 1 said "strings shorter than 5 digits are ignored", which overstated it.
- Only the first 1,000 results are returned.
- Searches that mix languages may give inconsistent results.

Backends: map to Graph `$search` (KQL) for Microsoft 365, or to IMAP `SEARCH` (RFC 9051) and a local full-text index for IMAP. A keyword the server doesn't support falls back to filtering the local index. Graph caution (verify against the Graph `$search` documentation before coding): a message `$search` request cannot be combined with `$filter` or `$orderby`, so sort and filter the returned page locally.

### 7.5 Results list

- The message list switches to results mode. The title becomes "Results" (estimate). A back arrow in the search box exits search, and so does selecting any folder in the folder pane **[MS]** (Outlook.com); its tooltip "Exit search" is an estimate. Esc or the back arrow restores the previous folder, selection and scroll position.
- "Top results" section: the most relevant matches (Axentra: up to 3), followed by every result sorted by date. Microsoft's Top results article covers classic Outlook for Windows (it is turned off under File > Options > Search) **[MS, classic]**; its behaviour in the new Outlook is assumed (estimate). Per that article: results in Top results are not repeated below (Microsoft says this may change), the section is omitted when no result stands out, and it is not available for local searches, third-party mailboxes or Exchange on-premises accounts **[MS, classic]**.
- Each result row uses the normal anatomy plus a caption1 `--fg-3` folder tag ("Inbox", "Sent Items") on line 3 (estimate).
- Search terms are highlighted in subject and preview with `--highlight-search`, and in the reading pane body (estimate).
- Results are flat by default, not threaded (estimate). The sort and Filter menu stay available.
- No results: the empty state in 3.16.

---

## 8. Flags (follow-up)

### 8.1 Options and date rules

Flag menu items for the new Outlook **[MS]** (Home > Flag / Unflag): "Today", "Tomorrow", "This week", "Next week", "No date". Re-checked 2026-10-19: Microsoft's article does not list "Custom", "Mark complete" or "Clear flag" for the new Outlook, and no source in this spec supports Draft 1's claim that "Custom" exists there (unverified). Classic Outlook documents "Custom" and "Add Reminder" **[MS]**. Axentra keeps "Custom…" as an Axentra extension (Graph accepts any start/due pair), and keeps "Mark complete" / "Clear flag" because the Insert key and Graph `flagStatus` need them anyway.

| Option | Start date | Due date | Source |
|---|---|---|---|
| "Today" | today | today | [MS] |
| "Tomorrow" | today + 1 | today + 1 | [MS] |
| "This week" | today + 2, but not later than the last workday of this week | last workday of this week | [MS] |
| "Next week" | first workday of next week | last workday of next week | classic [MS]; the new Outlook text only says "next week" |
| "No date" | none | none | [MS] |
| "Custom…" | user-picked | user-picked (must be ≥ start) | (Axentra extension; unverified for Outlook). Dialog: "Start date", "Due date", optional "Reminder" (date + time), buttons "Save" / "Cancel" (estimate of labels). Graph `followupFlag` has no reminder field; store the reminder locally or use the message's extended reminder properties (estimate). |
| "Mark complete" | n/a | n/a | Sets `flagStatus = complete` and `completedDateTime = now` (labels estimate) |
| "Clear flag" | n/a | n/a | Sets `flagStatus = notFlagged` (labels estimate) |

- Workdays come from the calendar work-week setting (calendar spec). Default Mon to Fri.
- Graph mapping **[MS]**: `followupFlag { flagStatus: notFlagged | complete | flagged, startDateTime, dueDateTime, completedDateTime }`. Setting `dueDateTime` requires `startDateTime` **[MS]**.
- IMAP mapping: `\Flagged` keyword; dates and completion are stored locally (keyed by Message-ID).
- Quick flag (hover action, Insert key, flag icon click) uses "Today" (classic default "Quick Click"; estimate for the new Outlook). On a flagged item, the Insert key marks it complete **[MS]**.

### 8.2 Visuals

| State | List icon | Reading pane banner |
|---|---|---|
| Not flagged | none; outline `flag` on hover | none |
| Flagged | Filled `flag`, `--flag-fg` | "Follow up. Start by {ddd M/D/YYYY}. Due by {ddd M/D/YYYY}." (no-date variant: "Follow up.") |
| Overdue | Filled `flag`, `--flag-fg`. The due date in the banner is shown in `--danger-fg` (estimate). | as above |
| Complete | `checkmark`, `--complete-fg` | "Completed on {date}." |

Flagged items also feed the To Do / tasks surfaces (tasks spec).

---

## 9. Categories

### 9.1 Defaults

The default categories have generic colour names; Microsoft's article writes them "Red Category" and "Blue Category", with a capital C **[MS]** (Draft 1 had lower-case "category"). The full default set is the six classic categories (estimate of the set for the new Outlook). For Microsoft accounts the names come from the server's master list (`GET /me/outlook/masterCategories`), so these seeds matter only for IMAP accounts:

| Default name | Graph preset | Light pill bg / text | Dark pill bg / text |
|---|---|---|---|
| "Blue Category" | preset7 (Blue) **[MS]** | `#a9d3f2` / `#004377` | `#004377` / `#a9d3f2` |
| "Green Category" | preset4 (Green) | `#9fd89f` / `#094509` | `#094509` / `#9fd89f` |
| "Orange Category" | preset1 (Orange) | `#f4bfab` / `#7a2101` | `#7a2101` / `#f4bfab` |
| "Purple Category" | preset8 (Purple) | `#c6b1de` / `#341a51` | `#341a51` / `#c6b1de` |
| "Red Category" | preset0 (Red) | `#f1bbbc` / `#751d1f` | `#751d1f` / `#f1bbbc` |
| "Yellow Category" | preset3 (Yellow) | `#fef7b2` / `#817400` | `#817400` / `#fef7b2` |

Pill colours use Fluent palette tint40/shade30 (light) and shade30/tint40 (dark), the same pattern Fluent uses for colourful persona tokens **[FL]**. Every hex value in 9.1 and 9.2 was re-checked against `colors.ts` on 2026-10-19 and is the Fluent palette value it claims to be. The mapping of Outlook preset names to Fluent palette colours is ours (estimate). The real Outlook hex values are not documented; Graph says the rendered colour depends on the client, and its preset-to-name table is for the Outlook desktop client **[MS]**.

### 9.2 Full preset palette (for "New category" and "Edit")

Graph defines 25 presets plus None **[MS]**. Up to 25 colours can be mapped, and names must be unique **[MS]**.

| Preset | Name **[MS]** | Fluent source (estimate) | Light bg / text | Dark bg / text |
|---|---|---|---|---|
| 0 | Red | red | `#f1bbbc` / `#751d1f` | `#751d1f` / `#f1bbbc` |
| 1 | Orange | darkOrange | `#f4bfab` / `#7a2101` | `#7a2101` / `#f4bfab` |
| 2 | Brown | brown | `#ddc3b0` / `#50301a` | `#50301a` / `#ddc3b0` |
| 3 | Yellow | yellow | `#fef7b2` / `#817400` | `#817400` / `#fef7b2` |
| 4 | Green | green | `#9fd89f` / `#094509` | `#094509` / `#9fd89f` |
| 5 | Teal | teal | `#9bd9db` / `#02494c` | `#02494c` / `#9bd9db` |
| 6 | Olive | forest | `#bdd99b` / `#294903` | `#294903` / `#bdd99b` |
| 7 | Blue | blue | `#a9d3f2` / `#004377` | `#004377` / `#a9d3f2` |
| 8 | Purple | purple | `#c6b1de` / `#341a51` | `#341a51` / `#c6b1de` |
| 9 | Cranberry | cranberry | `#eeacb2` / `#6e0811` | `#6e0811` / `#eeacb2` |
| 10 | Steel | steel | `#94c8d4` / `#00333f` | `#00333f` / `#94c8d4` |
| 11 | DarkSteel | anchor | `#bcc3c7` / `#202427` | `#202427` / `#bcc3c7` |
| 12 | Gray | platinum | `#cdd6d8` / `#3b4447` | `#3b4447` / `#cdd6d8` |
| 13 | DarkGray | platinum shade10 | `#5f6d71` / `#ffffff` | `#5f6d71` / `#ffffff` |
| 14 | Black | grey[14] | `#242424` / `#ffffff` | `#424242` / `#ffffff` |
| 15 | DarkRed | red shade20 | `#9f282b` / `#ffffff` | same |
| 16 | DarkOrange | darkOrange shade20 | `#a62d01` / `#ffffff` | same |
| 17 | DarkBrown | brown shade10 | `#804d29` / `#ffffff` | same |
| 18 | DarkYellow | marigold shade20 | `#b27c00` / `#ffffff` | same |
| 19 | DarkGreen | darkGreen | `#0b6a0b` / `#ffffff` | same |
| 20 | DarkTeal | teal shade10 | `#037679` / `#ffffff` | same |
| 21 | DarkOlive | forest shade10 | `#427505` / `#ffffff` | same |
| 22 | DarkBlue | royalBlue | `#004e8c` / `#ffffff` | same |
| 23 | DarkPurple | purple shade10 | `#532982` / `#ffffff` | same |
| 24 | DarkCranberry | cranberry shade10 | `#b10e1c` / `#ffffff` | same |
| None | No colour | neutral | `#ebebeb` / `#424242` | `#383838` / `#d6d6d6` |

- Draft 1's DarkSteel row broke the stated tint40/shade30 rule (text `#242424` was grey[14], and the dark background `#394146` was anchor primary). It now uses anchor tint40 `#bcc3c7` and anchor shade30 `#202427` like every other light-pill row.
- Rows 13 to 24 use solid dark fills with white text, so they do not follow the tint40/shade30 rule; that is intended (estimate).
- **Conflict with `calendar.md` 2.4.** The calendar spec maps the same presets to different base colours for event blocks. Examples: Preset1 Orange is orange `#F7630C` there but darkOrange here; Preset3 Yellow is marigold `#EAA300` there but the yellow ramp here; Preset13 DarkGray `#5D5A58` vs `#5f6d71`; Preset15 DarkRed `#750B1C` vs `#9f282b`; Preset16 DarkOrange `#CA5010` vs `#a62d01`; Preset17 DarkBrown `#6C4123` vs `#804d29`; Preset18 DarkYellow `#C19C00` vs `#b27c00`; Preset20 DarkTeal `#026467` vs `#037679`; Preset21 DarkOlive `#986F0B` vs `#427505`. A category must look the same in mail and calendar. Design must pick one base colour per preset, store it in one shared table, and derive the mail pill tints and calendar fills from it. Until then, neither table is authoritative.

### 9.3 Behaviour

- Categorize menu: the most recently used categories with checkmarks **[MS]**, then "New category" **[MS]**, "Clear all categories" **[3P]** (Draft 1 had "Clear categories" tagged [MS]; Microsoft's article does not mention a clear command, and a third-party guide gives "Clear all categories"), "Manage categories" **[MS]**. Clicking a checked category removes just that one **[3P]**. An item can have several categories **[MS]**. The menu is reached by right-clicking a message or from the ribbon **[MS]**.
- New category dialog: "Name" text field, a colour swatch grid (the 25 presets as 24px circles with a check on the selected one), "Save" / "Cancel" (Save **[MS]**; the grid is an estimate). Settings also offers "Create category" **[3P]**. Name rules: required, unique ignoring case, at most 255 characters (estimate). Graph requires `displayName` to be unique in the master list **[MS]**.
- Manage categories (Settings > Accounts > Categories **[MS]**): a list of rows with a swatch, the name, a "favorite" star (adds it to Favorites **[3P]**), Edit (pencil; rename or recolour, then Save) **[MS]**, and Delete (trash) **[3P]**.
  - Delete asks for confirmation: "Delete category \"{name}\"? It will be removed from all items." (Axentra copy).
  - Deleting removes the category from every item **[3P]**.
- Graph `displayName` is read-only after creation **[MS]**. Renaming therefore means creating the new name, re-tagging items, and deleting the old one. Do this as a background job with progress. Warn that it is not atomic.
- IMAP: categories map to IMAP keywords (RFC 9051 flag-keyword). Encode names to the keyword charset and keep the display names in a local master list.

---

## 10. Other mail actions

### 10.1 Sweep

- Not available in Junk Email, Drafts, Sent Items or Deleted Items **[MS]**.
- Dialog title "Sweep messages" (estimate). It opens for the selected message's sender, with radio options, in our paraphrase of the observed labels **[3P]**:
  1. "Move all messages from the {folder} folder"
  2. "Move all messages from the {folder} folder and any future messages"
  3. "Always keep the latest message and move the rest"
  4. "Always move messages older than 10 days"
- "Move to" folder picker, default "Deleted Items" **[3P]**. It includes "New folder" **[3P]**.
- Buttons: "Sweep" (primary) **[MS]**, "Cancel" (estimate).
- Options 2 to 4 create a standing sweep rule. Sweep rules run once a day, unlike inbox rules, which run on arrival **[MS]**. Options 2 to 4 need a confirmation line in the dialog: "This creates a rule that runs once a day." (Axentra copy).
- Existing sweep rules are listed (and can be deleted) under Settings > Mail > Sweep **[3P]**. Axentra needs the same page (settings spec).
- Backend: no documented Microsoft Graph API creates or lists Outlook's server-side sweep rules (to our knowledge). Axentra therefore runs every sweep rule on the client, for all account types, daily and on launch (Axentra). The one-time option 1 is a plain bulk move. Sweep rules made in Outlook will not appear in Axentra and vice versa; say so in the dialog's help text.

### 10.2 Archive

- One-click "Archive" moves the item to the account's Archive folder **[MS]** (Graph `archive` **[MS]**, IMAP `\Archive` RFC 6154). This is not the Exchange online-archive mailbox **[MS]**.
- If no Archive folder exists, show the dialog "Set up Archive" (Axentra copy). It offers "Create Archive folder" (default) or "Choose an existing folder…" and remembers the choice.
- To restore, use "Move to" from Archive **[MS]**.
- The toast "Archived {n} item(s)" offers "Undo" (Axentra).
- Shortcut: E (OWA **[MS]**). Classic uses Backspace (estimate).

### 10.3 Junk and Report

- "Report" ▸ "Report phishing" / "Report junk" **[3P]**; on the web a university write-up lists Junk, Phishing and Not junk **[3P]**. Draft 1 said clicking the Report button itself defaults to phishing in some tenants; the cited page does not say so (unverified). Axentra: the main button opens the menu, it never reports directly.
- A confirmation dialog appears (unverified; the cited page does not mention one). Axentra copy: "Report as junk?" / "Report as phishing?", with "Report" / "Cancel". Include a checkbox "Also block {sender}" (Axentra). Draft 1 cited a Microsoft blog for consumer accounts offering block or unsubscribe while reporting; that page no longer has readable content (unverified).
- After reporting, the message moves to Junk Email (junk) or Deleted Items (phishing; estimate).
- In Junk Email: "Not junk" moves the message back to Inbox and offers to add the sender to Safe senders (estimate of the flow).
- Junk retention: Microsoft pages disagree, and the split is by product: the Outlook.com junk page says 14 days and not recoverable afterwards **[MS]**; the Outlook on the web junk-filter page and the recover-deleted-items page say 30 days **[MS]**. Axentra shows the server's policy where it is known and never deletes junk on the client.
- Settings > Mail > Junk email: "Blocked senders and domains" with "Add blocked sender", and "Safe senders and domains" with "Add safe sender"; enter an address or domain, then OK and Save **[MS]**. (A second Microsoft page describes separate "Blocked senders" and "Blocked domains" boxes with "Add"; the UI may differ between consumer and work accounts.) The lists hold up to 10,000 entries **[MS]**. Graph exposes no documented API for these lists (to our knowledge); for Microsoft accounts Axentra links out to Outlook on the web settings, and for IMAP keeps local lists that drive a client-side filter (Axentra).
- Server actions:
  - Graph: `POST /me/messages/{id}/reportMessage` with `ReportAction` = `junk`, `notJunk` or `phish` and `IsMessageMoveRequested` **[MS]**. It is beta-only today, and `protocols.md` bans `/beta` in shipped code; Microsoft targets v1.0 parity for "Report Message" in Q4 2026 **[MS]**. Until then Axentra only moves the message (to Junk Email for junk, to Deleted Items for phishing, back to Inbox for not junk) and tells the user the report did not reach Microsoft's filters. The older beta `markAsJunk` (which also added the sender to Blocked senders) is deprecated and stopped returning data on 2025-12-30 **[MS]**. Do not use it.
  - EWS: do not build on it. Microsoft starts disabling EWS in Exchange Online in October 2026 and finishes in April 2027 **[MS]**.
  - IMAP: set `$Junk` / `$NotJunk` (and `$Phishing` for phishing reports) and move the message. These keywords are listed in RFC 9051 section 2.3.2, which says servers should support them; they are not among RFC 5788's initial registrations (Draft 1 credited RFC 5788). Only set them when the folder's `PERMANENTFLAGS` includes `\*` or the keyword itself.

### 10.4 Block sender

- "Block" ▸ "Block sender". Confirmation dialog (Axentra copy): "Block {name}?" with body "Messages from {address} will go to Junk Email." Buttons: "Block" (primary) / "Cancel".
- Draft 1 said Outlook.com deletes the selected messages and blocks future ones **[MS]**; the current Outlook.com page only says that mail from a blocked address or domain is moved to Junk Email, and is silent on messages already received (unverified). Classic Outlook with Exchange sends blocked senders' future mail to Junk Email (or quarantine, if the admin set it up) **[MS]**. Axentra moves future mail to Junk Email and leaves existing messages where they are unless the user also deletes them (Axentra), and states the effect in the dialog body.
- "Never block sender" / "Never block sender's domain" / "Never block this group or mailing list" add to Safe senders. These are classic labels **[MS]**; the domain item is an estimate; their presence in the new Outlook is unverified.
- Domain blocking: the new Outlook and the web support it in Settings > Mail > Junk email **[MS]**. Draft 1's "domain blocking may be desktop-only [3P]" is wrong for current Outlook and is removed.
- The new Outlook does not support blocking senders on third-party accounts (Gmail, Yahoo, iCloud and similar) **[MS]**. Axentra can still block on IMAP accounts with its local filter (Axentra), which is a deliberate extension.

### 10.5 Pin

- Pin keeps a message at the top of the list **[MS]** (Home > Tags > Pin), under a "Pinned" header, when the list is sorted by date **[3P]**.
- Pinned state is per folder: a pinned message stays on top only in the folder where it was pinned **[3P]**. Several pinned messages are ordered by when they were pinned, not by date **[3P]**.
- Exchange stores pin state server-side, but no documented Graph property exposes it (to our knowledge). Axentra keeps pin state locally for all account types, keyed by account + message id (Axentra), so pins made in Outlook will not show in Axentra and vice versa. Revisit if Graph adds it.
- Classic: classic Outlook for Windows has no pin **[3P]**.

### 10.6 Ignore conversation

- Availability: documented for classic Outlook **[MS]** and for Outlook on the web (right-click > "Ignore") **[3P]**. Not confirmed for the new Outlook for Windows (unverified). Axentra ships it.
- "Ignore" moves the conversation to Deleted Items, and future messages in the conversation go there too **[MS, classic]** / **[3P, OWA]**. On the web, existing messages are removed from every folder except Sent Items **[3P]**. Senders are not blocked **[3P]**. With Exchange 2010 and later, matching is by conversation id, not subject **[MS, classic]**.
- In classic Outlook the ignore state expires after 14 days with no activity on the thread **[MS, classic]**. Axentra applies the same 14-day expiry to its local ignore list (Axentra).
- Confirmation dialog (Axentra copy): "Ignore this conversation?" Body: "This conversation and any new messages in it will be moved to Deleted Items." Buttons: "Ignore" / "Cancel", plus a "Don't show this again" check box (Outlook on the web has an equivalent option **[3P]**).
- To undo, select the conversation in Deleted Items and choose Ignore again; classic then offers "Stop Ignoring Conversation" and moves the messages back to their original folder **[MS, classic]**. Axentra label: "Stop ignoring" (Axentra).
- Backend: Graph has no conversation-action API (to our knowledge), and EWS's conversation actions are going away with EWS (October 2026 to April 2027) **[MS]**. Axentra implements Ignore on the client for all account types: a local list of ignored conversation ids (Graph `conversationId`, or the IMAP thread root Message-ID), checked whenever new mail is synced (Axentra). It only works while Axentra is running.

### 10.7 Snooze

- Available in the new Outlook and the web, not in classic **[MS]**.
- Snoozed messages move to the Snoozed folder **[MS]** and return to the top of Inbox as unread at the chosen time **[3P]** (Microsoft does not say).
- Path: select the message (on the web, open it), then Home > Snooze **[MS]**. Microsoft's article does not name the presets.
- Presets **[3P]** (one third-party guide), with times partly from that guide and partly estimates:

| Option | Time |
|---|---|
| "Later today" | a few hours later **[3P]**. Axentra: now + 3 h, rounded up to the next half hour; hidden after 6 PM (estimate) |
| "Tomorrow" | 8:00 AM tomorrow **[3P]** |
| "This weekend" | Saturday morning **[3P]**; Axentra uses 8:00 AM and hides it on Saturday and Sunday (estimate) |
| "Next week" | Monday 8:00 AM **[3P]** |
| "Choose a date" (label unverified; the third-party guide says "Pick a date", Microsoft gives no label) | date picker plus time; the time field steps in 30 minutes but any time can be typed **[MS]** |

- Hiding rules: no source documents when "Later today" or "This weekend" disappear; ours are estimates.
- Backend: Outlook does not offer Snooze for IMAP or POP accounts **[3P]**, and no documented Graph API snoozes a message (to our knowledge); moving a message into the `scheduled` folder does not schedule its return. Axentra therefore implements snooze on the client for all account types: move the message to the account's Snoozed folder (Graph: the `scheduled` well-known folder when present, otherwise a folder Axentra creates; IMAP: a folder Axentra creates), store the due time locally, and move it back to Inbox and mark it unread when due. This needs the app or the background service running (Axentra). Snoozes set in Outlook will still return on Outlook's schedule; snoozes set in Axentra will not return if only Outlook is running.

### 10.8 Rules entry points

"Create rule…" opens the rules editor (rules spec), prefilled with "From {sender}" and "Move to folder" **[MS]** (Create rule from the More options icon **[MS]**).

---

## 11. Keyboard shortcuts (mail)

New Outlook bindings are **[MS]** unless tagged. The OWA column lists the documented web alternates, for users who pick the "web" key scheme (Axentra setting; v2).

| Action | New Outlook | OWA | Notes |
|---|---|---|---|
| New message | Ctrl+N | N | |
| Reply | Ctrl+R | Ctrl+R or R | |
| Reply all | Ctrl+Shift+R | Ctrl+Shift+R or Shift+R | |
| Forward | Ctrl+F | Ctrl+Shift+F or Shift+F | Ctrl+F is not Find |
| Send | Ctrl+Enter | Ctrl+Enter | |
| Open in new window | Shift+Enter | Shift+Enter | |
| Delete | Delete | Delete | |
| Permanently delete | Shift+Delete | Shift+Delete | Confirmation dialog |
| Archive | (not documented) | E | Axentra: E in the web scheme only; Backspace (classic binding; estimate for the new Outlook) |
| Undo | Ctrl+Z | n/a | Undoes the last list action (toast Undo) |
| Mark as read | Ctrl+Q | Ctrl+Q or Q | |
| Mark as unread | Ctrl+U | Ctrl+U or U | Ctrl+U = Underline while in the compose body |
| Flag / mark complete | Insert | Insert | |
| Next / previous message | Ctrl+> / Ctrl+< | n/a | Ctrl+Shift+. / Ctrl+Shift+, |
| Extend selection | Shift+Down / Shift+Up | same | |
| Select first message | Home or Ctrl+Home | same | |
| Select all | Ctrl+A | n/a | [MS, classic]; not in the new Outlook's mail table |
| Go to Mail | Ctrl+1 | Ctrl+Shift+1 | New Outlook also: Ctrl+2 Calendar, Ctrl+3 Copilot, Ctrl+4 People **[MS]** (shell spec) |
| Go to folder pane | Ctrl+Y | n/a | Microsoft's new Outlook table also lists Ctrl+Y as "repeat the most recent action". Axentra: folder pane in the list, redo/repeat inside the compose body. |
| Jump to folder | type letters in the folder pane | n/a | |
| Expand / collapse folder | Right / Left | Right / Left | |
| Search | not documented for the new Outlook; Axentra binds Alt+Q and Ctrl+E (estimate) | Alt+Q | |
| Cancel search | Esc | Esc | |
| Close compose | Esc | Esc | Never discards (6.5) |
| Open split button menu | Alt+Down | Alt+Down | Send ▾, Forward ▾ |
| Show shortcuts | ? | Shift+? | Opens the shortcut sheet |
| Show key tips | Alt | n/a | Ribbon key tips (ribbon spec) |
| Insert link | Ctrl+K | Ctrl+K | |
| Bold / Italic / Underline | Ctrl+B / Ctrl+I / Ctrl+U | same | |
| Paste as plain text | Ctrl+Shift+V | n/a | (unverified: the Q&A thread Draft 1 cited does not mention it; it is the Chromium/WebView default). In the compose body only. |
| Move to folder | Ctrl+Shift+V | n/a | Classic; in the message list only (estimate) |
| Reading pane: top / bottom / page | Home / End / PgUp / PgDn | same | |
| Cycle panes | F6 / Shift+F6 | n/a | (Axentra) |
| Context menu | Shift+F10 / Menu | n/a | (Axentra) |

Shortcuts can't be customised in the new Outlook, but the whole set can be turned off **[3P]**. Axentra offers "Keyboard shortcuts: On / Off" (Axentra).

Shortcut dispatch must be context-scoped: compose body > list > global. The Ctrl+U, Ctrl+Y and Ctrl+Shift+V conflicts above depend on this.

---

## 12. Settings surfaced by this area (labels)

| Path | Setting | Values |
|---|---|---|
| Settings > Mail > Message list (Microsoft's Focused Inbox article; another Microsoft page says Layout) | Focused Inbox | "Sort messages into Focused and Other" on/off, per account [MS] |
| Settings > Mail > Layout | Message organization | by conversation / individual messages [MS] |
| Settings > Mail > Layout | Reading pane conversation | all messages / selected only; newest on top / bottom [MS] |
| Settings > Mail > Layout | Deleted items in conversations | show / hide [MS] |
| Settings > Mail > Layout | Reading pane | "Right", "Bottom", "Fill screen", "Popout only" [MS] |
| Settings > Mail > Layout | After moving or deleting | "Open the next item", "Open the previous item", "Return to the message list" [MS] |
| Settings > Mail > Layout | Message list format | "Sender name first", "Subject first" [MS] |
| Settings > Mail > Layout | Date header | show / "Don't show date header in message list" [MS, Q&A] |
| View tab > Density | Density | "Roomy", "Cozy", "Compact" [MS] |
| Settings > Mail > Layout | Sender image / preview text / attachment preview | Axentra-only toggles (Axentra). Outlook has no separate toggles; Roomy is the only density that shows sender images and attachment previews [MS] |
| Settings > Mail > Message handling | Mark as read | see 3.15 [3P] |
| Settings > Mail > Message handling | Read receipts | see 6.12 [MS] |
| Settings > Mail > Message handling | Empty Deleted Items on exit | a check box that empties Deleted Items at sign-out [MS]; the label "Empty my deleted items folder" is an estimate (Axentra: on app quit) |
| Settings > Mail > Compose and reply | Message format, default font, size | HTML / plain text; font; size [MS]. Microsoft gives the new Outlook path as Settings > Mail > Compose (and reply) > Message format; the default font it names is Calibri [MS] (see 1.2) |
| Settings > Mail > Compose and reply | Undo send | 0 to 30 s; 30 s is the documented maximum [MS] (6.9) |
| Settings > Mail > Attachments | Attachment reminder | "Warn me when I send a message that may be missing an attachment" on/off [3P] |
| Settings > Mail > Customize actions | Message list hover actions | 4 slots (3.10) [3P] |
| Settings > Mail > Sweep | Sweep rules | list with delete (10.1) [3P] |
| Settings > Mail > Junk email | Blocked / Safe senders and domains | lists [MS] |
| Settings > Accounts > Signatures | Signatures, defaults | 6.11 [MS] |
| Settings > Accounts > Categories | Categories | 9.3 [MS] |
| Settings > General > Search | Search scope | 7.1 [MS] |
| Settings > General > Appearance | Theme, dark mode | (shell spec) [MS] |

---

## 13. Accessibility

- Folder pane: `role="tree"`, nodes `role="treeitem"` with `aria-expanded`, `aria-level`, `aria-selected`. The count is in the accessible name, for example "Inbox, 12 unread". Favorites and each account are separate tree groups with `aria-label`.
- Message list: `role="listbox"` with `aria-multiselectable="true"`, or `role="treegrid"` when conversations are expanded. Each row's accessible name: "{Unread, }{Flagged, }{Pinned, }{sender}, {subject}, {time}, {preview}". Group headers use `role="presentation"` and are announced as part of the first row ("Today").
- Hover actions must also be reachable from the keyboard. Tab from a focused row enters its action buttons (roving tabindex); Esc returns to the row.
- Reading pane: each message card is `role="article"` with `aria-label="{sender}, {time}"`. Banners use `role="status"`, and `role="alert"` for phishing.
- Compose: the recipient input is a combobox (`aria-autocomplete="list"`). Chips are buttons with names like "Alex Wu, alex@contoso.com, press Delete to remove".
- Live regions announce "Moved 3 items to Archive", "Message sent", "Draft saved".
- Respect high contrast (`forced-colors`): Fluent Nav uses a 2px outline indicator in forced colours **[FL]**. Repeat that for selected rows and the unread bar, which otherwise disappear.
- Minimum target size is 24x24. Hover action buttons are 28x28.
- Never rely on colour alone. Unread uses bold plus the bar; flag uses an icon; categories always show text in the pill.

---

## 14. Implementation notes (must not miss)

1. **Identify folders by role, never by display name.** Use Graph well-known names or IMAP SPECIAL-USE (RFC 6154). Display names are localised server-side. Do not use EWS (distinguished folder ids or anything else): Microsoft begins disabling EWS in Exchange Online in October 2026 and completes it in April 2027 **[MS]**, and `protocols.md` forbids EWS outright (on-premises Exchange is reached over IMAP only, if at all).
2. **Drafts count is total items. Every other folder shows unread.** Use `totalItemCount` vs `unreadItemCount` (Graph). Hide zero counts.
3. **Default folders cannot be renamed, deleted or moved.** Enforce this in the UI and in the command layer, not only by hiding menu items, so shortcuts and DnD can't bypass it.
4. **Shortcut conflicts are context-dependent.** Ctrl+U (unread vs underline), Ctrl+Y (folder pane vs redo), Ctrl+F (forward, not find), Ctrl+Shift+V (plain paste vs move). Build a scoped key dispatcher (compose > list > global) before wiring any shortcut.
5. **Esc in compose must never lose data.** Save, then close to a draft tab. Discard always goes through the explicit button and dialog.
6. **Remote content is blocked by default.** Axentra has no image proxy, unlike Outlook.com. Sanitize HTML in Rust before it reaches the webview; never rely on CSP alone. Strip tracking pixels even after the user chooses "show". Allow `cid:` and `data:` only.
7. **Run the HTML body in an isolated, script-disabled context.** Dioxus desktop uses a webview. Render bodies in a sandboxed iframe (`sandbox` without `allow-scripts`, `allow-same-origin` off) or a separate webview. Message HTML must never share the app's DOM or JS context.
8. **Virtualize the message list from day one.** Keep fixed row heights per density plus a height cache for the variable rows (Roomy attachments, expanded conversations). A folder with 50,000 items must scroll at 60 fps.
9. **Optimistic actions need undo and reconciliation.** Move, delete, archive, flag, pin, categorize, mark read: update local state immediately, queue the server op, revert with a toast on failure. Every destructive list action offers "Undo" for about 5 s.
10. **Undo send is a client-side delay.** Hold the message in a local "pending send" queue for N seconds before SMTP or Graph submission. Decide what happens on quit during the window (see open questions). Classic Outlook has no undo send.
11. **Scheduled send and snooze need a server, or a running client.** Exchange/Outlook.com run both server-side for Outlook, but Graph documents no snooze API and only an extended-property route for deferred send (6.6, 10.7). So snooze is client-side for every account type, scheduled send is client-side for IMAP and possibly for Graph, and both need a background agent (systemd user service or autostart). Surface this limitation in the UI; Outlook does not offer either for IMAP/POP accounts.
12. **Flag date math uses the work week.** "This week" starts at today+2 capped at the last workday; "Next week" spans the next work week. Graph rejects `dueDateTime` without `startDateTime`.
13. **Category rename is not atomic on Graph.** `displayName` is read-only after creation, so a rename means create, re-tag, delete. Run it as a resumable job.
14. **Category colours: our hex values are estimates.** Graph presets are names only and render differently per client. Keep preset → token mapping in one table shared by mail and calendar (9.2 currently disagrees with `calendar.md` 2.4) so design can retune it in one place.
15. **Time labels are locale- and timezone-aware, and change at midnight.** Use ICU (icu4x) skeletons, not hand-built strings. Re-bucket the date group headers on the day change and on timezone change.
16. **Conversation actions scope to the current folder.** Deleting a conversation from Inbox must not delete the user's own replies in Sent Items.
17. **Pin, snooze, sweep, ignore and favorites are local state on every account type; categories are local on IMAP.** Graph has no documented API for pin, snooze, sweep rules, ignore or folder favorites, so these do not sync with Outlook even for Microsoft accounts. Key the state by account plus Graph id (or Message-ID for IMAP). Expect it not to roam to other clients, and make that visible in settings help text.
18. **The attachment reminder checks new text only.** Exclude the quoted history, the signature and forwarded headers, or users will see false positives (a known complaint about the new Outlook **[3P]**).
19. **Recipient parsing follows RFC 5322.** Handle quoted display names with commas, group syntax, IDN domains (punycode on the wire, Unicode in chips) and SMTPUTF8 local parts.
20. **Linux font reality.** Segoe UI and Aptos aren't available. Bundle Selawik (metric-compatible with Segoe UI) per `reference-measurements.md`, validate truncation and row heights with it and with Noto Sans, and in compose use metric-compatible fallbacks (Carlito for Calibri).
21. **Dark mode for message bodies is a transform, not a theme.** Keep an original-colours toggle per message ("Switch background"). Never transform inside images, and keep the colours of user-authored highlight text.
22. **Hover-only UI needs keyboard and touch equivalents.** Checkboxes, hover actions and the folder "More options" button must also appear on focus, and on long-press for touchscreens.
23. **Search keywords: accept the variant spellings** (`hasattachment:` and `hasattachments:`). Fall back to the local index for anything the backend can't evaluate, and cap results at 1,000 to match Outlook.
24. **Tag every estimate in code comments.** Constants in `ui/tokens.rs` that come from "(estimate)" or "(unverified)" rows in this spec carry a `// ESTIMATE` comment so design QA can find and tune them. Constants from **[REF]** rows carry `// MEASURED` and must change only when `reference-measurements.md` changes.
25. **"Delete" on Graph means move to Deleted Items.** For a normal delete, call `POST /messages/{id}/move` with `destinationId: "deleteditems"`. Graph's `DELETE /messages/{id}` page does not say where the item goes (unverified: it is widely reported to behave as a soft delete that skips Deleted Items), and `POST .../permanentDelete` moves the item to the Purges folder, where Outlook clients can no longer reach it **[MS]**. Use `permanentDelete` only for Shift+Delete and "Empty folder", after confirmation.
26. **Graph gaps that block one-call features.** No single "mark folder as read" call yet (Microsoft target: Q4 2026) **[MS]**; `reportMessage` is beta-only (GA target Q4 2026) **[MS]**; `mentionsPreview` (the "@ Mentions me" filter and the @ icon) is beta-only. `protocols.md` bans `/beta` in shipped code, so ship the fallbacks (batched PATCH; plain move to Junk Email or Deleted Items; a local mention check that looks for the user's own address in `mailto:` links in the body) and put the v1.0 calls behind a capability flag to switch on when they reach v1.0.
27. **IMAP keywords need permission.** Categories, `$Junk`/`$NotJunk`/`$Phishing`, `$Forwarded` and `$MDNSent` are IMAP keywords. Check the folder's `PERMANENTFLAGS` (it must contain `\*` or the keyword) before storing them; otherwise keep them locally. Replied/forwarded icons (3.6) come from `\Answered` and `$Forwarded`.
28. **IMAP capabilities used by mail features** are specified in `protocols.md` (IDLE, CONDSTORE/QRESYNC, UIDPLUS, MOVE, SPECIAL-USE, ESEARCH and others). Mail UI code must ask the account layer whether a capability is present and must not issue commands directly.

---

## 15. Open questions

Status after the 2026-10-19 fact-check is given in brackets.

1. Default Favorites: Microsoft documents both "Inbox, Deleted Items" (new Outlook) and "Inbox, Sent Items, Deleted Items" (OWA). Spec picks the second. [Re-verified: both pages still say this; it is a per-product difference. Product decision: keep the web set, or drop Sent Items for strict new-Outlook parity. 2.3.]
2. Exact default-folder order in the current new Outlook: does Junk Email sit right below Inbox? [Partly answered: the OWA article lists Inbox, Junk Email, Drafts, Sent Items, Deleted Items in that sequence but does not call it the display order. Unconfirmed for work accounts in the new Outlook. Per-provider override added. 2.5.]
3. Undo send range and default. [Range answered: Microsoft now documents a 30 s maximum for the web and lists Undo send for the new Outlook; Axentra's 0 to 30 s is correct. Default still undocumented. Quit behaviour: recommendation written in 6.9 (flush on quit, send on next start after a crash); needs product sign-off.]
4. Inline compose Insert tab. [Confirmed: Microsoft still says there is no Insert tab inline in the new Outlook. Showing it inline is a deliberate deviation; the parity alternative is in 6.1.]
5. Discarding a server-saved draft. [Microsoft does not document it. Recommendation in 6.5: move to Deleted Items via Graph move / IMAP MOVE; never Graph DELETE or permanentDelete.]
6. "Custom…" flag option. [Not supported by any source found; Microsoft's new-Outlook flag list has no Custom. Kept as an Axentra extension, tagged so. 8.1.]
7. Default density and default conversation grouping for new installs. [Still open: Microsoft states no defaults.]
8. Snooze preset times and hiding rules. [Partly answered from one third-party guide: Tomorrow 8:00 AM, This weekend Saturday morning, Next week Monday 8:00 AM; the custom item's label is "Choose a date" or "Pick a date" (unresolved). Hiding rules still undocumented. 10.7.]
9. External-content banner copy and "Always show content from sender". [Still open: no current Microsoft page documents either for the new Outlook. Banner copy rewritten as original Axentra text. 5.4.]
10. "Copy" (to folder) and "Forward as attachment" submenu in the new Outlook context menu. [Still open: no source found.]
11. Is "Ignore" available in the new Outlook for Windows? [Still open: documented for classic (Microsoft) and the web (third party) only.]
12. Quick-flag default (Today vs No date) in the new Outlook. [Still open.]
13. Empty-subject prompt wording. [Still open: our dialog text is original Axentra copy; nothing documents Outlook's wording for the new Outlook.]
14. Category colours. [Hex values verified as genuine Fluent palette values; DarkSteel row fixed. New conflict found with `calendar.md` 2.4 that design must resolve. 9.2.]
15. Pane widths and row heights vs `reference-measurements.md`. [Done for folder pane width (216), folder rows (36), list header (about 44, 20px title, Select / Jump to / Filter / By Date), columns-layout rows (37), unread bar (3px), search box position and size, and surface treatment. Not measurable from the capture: message list width with the reading pane on the right, three-line row heights, reading pane metrics.]
16. Graph has no documented API for pin, snooze, sweep rules, ignore, favorites, signatures, or the junk/safe sender lists, and `reportMessage` / `mentionsPreview` are beta-only. [New. The spec now makes all of these client-side; product should accept that they will not sync with Outlook.]

---

## Sources

- https://support.microsoft.com/en-US/Outlook/mail/use-favorites-in-outlook
- https://mc.merill.net/message/MC926191 (Message Center MC926191: Pin the Favorites section)
- https://support.microsoft.com/en-US/outlook/working-with-message-folders-in-outlook-on-the-web
- https://support.microsoft.com/en-US/Outlook/mail/create-a-folder-or-subfolder-in-outlook
- https://learn.microsoft.com/en-us/graph/api/resources/mailfolder
- https://learn.microsoft.com/en-us/graph/api/resources/outlookcategory
- https://learn.microsoft.com/en-us/graph/api/resources/followupflag
- https://support.microsoft.com/en-us/office/flag-your-messages-for-follow-up-d9d69c74-b644-4fe8-af8c-43eec4afd8b8
- https://support.microsoft.com/en-US/Outlook/mail/sort-email-messages-in-outlook
- https://support.microsoft.com/en-US/Outlook/mail/change-how-the-message-list-is-displayed-in-outlook
- https://support.microsoft.com/en-us/Outlook/change-the-look-of-your-mailbox-in-outlook-com-and-outlook-on-the-web
- https://support.microsoft.com/help/f714d94d-9e63-4217-9ccb-6cb2986aa1b2 (Focused Inbox for Outlook)
- https://support.microsoft.com/en-US/Outlook/group-or-ungroup-messages-in-your-inbox
- https://learn.microsoft.com/en-us/answers/a/12770262 (Q&A: new Outlook date headers incl. "This week"; Date header setting)
- https://mc.merill.net/message/MC1041465 (Message Center MC1041465: hover actions in Sender/From column)
- https://msoutlook.info/?p=1157 (Customize actions / default hover actions, third-party; now redirects to https://robert365.com/?p=1157)
- https://support.microsoft.com/topic/19eee6b9-09a1-4db6-b5d6-37644190884f (Organize your inbox with Archive, Sweep and other tools)
- https://www.howtogeek.com/662263/use-the-built-in-sweep-feature-in-outlook-online-to-clear-unwanted-emails/
- https://www.wisestamp.com/blog/outlook-sweep/
- https://support.microsoft.com/en-au/outlook/mail/create-and-assign-color-categories-in-outlook
- https://www.ablebits.com/office-addins-blog/color-categories-new-outlook/
- https://support.microsoft.com/en-us/article/6c5aa4d4-147a-4a0b-831b-eafc041e6028 (Snooze email)
- https://usecarly.com/blog/how-to-snooze-emails-in-outlook/
- https://support.microsoft.com/en-us/office/keyboard-shortcuts-for-outlook-3cdeb221-7ae5-4c1d-8c1d-9e63216c1efd
- https://learn.microsoft.com/en-us/answers/a/12333546 (new Outlook right-click changes; community list of compose tabs. Note: it does not mention Ctrl+Shift+V, which Draft 1 attributed to it)
- https://support.microsoft.com/office/d824d1e9-a255-4c8a-8553-276fb895a8da (How to search in Outlook: operators, prefix matching, limits)
- https://support.microsoft.com/en-US/Outlook/top-search-results-in-outlook
- https://support.microsoft.com/en-US/Outlook/troubleshooting-outlook-search-issues (new Outlook search scope setting)
- https://support.microsoft.com/en-us/office/find-email-contacts-and-events-in-outlook-on-the-web-189276e4-3cc6-411f-b49b-b43ffd31fb9f (OWA search filters; via search excerpt. Returns 404 as of 2026-10-19)
- https://support.microsoft.com/en-US/Outlook/find-all-messages-with-attachments-in-outlook
- https://support.microsoft.com/en-US/Outlook/mail/use-mentions-to-get-someone-s-attention-in-outlook
- https://support.microsoft.com/topic/92f87f3f-1085-425a-87f6-08d43c19b43d (Open, save, and edit attachments)
- https://usecarly.com/blog/how-to-save-attachments-in-outlook/
- https://learn.microsoft.com/en-us/archive/blogs/catastrophic_failure_joannav/how-to-turn-off-blocked-feature-in-owa-outlook-web-app
- https://support.microsoft.com/en-us/outlook/external-image-protection-in-outlook-com
- https://kb.wisc.edu/wiscel/34567 (view message details / headers in OWA, third-party)
- https://support.microsoft.com/office/cd039382-dc6e-4264-ac74-c048563d212c (Draft 1 cited this for View message source; the short link returns 404. The full URL below is the message-headers article, which documents More actions > View > View message details, not View message source)
- https://support.microsoft.com/en-us/office/view-internet-message-headers-in-outlook-cd039382-dc6e-4264-ac74-c048563d212c
- https://support.microsoft.com/en-gb/office/where-to-find-the-insert-tab-69b2151a-5bc4-47ca-8e42-613d206ef470
- https://support.microsoft.com/en-us/office/delay-or-schedule-sending-email-messages-026af69f-c287-490a-a72f-6c65793744ba
- https://support.microsoft.com/en-US/Outlook/undo-send-in-outlook-for-mac
- https://bowdoin.teamdynamix.com/TDClient/1814/Portal/KB/Article/170909/Undo-Send-or-Add-a-Short-Delay-to-Outgoing-Email-in-Outlook
- https://sandmat.freshservice.com/support/solutions/articles/51000063976 (titled Undo Send; no readable body as of 2026-10-19, and it does not support the Schedule send presets Draft 1 attributed to it)
- https://support.d11.org/kb/article/71-recalling-an-email-from-outlook-online/
- https://support.microsoft.com/en-US/Outlook/7-cool-things-you-didn-t-know-about-outlook (attachment reminder)
- https://learn.microsoft.com/en-us/answers/a/9488172 (new Outlook attachment reminder setting, community)
- https://support.microsoft.com/office/add-and-request-read-receipts-and-delivery-notifications-in-outlook-a34bf70a-4c2c-4461-b2a1-12e4a7a92141
- https://support.microsoft.com/office/create-add-or-change-an-email-signature-in-outlook-8ee5d4f4-68fd-464a-a1c1-0e1c80bb27f2
- https://support.microsoft.com/en-us/topic/20f72414-2c42-4b53-9654-d07a92b9294a (default font, new Outlook)
- https://support.microsoft.com/de-DE/Outlook/change-the-message-format-to-html-rich-text-format-or-plain-text-in-outlook
- https://kb.siue.edu/153122 (default font Calibri to Aptos, third-party)
- https://support.microsoft.com/en-au/office/find-draft-email-messages-fcc5a4bf-efef-417d-83fc-577924d9bc81
- https://support.microsoft.com/en-US/Outlook/mark-a-message-as-read-or-unread-in-outlook
- https://robert365.com/article/250 (mark as read settings, third-party)
- https://support.microsoft.com/en-au/office/block-senders-or-mark-email-as-junk-in-outlook-com-a3ece97b-82f8-4a5e-9ac3-e92fa6427ae4
- https://support.microsoft.com/en-US/Outlook/filter-junk-email-and-spam-in-outlook
- https://support.microsoft.com/en-gb/outlook/block-a-mail-sender-in-outlook
- https://www.godaddy.com/help/block-a-sender-in-outlook-42228
- https://insider.microsoft365.com/en-us/blog/improvements-to-tackle-spam-in-outlook (redirects to techcommunity.microsoft.com; only the title was readable on 2026-10-19, so claims citing it are now unverified)
- https://services.duq.edu/TDClient/30/Portal/KB/ArticleDet?ID=396 (Report button behaviour, third-party)
- https://support.microsoft.com/kb/2697913 (Ignore conversation, classic; now redirects to https://learn.microsoft.com/troubleshoot/outlook/email-management/message-automatically-moved-deleted-items-folder)
- https://www.teachucomp.com/?p=12834 (Ignore in OWA, third-party)
- https://mailmeteor.com/blog/how-to-pin-emails-in-outlook
- https://support.microsoft.com/topic/c3d8fc15-eeef-4f1c-81df-e27964b7edd4 (Recover deleted items)
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/alias/lightColor.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/alias/darkColor.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/colors.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/brandColors.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/fonts.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/spacings.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/borderRadius.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/utils/shadows.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/alias/lightColorPalette.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/alias/darkColorPalette.ts
- https://github.com/microsoft/fluentui/blob/master/packages/react-components/react-nav/library/src/components/sharedNavStyles.styles.ts
- https://github.com/microsoft/fluentui/blob/master/packages/react-components/react-avatar/library/src/components/Avatar/Avatar.types.ts
- https://fluent2.microsoft.design/components/web/react/core/avatar/usage
- https://github.com/microsoft/fluentui-system-icons (icon set, MIT)
- RFC 5322 Internet Message Format: https://www.rfc-editor.org/rfc/rfc5322
- RFC 6154 IMAP LIST Extension for Special-Use Mailboxes: https://www.rfc-editor.org/rfc/rfc6154
- RFC 5256 IMAP SORT and THREAD: https://www.rfc-editor.org/rfc/rfc5256
- RFC 9051 IMAP4rev2: https://www.rfc-editor.org/rfc/rfc9051
- RFC 8621 JMAP for Mail: https://www.rfc-editor.org/rfc/rfc8621
- RFC 5788 IMAP4 keyword registry: https://www.rfc-editor.org/rfc/rfc5788
- RFC 2392 Content-ID and Message-ID URLs: https://www.rfc-editor.org/rfc/rfc2392
- RFC 2369 / RFC 8058 List-Unsubscribe and one-click: https://www.rfc-editor.org/rfc/rfc2369 , https://www.rfc-editor.org/rfc/rfc8058
- RFC 3461 SMTP DSN: https://www.rfc-editor.org/rfc/rfc3461
- RFC 8098 Message Disposition Notification: https://www.rfc-editor.org/rfc/rfc8098
- RFC 1870 SMTP SIZE: https://www.rfc-editor.org/rfc/rfc1870
- RFC 2156 (Importance header, MIXER): https://www.rfc-editor.org/rfc/rfc2156
- RFC 3676 format=flowed: https://www.rfc-editor.org/rfc/rfc3676

Added in the 2026-10-19 fact-check:

- `docs/spec/reference-measurements.md` (measured Outlook on the web layout, October 2026; the **[REF]** tag)
- https://support.microsoft.com/en-us/office/recall-or-replace-an-email-message-that-you-sent-35027f88-d655-4554-b4f8-6c0729a723a0 (Undo send: 30 s maximum on the web, Undo prompt at the bottom of the message list)
- https://support.microsoft.com/en-us/office/search-mail-and-people-on-outlook-com-88108edf-028e-4306-b87e-7400bbb40aa7 (Outlook.com search: All folders default, AQS keyword list incl. participants/body/sent/isflagged, exit search)
- https://support.microsoft.com/en-au/office/create-and-assign-color-categories-in-outlook-a1fde97e-15e1-4179-a1a0-8a91ef89b8dc (category names "Red Category" etc., Categorize menu, Settings > Accounts > Categories)
- https://learn.microsoft.com/troubleshoot/outlook/email-management/message-automatically-moved-deleted-items-folder (Ignore in classic: conversation id matching, 14-day expiry, Stop Ignoring Conversation)
- https://robert365.com/?p=1157 (hover actions: defaults, Customize actions, 4 slots, available actions; third-party)
- https://robert365.com/article/250 (Mark as read setting in the new Outlook and web; third-party)
- https://learn.microsoft.com/en-us/answers/a/9488172 (attachment reminder label in the new Outlook; community)
- https://support.d11.org/kb/article/71-recalling-an-email-from-outlook-online/ (older 0/5/10 s undo-send slider; third-party)
- https://learn.microsoft.com/en-us/graph/api/message-delete (Graph DELETE; destination not stated)
- https://learn.microsoft.com/en-us/graph/api/message-permanentdelete (permanentDelete moves to Purges)
- https://learn.microsoft.com/en-us/graph/api/message-reportmessage?view=graph-rest-beta (reportMessage, beta)
- https://learn.microsoft.com/en-us/graph/api/message-markasjunk?view=graph-rest-beta (markAsJunk, deprecated, stopped 2025-12-30)
- https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/deprecation-of-ews-exchange-online (EWS disablement October 2026 to April 2027; Graph parity roadmap incl. Notes, Report Message, Mark All Items As Read)
- https://github.com/microsoft/fluentui/blob/master/packages/react-components/react-nav/library/src/components/sharedNavStyles.styles.ts (re-read: nav item hover/pressed tokens, indicator token and offset, 16px root gap, default drawer width 260)
- https://www.rfc-editor.org/rfc/rfc9051#section-2.3.2 ($Forwarded, $MDNSent, $Junk, $NotJunk, $Phishing)
- https://www.rfc-editor.org/rfc/rfc5788 (keyword registry; initial registrations are $MDNSent, $Forwarded, $SubmitPending, $Submitted only)
- RFC 3503 ($MDNSent): https://www.rfc-editor.org/rfc/rfc3503
- https://github.com/microsoft/Selawik (open-source Segoe UI fallback font)

---

## Fact-check log (2026-10-19)

Scope: every claim tagged **[MS]**, **[FL]** or **[3P]**, plus Graph endpoints, IMAP keywords and the open questions, checked against the sources above and against `reference-measurements.md`. Web search hit its per-turn limit partway through, so some items could only be checked against pages already cited; those stay tagged as estimates or "(unverified)".

Corrected (was wrong or mis-tagged): shadow alpha values are Fluent tokens, not estimates; folder-row hover/pressed tokens and the nav indicator token (Fluent Nav); message list title size (20px, not 16px); folder pane width (216, not 260); folder row height (36, not 32); list header controls (Select, Jump to, Filter, sort); pane surfaces and radius; search box placement and size; date bucket "This month" replaces "Earlier this month", and "Two/Three weeks ago" removed; columns-layout row height, Received formats and unread styling; read-row text colour in dark mode; undo-send range (30 s documented); Focused Inbox settings path; hover-action option list; "Clear all categories" label; category name casing; DarkSteel pill colours; signatures (check boxes, Message tab > Insert group); Junk Email "Delete all"; Restore destination now **[MS]**; "View message details" now **[MS]**; domain blocking is supported in the new Outlook; $Junk/$NotJunk credited to RFC 9051, not RFC 5788; Clutter and Conversation History descriptions; Graph `scheduled` mapping to Snoozed marked unverified; Top results is a classic-Outlook article; search number-matching rule; day/month-name note is classic-only.

Re-tagged "(unverified)": "Custom…" flag in the new Outlook; Schedule send presets; "View message source"; Notes read-only; "New folder" button; "Show blocked content"; Outlook.com image proxy default; Report button defaulting to phishing; report confirmation dialog; block-or-unsubscribe while reporting; Outlook.com deleting messages on block; Ctrl+Shift+V plain paste; recent searches in suggestions.

Added: Graph limits that force client-side pin, snooze, sweep, ignore, favorites, signatures and sender lists; Graph delete vs move vs permanentDelete; mark-all-as-read batching; reportMessage/markAsJunk status and the `/beta` ban; EWS retirement; hidden-folder listing; Archive creation on IMAP; IMAP keyword permission check and `$MDNSent`; extra search keywords; sweep-rule settings page; ignore expiry; read-receipt and blocking limits for IMAP/third-party accounts in Outlook; font stack aligned with the reference file; the calendar.md category-colour conflict.
