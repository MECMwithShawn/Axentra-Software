# Reference measurements (Outlook on the web, October 2026)

Measured from the rendered page of outlook.live.com at a 1440×900 viewport,
dark theme with a background-image theme, user density setting as configured
(folder rows came out at 36px). These are measurements of the rendered layout
only: no Outlook source, CSS or scripts were copied. Where these numbers and
`docs/spec/*.md` disagree, these win for geometry; the spec wins for behaviour.

## Global frame

| Region | x | y | w | h | Notes |
|---|---|---|---|---|---|
| App header | 0 | 0 | 1440 | 48 | Waffle (app launcher) at far left, app name "Outlook" next to it (we show "Axentra Outlook") |
| Header search box | 266 | 8 | 356 | 32 | Radius 4px, 14px text, placeholder "Search"; its left edge lines up with the message list's left edge |
| Left app rail | 4 | 48 | 40 | 184 (5 icons) | Mail, Calendar, People, To Do, Apps; icons ~20px in 40×36 targets |
| Ribbon card | 49 | 48 | 1370 | 81 | Rounded 8px card floating on the canvas; in dark/image theme bg `rgba(0,0,0,.65)` |
| Ribbon tab row | 145 | 49 | – | 31 | Tabs "File", "Home", "View", "Help"; tab buttons 27px tall, 14px text; selected tab has underline |
| Hide navigation pane (hamburger) | 59 | 49 | 40 | 32 | Left of the tabs, inside the ribbon card |
| Ribbon command row | 53 | 85 | 1362 | 40 | Command buttons 32px tall, 14px text, 20px icons; split buttons = main part + 24px chevron part |
| Navigation (folder) pane | 50 | 129 | 216 | to bottom | |
| Folder tree items | 58 | 143+ | 190 | 36 | Radius 4px, 14px text; unread counts right-aligned in brand colour |
| Message list region | 264 | 137 | rest | to bottom | Top-right radius 4px |
| Message list header (folder name, Select, Jump to, Filter, sort) | 264 | 137 | – | ~44 | Folder name 20px semibold with favourite star |
| Message rows (single-line / columns layout) | 265 | 220+ | 1136 | 37 | Row bg `#292929` in dark; text `#d6d6d6` |

Font stack on the page: `"Segoe UI", "Segoe UI Web (West European)", -apple-system, BlinkMacSystemFont, Roboto, "Helvetica Neue", sans-serif`.
Base UI text 14px. On Linux we use `"Segoe UI Variable Text", "Segoe UI", Selawik, Inter, "Noto Sans", sans-serif` (Selawik is installed on this machine and is metric-compatible with Segoe UI).

## Dark-theme colours seen

| Use | Value |
|---|---|
| Primary text on panes | `#d6d6d6` (Fluent dark neutralForeground2) |
| Strong text / white | `#ffffff` |
| Secondary text (time labels) | `#adadad` (neutralForeground3) |
| Brand foreground (today, links, unread counts) | `#479ef5` (brand100) |
| Pane / row background | `#292929` (neutralBackground1) |
| Disabled command text | `#5c5c5c` |

## Mail list, columns layout (reading pane hidden)

Seen when the reading pane is off: a grid with column headers **From | Subject | Received ⌄**, a checkbox column at the left, rows 37px, date group headers ("Yesterday", "This Month") as full-width rows with a collapse chevron. Unread rows: sender and subject in brand colour + bold, a 3px brand bar on the row's left edge. Subject column shows subject followed by the preview text in secondary colour on the same line. Received column: time (e.g. "10:30 AM") for today, "Sun 8:16 PM" for this week, `M/D/YYYY` for older.

The list header has, right-aligned: **Select**, **Jump to**, **Filter**, **By Date ⌄** (sort).
The Mail Home ribbon (simplified) in order: New mail (split, primary blue), Delete (split), Archive, Report (split), Sweep, Move to (split), Undo-ish reply split (Reply / Reply all / Forward), Quick steps (split), Read / Unread, Categorize (split), Flag (split), Pin, Snooze (split), Print, Immersive reader-style button, Rules-like overflow, "…" overflow. The ribbon shows commands greyed out (disabled) when no message is selected.

## Calendar (Work week view)

| Element | Measurement |
|---|---|
| Hour row height | 80px (two 40px half-hour slots) |
| Time gutter label | 12px, `#adadad`, 48px wide box, right side of gutter; format "1 PM" |
| Day header | date number 20px (bold 700 for today, 400 others) above weekday 12px ("Mon"); today in brand colour `#479ef5` with a 2px brand line on top of the column |
| Day column width (5 days, 1440 wide) | ~215px pitch |
| Range title | "October 19–23, 2026" (en dash), 20px semibold, with a dropdown chevron |
| Today button | 110×32 with calendar icon + "Today"; then ‹ › arrows; then the title |
| Mini month | header "October 2026" 14px semibold with ↑ ↓ arrows at right; day cells 24×24 on a 28px pitch, 12px text; current week row highlighted; today filled brand circle |
| Left pane below mini month | "Add calendar", then group "My calendars" (chevron + checkbox), calendars listed with coloured checkboxes, "Show all" link |
| Calendar Home ribbon, in order | New event (split, primary), Day (split), Work week, Week, Month, Split view, Filter (dropdown), Share, Print |

## People / To Do

Not measured (time-zone dialog was up; nothing was dismissed on the user's account). Use `docs/spec/people-todo.md`.
