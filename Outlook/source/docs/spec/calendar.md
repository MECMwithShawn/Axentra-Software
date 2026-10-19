# Axentra Outlook — Calendar module specification

Status: draft v1 · Area: Calendar · Target: Rust + Dioxus desktop (HTML/CSS rendering) on Linux

Axentra Outlook mirrors the layout and behaviour of the **new Outlook for Windows / Outlook on the web** (Fluent 2 era, 2024–2026). Where classic Outlook (Win32) behaves differently, that is called out as **Classic differs:**.

## 0. Conventions used in this document

| Marker | Meaning |
|---|---|
| plain statement | Backed by a public source listed in **Sources** (Microsoft Support, Microsoft Learn / Graph, Fluent UI repo tokens, RFCs, or a named third-party write-up). |
| **(estimate)** | No public number or label exists. This is our best guess, kept consistent with Fluent 2. Engineers may tune it. It is not a documented fact. |
| **(third-party)** | Comes only from a non-Microsoft write-up. Treat it as likely, not confirmed. |
| **(unverified)** | A claim that came from an earlier draft or memory and that the fact-check pass (2026-10-19) could not confirm against a reachable public source. Treat it like an estimate until someone confirms it. |
| `token` | A Fluent 2 design token name from `microsoft/fluentui` `packages/tokens` (MIT). Hex values are given for the **webLightTheme** and **webDarkTheme** brand ramp (`brandWeb`). |
| "Label" | An exact UI string. Strings marked (estimate) are proposed copy. |

All examples use locale en-US, the time zone America/New_York, and "today" = Monday, October 19, 2026.

---

## 1. Module layout

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ Title bar / search (shell spec)                                               │
├───────────────────────────────────────────────────────────────────────────────┤
│ Ribbon tabs: Home | View | Help        (Simplified ribbon, shell spec)         │
│ Home commands: [New event ▾] | Day ▾ | Work week | Week | Month | Split view | │
│                Filter ▾ | Share ▾ | Print                                      │
├──┬──────────────────┬─────────────────────────────────────────────────────────┤
│A │ Folder pane      │ Surface header: [Today] [<] [>]  October 18–24, 2026 ▾   │
│p │ ┌──────────────┐ │                                       (weather, optional)│
│p │ │ New event    │ ├─────────────────────────────────────────────────────────┤
│  │ ├──────────────┤ │ Day header row   Sun 18 | Mon 19 (today) | Tue 20 | ...  │
│b │ │ Mini month   │ ├─────────────────────────────────────────────────────────┤
│a │ ├──────────────┤ │ All-day row                                             │
│r │ │ Add calendar │ ├────┬────────────────────────────────────────────────────┤
│  │ │ My calendars │ │gut-│ Time grid (scrolls vertically)                     │
│  │ │  ☑ Calendar  │ │ter │   events, current-time line, working-hours shading │
│  │ │  ☑ Birthdays │ │    │                                                    │
│  │ │ Other groups │ │    │                                                    │
│  │ └──────────────┘ │    │                                                    │
└──┴──────────────────┴────┴────────────────────────────────────────────────────┘
```

| Region | Size | Notes |
|---|---|---|
| App bar (module switcher) | Shell spec | Calendar is the second module. `Ctrl+2` opens it in new Outlook for Windows; the web shortcut set uses `Ctrl+Shift+2` (both documented). |
| Folder pane | 260 px default, resizable 200–360 px (estimate) | It collapses through the shell's "Hide navigation pane" toggle. The pane auto-hides when the window is narrower than 1024 px (estimate). |
| Surface header | 48 px tall (estimate) | Sticky above the grid. |
| Day header row | 52 px tall (estimate) | Sticky. |
| All-day row | min 28 px, grows (see §6.4) (estimate) | Sticky. |
| Time grid | fills the rest, scrolls on the y axis | It scrolls on the x axis only in Split view when the panes overflow. |
| My Day pane (optional, right) | 320 px (estimate) | Opened from the "My Day" icon at the top right. It shows a Calendar tab with a date picker and upcoming events, and can switch between Agenda and Day (third-party). It is out of scope for v1 except as a stub. |

Reopening the calendar restores the last view used (third-party). Persist `{view, day_count, anchor_date, split, scroll_top}` per account.

Classic differs: classic has a To-Do Bar and Reading Pane, plus a Schedule view and List view. Microsoft notes that "Calendar Peek, List view and Schedule view" are not available in new Outlook, so Axentra omits them.

---

## 2. Design tokens used by Calendar

### 2.1 Neutral and brand tokens (Fluent 2, `brandWeb` ramp)

| Use in Calendar | Token | Light | Dark |
|---|---|---|---|
| Surface / working-hours slot bg | `colorNeutralBackground1` | #FFFFFF | #292929 |
| Non-working-hours slot bg | `colorNeutralBackground2` (estimate mapping) | #FAFAFA | #1F1F1F |
| Folder pane bg | `colorNeutralBackground3` (estimate mapping) | #F5F5F5 | #141414 |
| Hover (rows, slots) | `colorNeutralBackground1Hover` | #F5F5F5 | #3D3D3D |
| Pressed | `colorNeutralBackground1Pressed` | #E0E0E0 | #1F1F1F |
| Selected row (calendar list) | `colorNeutralBackground1Selected` | #EBEBEB | #383838 |
| Subtle button hover / pressed | `colorSubtleBackgroundHover` / `Pressed` | #F5F5F5 / #E0E0E0 | #383838 / #2E2E2E |
| Primary text | `colorNeutralForeground1` | #242424 | #FFFFFF |
| Secondary text (weekday labels, location) | `colorNeutralForeground2` | #424242 | #D6D6D6 |
| Tertiary text (gutter hours) | `colorNeutralForeground3` | #616161 | #ADADAD |
| Out-of-month days, week numbers | `colorNeutralForeground4` | #707070 | #999999 |
| Disabled | `colorNeutralForegroundDisabled` | #BDBDBD | #5C5C5C |
| Hour grid line | `colorNeutralStroke2` | #E0E0E0 | #525252 |
| Half-hour / sub-slot line | `colorNeutralStroke3` (estimate mapping) | #F0F0F0 | #3D3D3D |
| Card / popover border | `colorNeutralStroke1` | #D1D1D1 | #666666 |
| Brand fill (New event button, today circle) | `colorBrandBackground` | #0F6CBD | #115EA3 |
| Brand fill hover | `colorBrandBackgroundHover` | #115EA3 | #0F6CBD |
| Brand fill pressed | `colorBrandBackgroundPressed` | #0C3B5E | #0C3B5E |
| Brand fill selected | `colorBrandBackgroundSelected` | #0F548C | #0F548C |
| Today column header tint, selected slot | `colorBrandBackground2` (estimate mapping) | #EBF3FC | #082338 |
| Brand text / links / today date label | `colorBrandForeground1` | #0F6CBD | #479EF5 |
| Brand stroke (selected mini-month day) | `colorBrandStroke1` | #0F6CBD | #479EF5 |
| Mini-month hover | `colorBrandBackgroundInvertedHover` | #EBF3FC | #EBF3FC |
| Mini-month pressed | `colorBrandBackgroundInvertedPressed` | #B4D6FA | #B4D6FA |
| Mini-month selected range | `colorBrandBackgroundInvertedSelected` | #CFE4FA | #CFE4FA |
| Text on inverted brand bg | `colorNeutralForeground1Static` | #242424 | #242424 |
| Current-time line and dot | `colorPaletteRedBorder2` | #D13438 | #E37D80 |
| Focus ring outer / inner | `colorStrokeFocus2` / `colorStrokeFocus1` | #000000 / #FFFFFF | #FFFFFF / #000000 |
| Modal overlay | `colorBackgroundOverlay` | rgba(0,0,0,0.4) | rgba(0,0,0,0.5) |
| Selected toggle button bg (Day / Week / Month) | `colorSubtleBackgroundSelected` | #EBEBEB | #333333 |
| Text and icons on brand fill | `colorNeutralForegroundOnBrand` | #FFFFFF | #FFFFFF |
| Event marker dot in mini month | `colorBrandForeground2` | #115EA3 | #62ABF5 |
| Disabled button bg | `colorNeutralBackgroundDisabled` | #F0F0F0 | #141414 |
| Loading shimmer bars (§18) | `colorNeutralStencil1` | #E6E6E6 | #575757 |

All hex values in this table were re-checked against `packages/tokens/src/alias/lightColor.ts`, `darkColor.ts` and `global/brandColors.ts` (`brandWeb`) on 2026-10-19. **Do not copy hex values from the comments in the alias files.** Those comments are generated from a different default brand ramp (for example they show `#0078d4` for `brand[80]`), while `webLightTheme` / `webDarkTheme` resolve `brand[80]` to `#0F6CBD`. Always resolve the `brand[n]` index through `brandWeb`.

The Fluent mini-calendar component uses the inverted brand tokens in both themes, so the mini month keeps a light-blue range highlight in dark mode. If that looks too bright in dark mode, swap to `colorBrandBackground2` (#082338) with `colorNeutralForeground1` text (estimate).

### 2.2 Typography, radius, elevation, motion

| Token | Value | Calendar use |
|---|---|---|
| `fontFamilyBase` | 'Segoe UI', 'Segoe UI Web (West European)', -apple-system, BlinkMacSystemFont, Roboto, 'Helvetica Neue', sans-serif | Append `'Noto Sans', 'Cantarell', 'Ubuntu', sans-serif` for Linux. Segoe UI is not shipped on Linux. |
| `fontSizeBase100` / `lineHeightBase100` | 10 px / 14 px | Week-number column in mini month (estimate) |
| `fontSizeBase200` / `lineHeightBase200` | 12 px / 16 px | Gutter labels, event secondary lines, mini-month days |
| `fontSizeBase300` / `lineHeightBase300` | 14 px / 20 px | Body, calendar list, peek-card body |
| `fontSizeBase400` / `lineHeightBase400` | 16 px / 22 px | Peek-card title (estimate) |
| `fontSizeBase500` / `lineHeightBase500` | 20 px / 28 px | Surface header date title, day-header date number (estimate) |
| `fontWeightSemibold` | 600 | Event titles, today marker, date title |
| `borderRadiusSmall` / `Medium` / `Large` / `XLarge` | 2 / 4 / 6 / 8 px | Checkbox / event block, buttons / cards / peek card and dialogs |
| `shadow4` | `0 0 2px ambient, 0 2px 4px key` | Event block while dragging (estimate) |
| `shadow16` | `0 0 2px ambient, 0 8px 16px key` | Peek card, quick-create callout, menus |
| `shadow64` | `0 0 8px ambient, 0 32px 64px key` | Modal dialogs (Repeat, Share, Print, Add calendar) |
| Shadow colours | ambient rgba(0,0,0,0.12) / key rgba(0,0,0,0.14) light; 0.24 / 0.28 dark | — |
| Durations | fast 150 ms, normal 200 ms, gentle 250 ms | Popover fade/scale 150 ms. View switch has no animation (estimate). |

### 2.3 Calendar colour palette

Each calendar has one colour. Graph exposes `calendar.color` (`auto`, `lightBlue`, `lightGreen`, `lightOrange`, `lightGray`, `lightYellow`, `lightTeal`, `lightPink`, `lightBrown`, `lightRed`, plus the enum sentinel `maxColor`, which is not a real colour; treat it like `auto`) and a read-only `hexColor`. Graph leaves `hexColor` empty when the user has never picked a colour explicitly, so fall back to the enum mapping below when it is empty. When `hexColor` is present, it wins over the enum and goes through the custom-hex derivation further down. Axentra maps each name to a Fluent shared-colour ramp. The hex values below are real Fluent palette values. **The choice of which tint or shade is used for each role is (estimate).**

| Axentra colour (Graph enum) | Ramp | Bar / checkbox (light) = primary | Fill (light) = tint50 | Text (light) = shade30 | Fill (dark) = shade30 | Bar (dark) = tint30 | Secondary text (dark) = tint50 |
|---|---|---|---|---|---|---|---|
| Blue (`auto`, `lightBlue`) | blue | #0078D4 | #D0E7F8 | #004377 | #004377 | #5CAAE5 | #D0E7F8 |
| Teal (`lightTeal`) | lightTeal | #00B7C3 | #CEF3F5 | #00666D | #00666D | #58D3DB | #CEF3F5 |
| Green (`lightGreen`) | green | #107C10 | #C9EAC9 | #094509 | #094509 | #54B054 | #C9EAC9 |
| Orange (`lightOrange`) | orange | #F7630C | #FEE5D7 | #8A3707 | #8A3707 | #FAA06B | #FEE5D7 |
| Yellow (`lightYellow`) | marigold | #EAA300 | #FCEFD3 | #835B00 | #835B00 | #F2C661 | #FCEFD3 |
| Red (`lightRed`) | red | #D13438 | #F8DADB | #751D1F | #751D1F | #E37D80 | #F8DADB |
| Pink (`lightPink`) | pink | #E43BA6 | #FBDDF0 | #80215D | #80215D | #EF85C8 | #FBDDF0 |
| Brown (`lightBrown`) | brown | #8E562E | #EDDED3 | #50301A | #50301A | #BB8F6F | #EDDED3 |
| Gray (`lightGray`) | platinum | #69797E | #E4E9EA | #3B4447 | #3B4447 | #A0ADB2 | #E4E9EA |
| Purple (extra) | lavender | #7160E8 | #E7E4FB | #3F3682 | #3F3682 | #A79CF1 | #E7E4FB |
| Cranberry (extra) | cranberry | #C50F1F | #F6D1D5 | #6E0811 | #6E0811 | #DC626D | #F6D1D5 |
| Grape (extra) | grape | #881798 | #EACEEF | #4C0D55 | #4C0D55 | #B55FC1 | #EACEEF |
| Dark teal (extra) | teal | #038387 | #C7EBEC | #02494C | #02494C | #4CB4B7 | #C7EBEC |
| Steel (extra) | steel | #005B70 | #C3E1E8 | #00333F | #00333F | #4496A9 | #C3E1E8 |
| Magenta (extra) | magenta | #BF0077 | #F5CEE6 | #6B0043 | #6B0043 | #D957A8 | #F5CEE6 |

- Dark-mode event title text is `#FFFFFF`.
- The default calendar starts as Blue.
- New Outlook also offers a "Custom" colour (third-party). For a custom hex, derive the ramp at runtime: light fill = mix(primary, #FFFFFF, 80%), light text = mix(primary, #000000, 55%), dark fill = mix(primary, #000000, 55%), dark bar = mix(primary, #FFFFFF, 35%) (estimate). Check WCAG contrast ≥ 4.5:1 for the title text and fall back to `colorNeutralForeground1` if it fails.
- Each calendar can also have a **charm**, a small icon chosen from a fixed set. It shows next to the calendar name and on that calendar's events.

### 2.4 Category colours

Categories come from the user's master list. Graph `outlookCategory.color` takes the values `None`, `Preset0`–`Preset24`. The Graph docs give a colour name for each preset but say the name is what the Outlook desktop client shows, and that the actual rendered colour depends on the client. So no Microsoft hex values exist for these. The `displayName` of a category is unique and cannot be renamed through Graph after it is created. If an event has categories, its block uses the **first category's colour** in place of the calendar colour (estimate; classic Outlook behaves this way). Mapping to Fluent palette values (estimate). Most are ramp primaries; a few are shades or other ramps, noted in brackets so the source is traceable:

| Preset | Name | Hex | Preset | Name | Hex |
|---|---|---|---|---|---|
| Preset0 | Red | #D13438 | Preset13 | DarkGray | #5D5A58 |
| Preset1 | Orange | #F7630C | Preset14 | Black | #242424 |
| Preset2 | Brown | #8E562E | Preset15 | DarkRed | #750B1C |
| Preset3 | Yellow | #EAA300 | Preset16 | DarkOrange | #CA5010 |
| Preset4 | Green | #107C10 | Preset17 | DarkBrown | #6C4123 |
| Preset5 | Teal | #038387 | Preset18 | DarkYellow | #C19C00 |
| Preset6 | Olive | #498205 | Preset19 | DarkGreen | #0B6A0B |
| Preset7 | Blue | #0078D4 | Preset20 | DarkTeal | #026467 |
| Preset8 | Purple | #5C2E91 | Preset21 | DarkOlive | #986F0B |
| Preset9 | Cranberry | #C50F1F | Preset22 | DarkBlue | #004E8C |
| Preset10 | Steel | #005B70 | Preset23 | DarkPurple | #3F3682 |
| Preset11 | DarkSteel | #394146 | Preset24 | DarkCranberry | #77004D |
| Preset12 | Gray | #69797E | None | (no colour; use calendar colour) | — |

Ramp sources for the non-obvious rows: DarkGray = `mink` primary, Black = `grey[14]` (same as `colorNeutralForeground1`), DarkOrange = `pumpkin` primary, DarkBrown = `brown` shade20, DarkYellow = `gold` primary, DarkTeal = `teal` shade20, DarkOlive = `brass` primary, DarkBlue = `royalBlue` primary, DarkPurple = `lavender` shade30, DarkCranberry = `plum` primary, DarkSteel = `anchor` primary, Olive = `forest` primary. All were checked against `global/colors.ts`.

Fills and text use the custom-hex derivation in §2.3. New Outlook added a "None" colour category in 2025. Since October 2025 (roadmap RM501453, GA), an event in new Outlook can carry several categories, ticked one by one from the right-click menu.

---

## 3. Folder pane (left pane)

Top to bottom:

### 3.1 "New event" button
- Fluent primary button with a full-width split (estimate). Label "New event". Icon `CalendarAdd` (Fluent System Icons, 20 px).
- Height 32 px, radius `borderRadiusMedium`, margin 12 px 12 px 8 px (estimate).
- States: rest `colorBrandBackground`, hover `…Hover`, pressed `…Pressed`, focus = 2 px `colorStrokeFocus2` outer ring with a 1 px `colorStrokeFocus1` inner ring, disabled `colorNeutralBackgroundDisabled` / `colorNeutralForegroundDisabled` (shown while offline with no cached writable calendar).
- Click opens the full event editor (§9). The start is the next half-hour slot after now, the duration is the default meeting length (30 min), and the calendar is the account's default calendar. If the user has selected a slot range in the grid, that range is used instead (estimate).
- Microsoft documents Home › "New Event" as the starting point in new Outlook. A "New event" > "Event" submenu on the web is (unverified). Axentra's split chevron menu holds: "Event" and "Event in <other calendar>…" (estimate).

### 3.2 Mini month (date navigator)
The anatomy follows the Fluent `react-calendar-compat` day grid (documented values from the MIT repo). Axentra-specific choices are marked.

| Part | Spec |
|---|---|
| Header | Month and year, e.g. "October 2026", 14 px semibold, left-aligned. Two icon buttons on the right: up chevron "Go to previous month" and down chevron "Go to next month" (labels (estimate)). Buttons are 24×24, subtle style. |
| Weekday labels | One letter each ("S M T W T F S"), 12 px, `colorNeutralForeground3`, starting on the user's first day of week (estimate copy). |
| Grid | 7 columns × 6 rows, always 6 rows so the height never changes (estimate). The table is 196 px wide, or 226 px with week numbers (Fluent). |
| Day cell | 2 px padding. The day button is 24×24 px with 24 px line height, 12 px text (`fontSizeBase200`). |
| Today | A brand circle 20×20 px (`colorBrandBackground`) with semibold `colorNeutralForegroundOnBrand` text. |
| Days outside the month | `colorNeutralForeground4`, still clickable (estimate). Fluent "outside bounds" days (beyond min/max date) are disabled. |
| Visible range highlight | Days shown in the main surface (one day, a week, or a work week) get the `colorBrandBackgroundInvertedSelected` band with `borderRadiusMedium` on the outer corners. In Month view, the whole month is not banded; only the selected day is (estimate). |
| Single selected day | Selected background plus a 1 px `colorBrandStroke1` border and `borderRadiusMedium`. |
| Hover / pressed | `colorBrandBackgroundInvertedHover` / `…Pressed`, radius 4 px. |
| Week numbers | Shown when "Show week numbers" is on (estimate; the setting is documented for Month view). The cell is 28×28 px, `colorNeutralForeground4`, 12 px, with a 1 px `colorNeutralStroke2` right border. |
| Event marker dot (optional) | 4 px dot below the date, `colorBrandForeground2` (Fluent supports it). Off by default (estimate). |

Behaviour:
- Clicking a date moves the main surface to the range that contains it and keeps the current view. In Day view it shows that day. In Week or Work week it shows the week that contains it. In Month it moves to that month.
- Clicking a week number switches to Week view for that week (estimate).
- Arrow buttons page the mini month only. The main surface does not move.
- Keyboard: the grid is a roving-tabindex grid. Arrows move by day or week, PageUp/PageDown move by month, Home/End jump to the start or end of the week, and Enter navigates the main surface (estimate; follows Fluent Calendar).
- When the main surface navigates, the mini month follows it to the month of the anchor date.
- An optional second mini month can show below the first when the pane is taller than 900 px (estimate; off in v1).

### 3.3 "Add calendar"
- A text-link style button with a `CalendarAdd` icon, label "Add calendar", placed under the mini month. This placement is documented by a university KB that covers New/Web Outlook.
- It opens the **Add calendar** modal (§3.5).

### 3.4 Calendar list ("My calendars" and other groups)
- Calendar groups appear per account. The account header row shows the email address and a "More options" (…) button with the item "New calendar group" (documented).
- Default groups: "My calendars" (documented). Others are created as needed: "Other calendars", "People's calendars", and "Group calendars" (labels (estimate)). User groups come from "New calendar group".
- Group header row: chevron (expand/collapse, state persisted), group name (14 px semibold), and a hover "More options" button with "Rename group", "Delete group", "Move up", "Move down" (estimate).
- Calendar row (height 32 px (estimate)):
  `[coloured checkbox 16 px] [charm icon, optional] [name, 14 px, ellipsis] [shared/subscribed glyph] [… on hover/focus]`
  - The checkbox follows Fluent Checkbox (16×16, `borderRadiusSmall`). Checked: the fill and border use the calendar's **primary** colour with a white check. Unchecked: a 1 px border in the calendar primary colour over a transparent fill (estimate colour override).
  - Toggling the checkbox shows or hides the calendar's events on the surface. At least one calendar must stay checked; unchecking the last one is blocked with a brief shake and no change (estimate).
  - Clicking the **name** (not the checkbox) selects the calendar as the "active" calendar (row bg `colorNeutralBackground1Selected`) and makes sure it is checked (estimate).
  - "More options" menu (documented items: colour, charm, rename, remove, move to, share): "Color" ▸ (a swatch grid of §2.3 plus "Custom"), "Charm" ▸, "Rename", "Sharing and permissions", "Move to" ▸ (list of groups), "Remove" (for added or subscribed calendars) or "Delete" (for owned secondary calendars). The default calendar cannot be removed or deleted; drive this from Graph `isRemovable` rather than `isDefaultCalendar` alone. Microsoft documents right-clicking a calendar as the way to reach "Sharing and permissions", so the same menu opens on right-click as well as from "…".
- Drag-and-drop reorders rows within and across groups (estimate).
- **Empty state:** a group with no calendars shows the muted text "No calendars" (estimate).
- **Error state:** a subscribed calendar that failed to sync shows a warning icon. Its tooltip reads "Couldn't update this calendar. Last updated <relative time>." (estimate). Microsoft documents that subscribed calendars refresh roughly every 6 hours for work or school accounts (roughly every 3 hours for Outlook.com), and that a refresh can take more than 24 hours. For ICS subscriptions Axentra fetches itself (non-Exchange accounts), poll every 3 hours and honour `REFRESH-INTERVAL` (RFC 7986) when the feed sets one (estimate).
- **Permission flags to read per calendar (Graph):** `canEdit` (hide edit affordances when false), `canShare` (only the creator can share), `canViewPrivateItems` (controls private-event redaction, §20 item 13), `isRemovable` (whether "Remove"/"Delete" is offered), `isDefaultCalendar`, and `isTallyingResponses`. Graph documents that only invitations sent from the user's primary calendar track responses, so warn before sending a meeting from any other calendar (§9.3).

### 3.5 Add calendar dialog
A modal 800×560 px (estimate), radius 8 px, `shadow64`. A vertical nav on the left switches the content on the right.

| Left nav item (order (estimate)) | Content |
|---|---|
| "Add personal calendars" | Connect a personal Outlook.com or Google calendar to a work or school account (the shell's account flow). Its events then count toward the work account's free/busy. Connected accounts appear in the folder pane. "Remove" disconnects. Work or school accounts only (the Outlook.com version of the dialog does not offer it). |
| "Edit my calendars" (label (unverified)) | Microsoft's article has a section with this name: pick one of your calendars, then rename it, change its colour or charm, or share it. Whether it is a nav item in the dialog was not confirmed. |
| "Create blank calendar" | Name (required), colour swatches, charm picker, "Add to" group dropdown, "Save". |
| "Add from directory" | Search a person, group or resource ("Search by name or email" (estimate copy)), then the documented "Add to" group dropdown, then "Add". Work or school accounts only. |
| "Subscribe from web" | URL field (ICS/iCal), "Calendar name" (label (estimate)), colour, charm, then the documented "Import" and "Save". Read-only. Note "Updates may take more than 24 hours" (estimate copy; the 24-hour figure is documented). |
| "Upload from file" | "Browse" for an `.ics` file, a "Select a calendar" dropdown for the target, then "Import" and "Save" (labels documented). This is a one-time snapshot and does not refresh. |
| "Holidays" | A filterable list of countries and regions with checkboxes. Selected ones are added to "My calendars". |

Microsoft's article lists the options in this order: Add personal calendars, Edit my calendars, Create blank calendar, Add a person's calendar (the directory search), subscribe or upload, Holidays. The table above follows that order. The live dialog's order is still (unverified).

The **birthday calendar** is switched on through Settings › Calendar › Calendar appearance (called "View" in some builds) › "Turn on the birthday calendar", not in this dialog. It is built from contacts' birthdays (documented).

---

## 4. Ribbon — Home tab (Calendar)

The Simplified ribbon is a single row. Commands in order (left to right). Each "group" is separated by a 1 px `colorNeutralStroke2` divider, 20 px tall (estimate).

| # | Command | Control type | Icon (Fluent System Icons) | Behaviour / states | Shortcut (new Outlook) |
|---|---|---|---|---|---|
| 1 | "New event" | Split button (primary body plus chevron) | CalendarAdd | Body opens the full editor (§9). The chevron menu lists "Event" and per-calendar options (estimate). | Ctrl+N |
| 2 | "Day" | Split toggle button | CalendarDay | Body selects Day view (1 day). The chevron opens a day-count menu "1 day" … "7 days". Two third-party write-ups (Ablebits, Office Watch) agree the new Outlook range is 1–7 consecutive days; the item labels are (estimate). The "up to 10 days" figure comes from classic Outlook's Alt+0 shortcut and does not apply here. Selected state uses `colorSubtleBackgroundSelected` plus brand-coloured text and icon. | Ctrl+Alt+1 (documented). Alt+3 = three-day view (documented). Alt+1…Alt+7 for other day counts is (estimate), borrowed from classic, which documents Alt+1…Alt+9 and Alt+0 for 1–10 days. |
| 3 | "Work week" | Toggle button | CalendarWorkWeek | Shows the days set in "Show work week as". | Ctrl+Alt+2 (documented) |
| 4 | "Week" | Toggle button | CalendarWeekStart | 7 days from the first day of week. | Alt+Minus (documented for new Outlook). Ctrl+Alt+3 is (estimate) for new Outlook: Microsoft's new Outlook table lists Ctrl+Alt+2 for Week as well as Work week, and only classic documents Ctrl+Alt+3. See §15. |
| 5 | "Month" | Toggle button | CalendarMonth | Month grid. | Ctrl+Alt+4 or Alt+= (documented) |
| 6 | "Split view" | Toggle button | SplitVertical | **Disabled unless 2 or more calendars are checked** (documented: Split view is only selectable when several calendars are selected). On: each calendar gets its own column or pane. Off: overlay (merged, colour-coded). Available in Day, Work week, Week, and, since the June–August 2024 rollout (MC793320), Month. | — |
| 7 | "Filter" | Menu button | Filter | See §4.1. Microsoft documents Filter on the **View** tab. Whether it is also on Home is (unverified); Axentra shows it on both. A brand dot badge shows on the icon when any filter is active (estimate). | — |
| 8 | "Share" | Menu button | Share | The menu lists the user's shareable calendars (`canShare = true`). Picking one opens Sharing and permissions (§13.2). | — |
| 9 | "Print" | Button | Print | Opens the print preview dialog (§13.1). Home › Print is documented. | Ctrl+P (estimate: Microsoft's new Outlook calendar table has no print key; Ctrl+P is the classic and browser convention) |

**Not implemented:** "Board" view (retired from Outlook on the web in 2023; the June 2023 date comes from a third-party admin digest that could not be reached during the fact-check, so treat the date as (unverified)), Schedule view, and List view (Microsoft documents that List view and Schedule view are not in new Outlook).

**View tab** (secondary): "Day ▾", "Work week", "Week", "Month", "Split view", "Time scale ▾" (5, 6, 10, 15, 30, 60 minutes; documented), "Filter ▾" (documented; it holds at least "Hide declined events", §4.1), "Saved views ▾" ("Save current view", then a list; third-party, Ablebits), and "Calendar settings" (documented; opens the Calendar page of Settings, §16).

Ribbon responsiveness: when the width can't fit, commands collapse right to left into an overflow "…" menu in this order: Print, Share, Filter, Split view (estimate). View buttons collapse into a single "View: Week ▾" dropdown below 900 px (estimate).

### 4.1 Filter menu (partly documented)
What is known:
- **Documented (Microsoft Support):** View › Filter has a checkable item **"Hide declined events"**. It only matters when "Show declined events on your calendar" is on (§16.3). Unchecking it brings declined events back.
- **Third-party (Chris Menard Training, May 2023, Outlook on the web):** the Filter menu's top-level entries are "All", "Appointments", "Meetings", "Categories", "Show as", "Recurrence" and "Privacy". No public source lists the sub-items.

Axentra's menu, built on that list (sub-items are (estimate)). The filter is client-side and applies to all checked calendars.
- "All" (radio-like reset; checked when no filter is active).
- "Appointments" / "Meetings" (checkboxes; both on by default). Proposed sub-items under "Meetings": "Organized by me", "Invitations I've accepted", "Not yet responded".
- "Categories" ▸ each category with its swatch, plus "Not categorized".
- "Show as" ▸ "Free", "Working elsewhere", "Tentative", "Busy", "Away".
- "Recurrence" ▸ "Recurring", "Not recurring".
- "Privacy" ▸ "Private", "Not private".
- Divider, then "Hide declined events" (documented label).
- Events hidden by a filter are not counted in Month "+N more".

**Classic differs:** classic uses Arrange group buttons (Day, Work Week, Week, Month, Schedule View) and filters through View Settings › Filter.

---

## 5. Surface header (top navigation bar)

Left to right, vertically centred, 48 px tall (estimate), padding 0 16 px:

1. **"Today"** button. Outline/secondary style with a `CalendarToday` icon and label "Today". It navigates to the range containing today. If today is already in view, it scrolls the time grid to the current time (estimate). Tooltip "Go to today (Alt+Shift+Y)" (estimate copy). Microsoft's new Outlook table lists both Alt+Shift+Y and Home for "go to today"; Outlook on the web lists only Alt+Shift+Y.
2. **Previous / Next** icon buttons (ChevronLeft / ChevronRight, 32×32, subtle). Accessible names follow the documented pattern "Go to previous <unit>" / "Go to next <unit>", where the unit is "day", "week" or "month". In N-day view the step is N days (estimate).
3. **Date title**, 20 px semibold (`fontSizeBase500`). It is a button with a chevron that opens a month-picker flyout (a mini-month plus a "Today" link) (estimate). Formats (en-US) (estimate; use ICU interval formatting per locale):

| View | Same month | Crosses month | Crosses year |
|---|---|---|---|
| Day (1) | "Monday, October 19, 2026" | — | — |
| N days (2–7) | "October 19–21, 2026" | "October 30 – November 1, 2026" | "December 31, 2026 – January 2, 2027" |
| Work week | "October 19–23, 2026" | "Oct 29 – Nov 2, 2026" (abbreviated months once the title would exceed the header width) | "Dec 28, 2026 – Jan 1, 2027" |
| Week | "October 18–24, 2026" | same pattern | same pattern |
| Month | "October 2026" | — | — |

4. **Weather** (optional, when "Show weather" is on): icon plus temperature for today. The scale is Fahrenheit or Celsius per setting. Up to five locations are supported. It appears to the right of the title, in 14 px `colorNeutralForeground2`. Hidden in v1 unless a weather provider is configured (estimate).
5. Right side: in Split view, a "Calendars: N" chip (estimate). In compact width, a view dropdown.

---

## 6. Time grid (Day, N-day, Work week, Week)

### 6.1 Structure
- Columns: [time gutter(s)] + one column per day (+ sub-columns per calendar in Split view, §6.12).
- Day columns share the width equally. Minimum day column width is 96 px; below that the grid scrolls horizontally (estimate).
- Optional setting "Dynamic column widths" (third-party). Ablebits describes it only as fitting the grid columns to the available space. Axentra's reading is that days with more side-by-side overlaps get a larger share of the width (estimate interpretation; unconfirmed).
- Vertical extent: always 24 wall-clock hours (00:00–24:00) rendered. Hours are never hidden.

### 6.2 Day header row
- Cell content (estimate): the weekday abbreviation ("Mon") in 12 px `colorNeutralForeground2`, and below it the date number in 20 px.
- The **today** cell has a `colorBrandBackground2` tint. Its date number is `colorBrandForeground1` and semibold, with a 2 px brand underline across the full cell bottom (estimate).
- Days with a work location (work plan) show a small location icon (office or remote) at the right (documented feature: Microsoft says a location icon appears on calendar days once a plan is set, and that a "Work plan" icon in Calendar changes one day's location; the placement is (estimate)). Clicking the icon opens a small menu "Office" / "Remote" / "No location" for that day only (estimate; the three values are the documented location choices).
- Clicking a day header switches to Day view for that day (estimate).

### 6.3 Time gutter
- Width 56 px (estimate). Labels are right-aligned with 8 px right padding, 12 px `colorNeutralForeground3`. Each label is vertically centred on its hour line, except 00:00, which is hidden (estimate).
- Format follows Settings › "Time format":
  - 12-hour (en-US default): "12 AM", "1 AM", … "11 AM", "12 PM", "1 PM" … (estimate: minutes are omitted on whole hours).
  - 24-hour: "00:00", "01:00", … "23:00" (estimate).
- **Additional time zones**: one extra gutter column per zone, 56 px each (estimate), left of the primary gutter. A short label shows at the top of each gutter (the user-typed "Label"). Right-clicking the gutter header opens a context menu "Time zones" with checkboxes to show or hide each zone. Time zones never show in Month view.

### 6.4 All-day row
- Sits between the day headers and the grid and is sticky.
- It holds all-day events and **timed events lasting 24 hours or more** (estimate rule). Each event is a single-line chip 22 px tall (estimate) that spans its day columns. Chips are stacked with 2 px gaps and packed greedily into rows.
- When collapsed, at most 3 rows are visible (estimate). Overflow shows a "+N" chip per day column. A chevron toggle in the gutter cell expands the row to show everything (max 40% of the surface height, then it scrolls) (estimate).
- A chip that continues beyond the visible range shows a left or right notch (a chevron glyph inside the chip edge) (estimate).
- Double-clicking empty all-day space opens the full editor with "All day" on for that date (estimate). A single click opens quick-create with All day on (estimate).

### 6.5 Slots, time scale, and slot height
- Time scale options are 5, 6, 10, 15, 30, and 60 minutes (documented: View › Time scale; Microsoft calls 15 and 30 the common choices). The **default is 30** (Microsoft's new Outlook screenshot shows 30 selected; no explicit default is stated). The older Settings page only documents "Show hours in" with 15 or 30; treat the View-tab list as the current one.
- Every slot boundary is a horizontal line. The **hour** line is solid 1 px `colorNeutralStroke2`. **Sub-hour** lines are 1 px `colorNeutralStroke3`, dashed for 30-min boundaries and solid lighter for the rest (estimate).
- Slot heights (estimate). They are CSS custom properties on the grid root, `--cal-slot-h` and `--cal-hour-h`:

| Time scale | Slot height | Hour height |
|---|---|---|
| 60 min | 40 px | 40 px |
| 30 min (default) | 24 px | 48 px |
| 15 min | 20 px | 80 px |
| 10 min | 18 px | 108 px |
| 6 min | 16 px | 160 px |
| 5 min | 16 px | 192 px |

### 6.6 Working-hours shading
- Slots inside working hours on working days use `colorNeutralBackground1`. Slots outside use `colorNeutralBackground2`; non-working days are fully `colorNeutralBackground2`. Microsoft documents only that non-working hours are darker. The exact token is (estimate).
- Working hours come from Settings › Calendar › "Work hours and location" (per-day hours) or, for accounts without it, a single start/end setting. Hours are converted from the work-hours time zone into the display time zone.

### 6.7 Initial scroll position
- On first render, and when "Today" is pressed with today already visible: if today is in range, scroll so that the current-time line sits 1/3 down the viewport. Otherwise scroll so that 30 minutes before the work-day start is at the top (estimate).
- Persist `scroll_top` across view switches within a session (estimate).

### 6.8 Current-time indicator
- A 2 px horizontal line (estimate) in `colorPaletteRedBorder2` (#D13438 light / #E37D80 dark) at the current time.
- In **today's column** the line is full opacity and has a filled circle **8 px** in diameter (estimate) centred on the column's left edge. In other visible day columns the line is drawn at 30% opacity (estimate).
- The gutter can also show the current time in a small red pill with white 10 px text, e.g. "2:47 PM". Off by default (estimate).
- Update on minute boundaries: one timer aligned to `:00` seconds, with no per-second re-render. When the date rolls over at midnight, re-compute "today" (header tint, mini-month circle, line column) and, if the view is anchored to today and the user hasn't navigated, keep the range (estimate).
- Hidden when today is not in the visible range.
- `z-index` is above events, and the line has `pointer-events: none`.

### 6.9 Event block anatomy (timed events)

```
┌─┬─────────────────────────────────┐
│▌│ Title of event            🔁 🎥🔒│  ← title 12 px semibold, icons 12 px at end (estimate)
│▌│ 9:00 AM – 9:30 AM               │  ← time 12 px regular (only if height allows)
│▌│ Room 4 / Microsoft Teams Meeting│  ← location 12 px regular, secondary colour
└─┴─────────────────────────────────┘
 └ colour bar 4 px (estimate)
```

| Part | Spec |
|---|---|
| Container | `position:absolute`, radius 4 px (`borderRadiusMedium`) (estimate), 1 px gap inside the column on each side and 2 px between overlap columns (estimate). Fill and text colours come from §2.3 or §2.4. |
| Colour bar | 4 px wide on the left (estimate), the calendar or category primary colour (dark: tint30). Its pattern follows Show as (§6.10). |
| Padding | 2 px 6 px 2 px 8 px (estimate). |
| Title | 12 px semibold, ellipsis. If the event height is below 2 lines (< 34 px), the title and start time share one line: "Title, 9:00 AM" (estimate). |
| Time line | "9:00 AM – 9:30 AM", 12 px, shown when height ≥ 34 px (estimate). |
| Location line | Location display name, shown when height ≥ 50 px (estimate). |
| Icons (right-aligned on the title row, in this order) | Recurring `ArrowRepeatAll` (an exception occurrence uses `ArrowRepeatAllOff`), Online meeting `Video`, Private `LockClosed`, Attachment `Attach`, plus the calendar charm when one is set. Icons are 12 px in the text colour (estimate set). |
| Response state | **Not yet responded** invitations render with the Tentative pattern and an italic title (estimate). **Declined** meetings: documented behaviour is that they are removed from the calendar unless the user turns on Settings › Calendar › Events and invitations › "Save declined events" › "Show declined events on your calendar" (off by default; work or school accounts only; Outlook.com, IMAP and POP accounts are not supported). When kept, they stay on the calendar with the time shown as **Free**, and View › Filter › "Hide declined events" hides them again. Microsoft does not document their look; Axentra renders them with the Free outline pattern, a strikethrough title, and 60% opacity (estimate). A decline made in classic Outlook never leaves a kept copy, so a declined event can be missing even with the setting on. **Canceled** meetings still on the calendar show the title prefixed "Canceled: " with strikethrough (estimate styling). |
| Private events from shared calendars | Title "Private event", no details, lock icon (estimate copy). |
| Free/busy-only shares | Block with title "Busy" (or the Show-as label) and no details (estimate copy). |
| Minimum height | One slot or 20 px, whichever is larger (estimate). Zero-length events also render at this height. |

States:
| State | Visual |
|---|---|
| Rest | Fill plus bar as above. |
| Hover | Fill darkens about 4%: light = blend toward the primary 10%, dark = blend toward white 8% (estimate). Cursor: pointer. Title tooltip after 600 ms showing the full title and time (estimate). |
| Selected (single click) | Fill becomes the **primary** colour with `#FFFFFF` text (dark: tint30 fill with `#242424` text) (estimate). The peek card opens. |
| Focused (keyboard) | Fluent focus ring: 2 px `colorStrokeFocus2` outside plus 1 px `colorStrokeFocus1` inside, radius preserved. |
| Dragging | The original stays at 40% opacity. The ghost uses `shadow4` with a live time label (§7.3). |
| Read-only (no edit permission) | Same visuals. Resize handles hidden. Cursor default (estimate). |

### 6.10 Show-as styles
Microsoft documents the new Outlook availability legend and says it applies to both the calendar and the Scheduling Assistant. The patterns for Busy, Tentative and Working elsewhere change to the calendar colour. Free is described only as a light blue outline, so making Free follow the calendar colour is an Axentra choice (estimate). The legend names the `oof` state "Out of office", while the Show-as menu labels it "Away" (third-party, several university KBs). Use "Away" in menus and "Out of office" in the legend and tooltips. CSS is (estimate):

| Show as (Graph `showAs`) | Legend label and description (documented) | Block rendering |
|---|---|---|
| Free (`free`) | "Free": light blue outline | Fill `colorNeutralBackground1`, 1 px border in the calendar primary, bar outlined (2 px border, transparent). |
| Working elsewhere (`workingElsewhere`) | "Working Elsewhere": light outline with an array of small dots | Free styling plus a dot pattern on the bar and fill: `radial-gradient(circle, <primary> 1px, transparent 1.5px) 0 0 / 6px 6px` at 35% opacity on the fill. |
| Tentative (`tentative`) | "Tentative": diagonal stripes | Fill = `repeating-linear-gradient(135deg, <tint50> 0 4px, <tint60 or bg1> 4px 8px)` (dark: shade30 / shade20). The bar uses the same stripes in primary / tint40. |
| Busy (`busy`) | "Busy": solid | Solid fill plus solid bar (default rendering). |
| Away (`oof`) | "Out of office": **pink** cross-hatch; does not follow the calendar colour | Fill = two crossed `repeating-linear-gradient`s (45° and 135°) of pink tint30 #EF85C8 over tint60 #FEF6FB (light). Dark: pink shade20 #AD2D7E over shade30 #80215D. The bar is solid pink primary #E43BA6. (Hex values are real Fluent `pink` ramp values; the exact shades Outlook uses are (estimate).) |
| Unknown / no info (`unknown`) | "Unknown": grey diagonal stripes (classic calls this "No information") | Stripes in `colorNeutralStroke1` / `colorNeutralBackground3`. |
| — | "Outside of working hours": solid grey | Used only in the Scheduling Assistant rows (§9.14). |

**Classic differs:** classic Outlook shows Out of Office in purple and draws the busy-status strip as a separate narrow column at the left of each item.

### 6.11 Overlapping event layout (side-by-side columns)
Algorithm (per day column, recomputed whenever events in that day change):
1. Take the timed segments for the day (multi-day timed events are split at midnight in the display time zone, §6.13).
2. Sort by start time ascending. Ties go to the longer duration first, then calendar order, then title.
3. Build **clusters**: maximal sets of events connected by transitive overlap. Use `[start, max(end, start + min_visual_duration))`, where the minimum visual duration is one slot, so short events don't visually collide.
4. Inside a cluster, greedily put each event into the first **column** whose last event ends at or before this start. Open a new column otherwise.
5. Width = (column width − right gutter) / column count. Leave an **8 px right gutter** in every day column so empty space is always clickable for create (estimate).
6. **Expansion:** an event may stretch right across adjacent columns that are free for its entire span (estimate; this matches Outlook's look).
7. If the computed width is < 28 px, render the remaining events of that cluster as a single "+N" chip at the top of the cluster's area. Clicking it opens a popover listing them (estimate).
- Selected or focused events are raised (`z-index`) without reflowing.

### 6.12 Split view in time grids
- When Split view is on and k ≥ 2 calendars are checked, **Day / N-day** views show each day as k sub-columns, one per calendar in calendar-list order. The day header gets a second line with calendar names, coloured with each calendar's dot (estimate layout).
- **Week / Work week**: one pane per calendar side by side, each with its own day columns and a header showing the calendar name. Pane minimum width is 320 px, and the panes scroll horizontally when they overflow (estimate).
- When Split view is off (overlay), all events merge into the same columns and are distinguished by colour.

### 6.13 Multi-day and boundary events
- Timed events crossing midnight but shorter than 24 h are drawn as one segment per day. The first segment has a squared bottom and the next has a squared top, with a small "continues" chevron at the cut edge (estimate).
- Events that start before or end after the visible range are clipped, with the same continuation affordance.
- **DST:** the grid always shows 24 wall-clock rows. Events are positioned by wall-clock start and end in the display zone. On a spring-forward day, a 01:30–03:30 wall-time event looks 2 h tall but lasts 1 h; the editor still shows the true times. In the 02:00–03:00 gap on spring-forward days, empty-slot creation is disabled and the slot is hatched with `colorNeutralBackground3` (estimate).

---

## 7. Time-grid interactions

### 7.1 Pointer
| Gesture | Result |
|---|---|
| Single click on an empty slot | Selects the slot (fill `colorBrandBackground2` with a 1 px `colorBrandStroke1` border) and opens the **quick-create callout** (§8.2) anchored to it. A second click elsewhere moves the selection. |
| Click-drag over empty slots | Selects a range snapped to the time scale and shows a live label "9:00 AM – 10:30 AM" inside the selection. On release, opens quick-create for the range. Dragging across days selects the same time range on consecutive days → this creates a multi-day event (estimate). |
| Double-click on an empty slot | Opens the full editor (§9) prefilled with that slot. |
| Single click on an event | Selects it and opens the **peek card** (§8.1). |
| Double-click on an event | Opens the event in the full editor (attendees see the read-only meeting view with RSVP, §9.13). For a recurring occurrence, new Outlook opens **that occurrence** and offers series actions in the form (documented labels): "View series" to open the series, and for the organizer "Edit series" or "Edit this and all following events". Outlook on the web instead asks after "Edit": "This event" / "This and all following events" / "All events in the series" (documented). Axentra follows the new Outlook pattern and uses the web wording for the delete and cancel prompts (§9.12). |
| Right-click on an event | Context menu (estimate set, except "Show as ▸", which Microsoft documents as the way to change availability, and "Categorize ▸", where multi-select is documented): "Open", "Accept / Tentative / Decline" (invitations), "Join" (online), "Categorize ▸" (multi-select, a check mark per applied category), "Show as ▸", "Private", "Duplicate event", "Delete" or "Cancel". |
| Drag an event body | Moves it (§7.3). |
| Drag top or bottom edge (6 px hit zone (estimate)) | Resizes it. Cursor `ns-resize`. |

Drag threshold: 4 px of pointer travel before a drag starts. Below that it counts as a click (estimate).

### 7.2 Create (quick path)
- New event defaults: Show as **Busy**, reminder = the default reminder (15 min unless changed), not private, the active (selected) calendar, otherwise the default calendar.
- All-day events default to Show as **Free** (estimate, classic default) and reminder **18 hours before** (third-party, classic default).

### 7.3 Move and resize
- Snap granularity = the current time scale, with a minimum of 15 min (estimate).
- During a drag: the ghost block follows the pointer and shows a live "Tue 9:30 AM – 10:00 AM" label. The source keeps 40% opacity. The grid auto-scrolls when the pointer is within 48 px of the top or bottom edge, at up to 600 px/s (estimate).
- Dragging into another day column changes the date and keeps the duration. Dragging into the all-day row converts the event to all-day. Dragging an all-day chip into the grid converts it to a 30-min timed event at the drop slot (estimate).
- In Split view, dropping on another calendar's column moves the event to that calendar. This is allowed only if both calendars are editable and on the same account (estimate). Otherwise the drop is refused with a tooltip.
- `Esc` during a drag cancels and restores the event.
- **Permissions:**
  - Read-only calendars: no drag or resize.
  - Meetings organised by someone else: move and resize are disabled (estimate). The user can propose a new time instead.
- **Recurring:** after the drop, prompt "Move this event" / "Move all events in the series" (estimate copy). Moving the whole series by drag only shifts the time of day. Dropping on another day for a series is allowed only as "This event".
- **Meetings you organise with attendees:** after the drop, a dialog asks "Send an update to attendees?" with the buttons "Send update", "Don't send", "Cancel" (estimate copy). "Don't send" saves locally and leaves `isDraft`-style pending changes (§9.12).
- **Optimistic UI:** apply the change immediately and roll back with a toast "Couldn't move the event. Try again." on server failure (estimate copy).

### 7.4 Keyboard inside the grid (estimate, modelled on classic Outlook's documented Day/Week keys)
| Key | Action |
|---|---|
| Tab / Shift+Tab | Next / previous event in chronological order across the visible range. |
| Arrow Up / Down | With no event focused, move the selected slot by one slot. With an event focused, move focus to the nearest event above or below in the same day. |
| Arrow Left / Right | Same slot on the previous / next day. At the range edge, navigate to the previous / next range. |
| Shift + Arrow | Extend the slot selection. |
| Home | Go to today (documented for new Outlook as an alternative to Alt+Shift+Y). |
| Ctrl+Home / Ctrl+End | Selection to the start / end of working hours (estimate). Classic documents plain Home / End for this, but new Outlook gives Home to "go to today", so Axentra moves it to Ctrl. |
| Page Up / Page Down | Scroll the grid by one viewport (classic Week view documents this). |
| Enter | On a focused event, open it. On a slot selection, open quick-create. |
| Space | On a focused event, open its peek card. |
| Delete | Delete or cancel the focused event (with confirmation for meetings and series). |
| Esc | Close the peek card or quick-create, clear the selection. |
| F6 / Ctrl+Shift+Tab | Move focus between regions (folder pane → ribbon → surface header → grid). The keys are documented for new Outlook; the region order is (estimate). |

---

## 8. Popovers on the surface

### 8.1 Peek card (single click on an event)
- Popover width 360 px, max height 480 px with internal scroll (estimate), radius 8 px (`borderRadiusXLarge`), `shadow16`, bg `colorNeutralBackground1`, 1 px `colorNeutralStroke1` border in high contrast only.
- Placement: to the right of the event, flipping left, then below or above. It keeps 8 px from window edges. An arrow is optional (estimate: none).
- Closes on: Esc, a click outside, scrolling the grid more than 40 px, or opening another event. Focus moves into the card on open and returns to the event on close.

Anatomy, top to bottom (estimate layout; Microsoft documents the content):
1. **Header action bar** (right-aligned icon buttons, 32×32): organizer only: "Edit" (Edit icon), "Delete" (appointments) or "Cancel" (meetings you organise; Outlook on the web documents the label as "Cancel", so use "Cancel meeting" only as the tooltip or accessible name (estimate)). Everyone: "Open in new window" (Open icon), "More options" (…), "Close" (Dismiss).
2. A colour strip 4 px tall across the card top in the calendar colour (estimate).
3. **Title**, 16 px semibold, wraps up to 3 lines.
4. **When**: "Mon 10/19/2026 9:00 AM – 9:30 AM". For all-day events: "Mon 10/19/2026 (All day)". For multi-day all-day events: "Mon 10/19/2026 – Wed 10/21/2026". A recurrence summary follows on the next line, e.g. "Occurs every Monday" (estimate formats).
5. **Location** (Location icon) as text. If the location is a URL it renders as a link.
6. **Join** button (primary, `Video` icon, label "Join") when the event has an online meeting `joinUrl`. It sits full width under the time. It is enabled at all times and visually emphasised from 5 minutes before the start until the end (estimate).
7. **Organizer**: avatar (24 px) + "<Name>" + "Organizer" caption.
8. **RSVP** (when the user is an attendee and `responseRequested`): three buttons "Accept", "Tentative", "Decline", each with a chevron for send options (§11.2). The current response is shown pressed with an icon (Checkmark / Question / Dismiss) (estimate).
9. **Attendees summary** for the organizer: "Accepted 3, Tentative 1, Declined 0, Didn't respond 2" (estimate copy). Clicking it expands the list with response icons.
10. **Body preview**: the first 3 lines of `bodyPreview`, `colorNeutralForeground2`.
11. Reminder line ("Reminder: 15 minutes before") and calendar name with its colour dot (estimate).

### 8.2 Quick-create callout (click or drag on an empty slot)
Width 400 px (estimate), same chrome as the peek card. Fields, in order:
1. Calendar picker (only if the user has more than one writable calendar): colour dot + name ▾.
2. Title input, placeholder "Add a title" (estimate; Microsoft's Outlook on the web article names the field "Add a title to your event"), autofocus.
3. Date/time line: date picker, start time, "to", end time, and an **"All day"** toggle (Fluent Switch; "All day" is documented).
4. "Invite required attendees" people picker (documented label). Availability for invitees appears under the date/time line when attendees are present.
5. "Search for a room or location" with suggestions (documented label; some accounts show "Add a room" instead).
6. **"Teams meeting"** switch (documented label), shown when the calendar's `allowedOnlineMeetingProviders` includes `teamsForBusiness`. Otherwise it reads "Online meeting" (estimate) or is hidden.
7. Footer: "More options" (secondary; opens the full editor with all entered values; label (unverified): the Outlook on the web article calls this "More details") and a primary **"Save"** (no attendees) or **"Send"** (≥1 attendee).
- Enter in the title field = Save/Send. Esc closes. If anything was typed, a confirm dialog "Discard this event?" appears with the buttons "Discard" and "Keep editing" (estimate copy).

---

## 9. Event editor (full form)

### 9.1 Container
- In-app it opens as a full-surface panel layered over the calendar, with its own toolbar. It can be popped out into a separate window ("Open in new window"). This follows a setting like Outlook's pop-out behaviour option (third-party) (estimate default: in-app panel).
- Two tabs directly under the toolbar: **"Event"** and **"Scheduling Assistant"** (documented). The Scheduling Assistant tab shows only for accounts that can query free/busy (work or school).

### 9.2 Toolbar (left to right) (order (estimate); command names documented unless marked)
| Control | Notes |
|---|---|
| **"Save"** / **"Send"** (primary) | "Save" for appointments, "Send" when there is at least one attendee (documented). When the organizer edits an existing meeting, the label stays "Send" and sends an update. Shortcuts: Ctrl+S (save) and Ctrl+Enter or Alt+S (send) are documented for the Outlook on the web form; new Outlook documents Ctrl+Enter for sending mail, and its event-form keys are otherwise (estimate). |
| **"Discard"** (Delete icon) | Closes without saving. If the form is dirty, a confirm dialog "Discard changes?" appears with the buttons "Discard" and "Cancel" (estimate copy). For an existing meeting you organise, the toolbar shows **"Cancel"** (documented web label; accessible name "Cancel meeting") instead of Discard (§9.12). |
| Show as menu (label = the current value, e.g. **"Busy"**, documented) | Radio items "Free", "Working elsewhere", "Tentative", "Busy", "Away" (third-party list, consistent across several university KBs), then a divider and a **"Private"** checkbox item (third-party: listed last and off by default). |
| **Reminder** menu (label = the current value, e.g. "15 minutes before") | Documented as a "Reminder" dropdown in the form's Options group in new Outlook, and as "Remind me" in Outlook on the web. For a series, Microsoft says to open the series first ("View series"). Items: §9.9. |
| **"Recurring"** button | Documented in new Outlook's meeting steps: opens the Repeat dialog (§10.2). It duplicates the inline Repeat dropdown in the form body (§9.3 row 5). |
| **"Categorize"** menu | A checkbox list of categories with swatches (multi-select), "New category", "Manage categories" (estimate items beyond the list). |
| **"Response options"** menu | Checkbox items: "Request responses" (on by default; documented in classic Outlook on the web; Graph `responseRequested` default true), "Allow forwarding" (on; label (estimate)), "Hide attendee list" (off; documented for new Outlook and the web, not available in classic Outlook for Windows; Graph `hideAttendees`), "Allow new time proposals" (on; Graph `allowNewTimeProposals` default true; label (estimate)). |
| **"Time zones"** toggle (Globe icon) | Shows per-field time-zone pickers (§9.6) (label (estimate)). |
| "…" overflow | "Print", "Insert ▸ (Attach file, Table, Link)", "Duplicate event" (estimate). |

Toolbar commands collapse into "…" when the width can't fit. Primary and Discard never collapse.

### 9.3 Form body (Event tab), in order
| # | Field | Spec |
|---|---|---|
| 1 | Calendar picker | Only when more than one writable calendar exists. "<colour dot> Calendar — user@domain ▾". Changing it after attendees are added warns that responses will not be tracked. Graph documents that only invitations sent from the user's primary calendar track responses (`isTallyingResponses`); the warning copy is (estimate). |
| 2 | **Title** | Single-line, 20 px (estimate), placeholder "Add a title". It is required for Send. For Save, an empty title saves as "(No title)" (estimate). |
| 3 | **Attendees** | A people picker labelled "Invite required attendees" (documented web label; new Outlook's article says "Invite attendees") with an **"Optional"** link at the right (documented) that reveals a second picker "Invite optional attendees" (estimate label). Chips show the avatar, the name, a free/busy dot for the selected time, and × to remove. Chip context menu: "Make optional" / "Make required", "Remove" (estimate). Rooms added here become resources. Graph documents a maximum of 500 attendees per event (the Exchange Online per-message recipient limit). Block Send above that with an inline error (copy (estimate)). |
| 4 | **Date and time** | `[Start date ▾] [Start time ▾] → [End time ▾] [End date ▾]` + **"All day"** switch. The end date shows only when it differs from the start date or when the user expands it (estimate). Suggested time chips appear below when attendees exist: "Suggested times: 10:00 AM, 1:30 PM, 3:00 PM" (estimate copy). |
| 5 | **Repeat** | A dropdown button whose resting label is **"Don't repeat"** (§10). Recurring Repeat icon. |
| 6 | **Location** | Input "Search for a room or location". Suggestions show recent locations, rooms ("Browse more rooms" opens the room finder; label (unverified)), and address lookups. Multiple locations are supported (Graph `locations[]`). Graph keeps `location` and `locations` in step: writing `location` replaces the whole `locations` list, so always write `locations` when more than one is set. |
| 7 | **Online meeting** | **"Teams meeting"** switch (Graph `isOnlineMeeting` + `onlineMeetingProvider = teamsForBusiness`). Graph documents that once an event is made online, later changes to `isOnlineMeeting` and `onlineMeetingProvider` are ignored and the meeting stays online. So after the first save, show the switch as on and disabled with a tooltip (copy (estimate)). Offer only providers in the calendar's `allowedOnlineMeetingProviders`, and preselect `defaultOnlineMeetingProvider`. The switch is on by default if Settings › "Add online meeting to all meetings" is checked. Read the join link from `onlineMeeting.joinUrl`; `onlineMeetingUrl` is being deprecated. |
| 8 | **Description** | A rich-text editor (shared with mail compose: bold, italic, underline, lists, link, table, image) with placeholder "Add a description or attach documents" (estimate). Min height 200 px. The online-meeting join block is inserted at the end of the body when the meeting is created and is not user-editable (estimate). |
| 9 | Attachments | Attachment chips above the description. Not supported for group calendars (Graph). |

### 9.4 Date picker
- A Fluent DatePicker popover (the same day grid as the mini month), keyboard-typable in the locale short date format ("10/19/2026"). Invalid text shows the error "Enter a valid date" (estimate copy).
- If the start date changes, the end date shifts to keep the duration.

### 9.5 Time pickers
- A Fluent TimePicker (combobox): the **default increment is 30 minutes** (Fluent default) and the list spans 00:00–24:00. Free typing is accepted and parsed: "9", "930", "9:30p", "21:30". Invalid input shows "Enter a valid time" (estimate).
- Format follows the 12h/24h setting: "9:30 AM" / "09:30".
- The **end-time list shows the duration** relative to the start (estimate format): "9:30 AM (0 min)", "10:00 AM (30 min)", "10:30 AM (1 hr)", "11:00 AM (1.5 hr)", "11:30 AM (2 hr)", … The list starts at the start time.
- When the start time changes, the end time shifts to keep the duration. If the user sets the end ≤ the start on the same date, the end is set to start + the original duration and a field hint "End time was adjusted" appears (estimate).
- The default duration is 30 minutes. If "End appointments and meetings early" is on, the default end is shortened (e.g. 25 or 50 min) (Settings › Events and invitations).

### 9.6 All day and time zones
- **All day** on: the time pickers hide. Start and end are date-only. Storage follows Graph: start and end at midnight in the same zone, **end exclusive** (a one-day event on 10/19 is stored as 10/19 00:00 → 10/20 00:00). The UI shows the inclusive end date.
- Turning All day off restores the previous times, or the next half hour with the default duration (estimate).
- **Time zones** toggle: adds a time-zone dropdown next to the start and next to the end, e.g. "(UTC-05:00) Eastern Time (US & Canada)" (Windows-style display names, matching what Outlook users see; map to IANA internally) (estimate display format). Graph's `dateTimeTimeZone.timeZone` accepts any Windows zone ID and also a documented subset of IANA names (for example `America/New_York`), although create and update may not accept every value. **Send Windows IDs on writes** to be safe. The default is the user's calendar time zone. The event keeps `originalStartTimeZone` / `originalEndTimeZone`. These can hold `tzone://Microsoft/Custom` for legacy custom zones from desktop Outlook; treat that as "unknown zone" and display in the user's zone.
- All-day events: Graph requires start and end at midnight **in the same zone**, for single-day and multi-day events alike.

### 9.7 Show as
Labels: "Free", "Working elsewhere", "Tentative", "Busy", "Away" (Graph `free`, `workingElsewhere`, `tentative`, `busy`, `oof`) (third-party list; Microsoft's availability legend calls `oof` "Out of office"). The default is Busy (documented: an invitation the attendee has not changed shows as Busy). The organizer's choice is copied to attendees, and each attendee's change affects only their own calendar (documented). Graph also has `unknown`: render it (§6.10) but don't offer it in the menu (estimate). **Classic differs:** the classic label for `oof` is "Out of Office".

### 9.8 Private
A checkbox in the Show-as menu (Graph `sensitivity = private`). When on, the event block shows `LockClosed`, delegates and sharees without `canViewPrivateItems` see only "Busy"-style blocks, and a "Private" chip appears next to the title in the editor (estimate).

### 9.9 Reminder
Menu items (values in minutes for `reminderMinutesBeforeStart`; label copy (estimate) except "Don't remind me"):

| Label | Minutes | Label | Minutes |
|---|---|---|---|
| "Don't remind me" | `isReminderOn = false` | "1 hour before" | 60 |
| "At time of event" | 0 | "2 hours before" | 120 |
| "5 minutes before" | 5 | "12 hours before" | 720 |
| "15 minutes before" | 15 | "1 day before" | 1440 |
| "30 minutes before" | 30 | "1 week before" | 10080 |

- The default comes from Settings › Calendar › Events and invitations › "Events you create" › "Default reminder" (path documented). **15 minutes** is the documented default for classic Outlook on the web; new Outlook's page does not state it, so treat 15 as the likely default.
- Only "Don't remind me" is a documented label. The other items and their order are (estimate); Microsoft does not publish the new Outlook list.
- If an event has a non-listed value (e.g. 10 min from another client), show it as an extra checked item "10 minutes before" (estimate).

### 9.10 Categories
Selected categories show as coloured chips under the title, each with an × to remove (estimate placement).

### 9.11 Validation and save rules
- Send requires at least one valid attendee address. Unresolved chips show red underline text and block Send with the inline error "Check the highlighted addresses" (estimate).
- The end must be after the start (all-day: end date ≥ start date).
- **Unsaved changes:** closing the panel or window prompts "Discard changes?" (§9.2).
- **Draft meetings:** if the organizer saves changes without sending, Graph `isDraft = true`. A banner reads "Your changes haven't been sent to attendees." with a "Send update" button (estimate copy).

### 9.12 Deleting and cancelling
- Appointment → "Delete". A meeting you organise → "Cancel" (documented web label): the editor switches into cancel mode with a message box ("Add a message for attendees", estimate copy; adding a message is documented) and "Send", which sends the cancellation (Graph `POST /events/{id}/cancel` with an optional `comment`; organizer only).
- Recurring: choose "This event", "This and all following events", or "All events in the series" (documented labels in Outlook on the web).
- An attendee deleting an invitation → confirm "Delete this event?" with "Delete" and "Delete and send decline" (estimate copy).

### 9.13 Attendee (read-only) view of a meeting
- Same layout, fields disabled. The toolbar shows "Accept", "Tentative", "Decline" (with send options §11.2), "Propose new time", "Join", and "Forward" (only when the organizer allows forwarding). An attendee can still change their own reminder, Show as, categories and private flag (estimate, classic behaviour).

### 9.14 Scheduling Assistant tab
- Shown for work or school accounts only (documented). Graph `getSchedule` and `findMeetingTimes` do not support personal Microsoft accounts.
- **Left column** (240 px, estimate): attendee list. "Add required attendee" and "Add optional attendee" inputs at the top (documented labels). Each row has the avatar, name, role (Required / Optional / Room), and the work location for that day if shared.
- **Right**: a horizontal free/busy grid for the selected date. Time runs left to right in 30-min columns of 48 px (estimate), with a header showing hours and the date navigator (prev/next day, date picker). The first row ("All attendees") is a merged busy summary (estimate). Each attendee row shows blocks using the §6.10 legend patterns (Free outline, Busy solid, Tentative stripes, Away cross-hatch, Working elsewhere dots, Unknown grey stripes), with **outside-of-working-hours in solid grey**.
- **Selected meeting time** is a vertical blue band (`colorBrandBackground` at 20% fill, 2 px `colorBrandStroke1` edges) spanning all rows. Drag the band to move it and drag its edges to resize. It stays in sync with the Event tab.
- **Suggested times** panel (right, 280 px, estimate) lists slots where all required attendees are free, ranked best first. Clicking one sets the time. Data comes from Graph `findMeetingTimes` and free/busy from `getSchedule` (third-party for the panel UI).
  - `getSchedule` (`POST /me/calendar/getSchedule`; least scope `Calendars.ReadBasic`): `availabilityViewInterval` defaults to 30 and accepts 5–1440 minutes, so request it equal to the current time scale. Results come back in UTC unless `Prefer: outlook.timezone` is sent. Each schedule also returns the person's `workingHours`, which drives the grey "outside of working hours" cells. If a slot holds more than 1000 entries, Exchange returns error 5006; show "Too many items to show availability" (estimate copy).
  - `findMeetingTimes` (`POST /me/findMeetingTimes`; least scope `Calendars.Read.Shared`): the default duration is 30 minutes and the default `activityDomain` is `work`. Suggestions are sorted by confidence, then chronologically, and an empty result carries `emptySuggestionsReason`, which the panel should show as a reason line (copy (estimate)).
- Legend at the bottom: swatches with labels "Busy", "Tentative", "Away", "Working elsewhere", "Unknown", "Outside of working hours" (estimate placement).
- Footer: the same primary "Send" / "Save" as the Event tab.

---

## 10. Recurrence

### 10.1 Repeat dropdown (in the editor)
Items are generated from the start date. Example for Monday, Oct 19, 2026. Only "Don't repeat" is a documented label (Microsoft Support, new Outlook). The rest are (third-party) or (estimate): the two newest third-party guides use "Daily", "Every weekday" / "Weekdays", "Weekly on <day>", "Monthly on the <nth> <weekday>", "Yearly on <date>" and "Custom", while an older Outlook on the web guide uses "Every day", "Every week", "Every month", "Every year".
1. "Don't repeat" (default; documented)
2. "Daily" (third-party; older web builds say "Every day")
3. "Every weekday" (the work-week days from settings; Graph `weekly` with those days)
4. "Weekly on Monday"
5. "Monthly on the third Monday" (Graph `relativeMonthly`, `index = third`)
6. "Monthly on day 19" (Graph `absoluteMonthly`)
7. "Yearly on October 19" (Graph `absoluteYearly`)
8. Divider, then "Custom…" which opens the **Repeat** dialog.
- When the start date is the last weekday of its kind in the month, also offer "Monthly on the last Monday" (`index = last`) (estimate).
- When a series exists, the button label becomes the summary, e.g. "Every Monday until 12/21/2026" (estimate copy).

### 10.2 Repeat dialog (Custom)
Modal 440 px wide (estimate). Fields:
| Field | Spec |
|---|---|
| "Start" | Date picker (the series start date; defaults to the event start date). |
| "Repeat every" | Number spinbutton 1–99 (estimate range) + unit dropdown "day(s)" / "week(s)" / "month(s)" / "year(s)". |
| Day picker (weekly) | Seven circular toggle buttons, 32 px (estimate), labelled with one letter in first-day-of-week order. Selected = `colorBrandBackground` fill with white text. At least one must stay selected. The start date's weekday is selected by default. |
| Monthly options | Radio: "On day 19" / "On the third Monday" (and "On the last Monday" when it applies). |
| Yearly options | Radio: "On October 19" / "On the third Monday of October". |
| End | Radio group (estimate labels): **"No end date"**, **"End by"** + date picker (defaults to start + 3 months, estimate), **"End after"** + number + "occurrences". Documented for new Outlook: the Repeat dialog lets you pick a pattern (Daily, Weekly, Monthly or Yearly), then set a new end date or remove the end date entirely. No Microsoft page shows an occurrence-count option in new Outlook. Classic offers all three. Axentra offers all three (Graph `recurrenceRange.type` = `noEnd` / `endDate` / `numbered`). This is mandatory anyway: series created in classic Outlook or on phones arrive as `numbered`, and Axentra must show and save them without converting them to an end date. |
| Summary line | e.g. "Occurs every 2 weeks on Monday and Wednesday until December 21, 2026" (estimate copy). |
| Buttons | "Save" (primary), "Cancel"; plus "Remove recurrence" when editing an existing series (estimate). |

### 10.3 Mapping (Graph ↔ iCalendar RFC 5545)
| Axentra option | Graph `pattern.type` | RRULE |
|---|---|---|
| Every N days | `daily` + `interval` | `FREQ=DAILY;INTERVAL=N` |
| Weekly on days | `weekly` + `daysOfWeek` + `firstDayOfWeek` | `FREQ=WEEKLY;INTERVAL=N;BYDAY=MO,WE;WKST=SU` |
| Monthly on day D | `absoluteMonthly` + `dayOfMonth` | `FREQ=MONTHLY;BYMONTHDAY=D` |
| Monthly on the Nth weekday | `relativeMonthly` + `daysOfWeek` + `index` (`first`…`fourth`, `last`) | `FREQ=MONTHLY;BYDAY=MO;BYSETPOS=3` (or `-1`) |
| Yearly on date | `absoluteYearly` + `month` + `dayOfMonth` | `FREQ=YEARLY;BYMONTH=10;BYMONTHDAY=19` |
| Yearly on the Nth weekday of a month | `relativeYearly` + `month` + `daysOfWeek` + `index` | `FREQ=YEARLY;BYMONTH=10;BYDAY=MO;BYSETPOS=3` |
| End | `endDate` / `noEnd` / `numbered` (`numberOfOccurrences`) | `UNTIL=` / (none) / `COUNT=` |

Graph rules an implementer must follow (Microsoft Learn, `recurrencePattern`):
- Send only the properties a pattern type needs. Graph rejects a supported property sent with an unsupported value. Required per type: `daily` → `type`, `interval`. `weekly` → also `daysOfWeek` and `firstDayOfWeek`. `absoluteMonthly` → `dayOfMonth`. `relativeMonthly` → `daysOfWeek` (`index` is optional and defaults to `first`). `absoluteYearly` → `dayOfMonth` and `month`. `relativeYearly` → `daysOfWeek` and `month`.
- `firstDayOfWeek` defaults to `sunday`.
- For `relativeMonthly` / `relativeYearly` with several `daysOfWeek`, Graph places the occurrence on the first day that satisfies the pattern. Do not translate that to a multi-day `BYDAY` + `BYSETPOS` RRULE without testing, because the two rules can pick different days.
- `recurrenceRange` always needs `startDate` (a date, no time), and it **must equal the date of the event's `start`**. `endDate` is required for type `endDate` and is inclusive; `numberOfOccurrences` is required and must be positive for `numbered`. `recurrenceTimeZone` is optional and defaults to the event's time zone.
- In RFC 5545, `UNTIL` must be in UTC when `DTSTART` has a zone. Convert the Graph `endDate` (a local date, inclusive) to the UTC instant of the last possible occurrence start on that date.

Edge cases:
- **Day 29–31:** whether Exchange skips or clamps months that lack that day is (unverified); Graph's docs don't say. Classic Outlook shows a similar warning and places the occurrence on the last day of the month. Show the hint "Some months have fewer than 31 days. The event will occur on the last day of the month." (estimate). For non-Graph (CalDAV/ICS) accounts, RFC 5545 skips invalid dates. Use `BYMONTHDAY=-1` when the user picks "last day".
- **Feb 29 yearly:** the same hint pattern.
- Editing a series' pattern discards existing exceptions after a confirmation: "Changing the series will remove changes made to individual occurrences." (estimate copy).
- Proposing a new time is not possible for recurring meetings (documented).
- Graph exposes series bookkeeping on the series master in v1.0: `cancelledOccurrences` (occurrence IDs, `$select` only) and `exceptionOccurrences` (`$select` + `$expand`), returned only on a GET of the master by ID. `instances` excludes cancelled occurrences. Use these to reconcile a locally cached series instead of re-expanding it.

---

## 11. Meeting invitations in Mail

### 11.1 Reading pane card for a meeting request (`eventMessage`, iTIP METHOD:REQUEST)
Placed above the message body (layout (estimate); content documented):
```
┌────────────────────────────────────────────────────────────────────────┐
│ 📅 Project sync                                                         │
│ Mon 10/19/2026 9:00 AM – 9:30 AM  ·  Occurs every Monday                │
│ 📍 Room 4 · [Join]                                                      │
│ ⚠ Conflicts with "Design review"  |  ✓ No conflicts                     │
│ [✓ Accept ▾] [? Tentative ▾] [✕ Decline ▾] [🕑 Propose new time ▾]       │
│ [Add a message to <Organizer> …                                    ]   │
├────────────────────────────────────────────────────────────────────────┤
│ Mini day view: 8:00–11:00 around the meeting, existing events shown,   │
│ the invite drawn with Tentative stripes (estimate)                      │
└────────────────────────────────────────────────────────────────────────┘
```
- The message list row of an invitation shows an inline **"RSVP"** button. It offers "Accept" and "Decline", with "More options" ▸ "Tentative", "Respond with note", "Respond without email", "Propose new time".
- When the event was already added to the calendar as tentative (the server auto-processes it), the card reads "Tentatively accepted — not yet responded" (estimate).
- **Updated** requests show a banner "This meeting has been updated" with the changed fields highlighted (estimate).
- **Cancellation** messages (METHOD:CANCEL) show "This meeting has been canceled" and a **"Remove from calendar"** button (estimate copy).
- **Responses** (METHOD:REPLY) received by the organizer show "<Name> accepted / tentatively accepted / declined" with the response note and the tally (estimate).

### 11.2 Response send options
Each of Accept / Tentative / Decline is a split button. The body sends immediately (with the text in "Add a message to …" if present). The chevron offers (labels from Outlook on the web's RSVP menu):
- "Respond with note" → focuses the message box and changes the primary to "Send".
- "Respond without email" → updates the local calendar only (Graph `sendResponse = false`). The organizer can't track this response.
Shortcuts: none (estimate). **Classic differs:** classic offers "Edit the Response before Sending" / "Send the Response Now" / "Do Not Send a Response".

### 11.3 Propose new time
- Hidden when the organizer disallowed proposals (`allowNewTimeProposals = false`) or the meeting is recurring. Work/school accounts only (Microsoft documents that personal and IMAP/POP accounts aren't supported; Axentra mirrors this for Graph accounts).
- The menu offers "I might attend" (= Tentative + proposal) and "No, I won't attend" (= Decline + proposal).
- This opens a **Propose new time** dialog: a Scheduling-Assistant-style grid limited to the organizer and attendees whose free/busy is visible, start and end pickers, then "Done". A compose surface follows with the optional message under "Propose new time", then "Send".
- Graph: `tentativelyAccept` or `decline` with `proposedNewTime` and `sendResponse = true`. Graph returns HTTP 400 if `proposedNewTime` is sent with `sendResponse = false` or on an event whose `allowNewTimeProposals` is false. Both actions return 202 with no body, so re-read the event to refresh `responseStatus`. iCalendar accounts: send METHOD:COUNTER (RFC 5546).
- **Organizer receiving a proposal:** the card shows "New time proposed: Tue 10/20/2026 2:00 PM – 2:30 PM" with **"Accept proposal"** (opens the editor with the new time, then Send update) and **"Decline proposal"** (iCalendar: DECLINECOUNTER) (estimate copy; Microsoft documents the organizer flow only for classic Outlook). Classic labels: "Accept Proposal", "View All Proposals", then "Send Update"; declining a proposal in classic is done with "Delete". Classic cannot accept a proposal from the Reading Pane, only from the opened item.

---

## 12. Reminders

### 12.1 Reminder window ("Reminder" notification style)
- A separate always-on-top top-level window (Linux: a normal window with an `_NET_WM_STATE_ABOVE` hint where the compositor allows it; on Wayland, a regular window plus a desktop notification). It collects event and task reminders in one place (documented). It is **off by default** (third-party; users whose migrated settings had it on start with it on). It is enabled under Settings › General › Notifications › "Notify me about" › Calendar (toggle on, then expand) by choosing the "Reminder" style (Microsoft's wording; a third-party write-up calls it "Reminder popup"). Otherwise reminders use desktop notifications (freedesktop `org.freedesktop.Notifications`). Documented actions in the window: snooze, dismiss, and join a Teams meeting.
- Size 380×460 px, resizable, min 320×200 (estimate). Title "N reminders" (estimate copy).
- List rows (64 px, estimate): a colour bar, the title (14 px semibold), the time status ("Starts in 5 min", "Now", "10 min overdue", "Tomorrow at 9:00 AM"), the location (12 px secondary), a **"Join"** primary button when it is an online meeting, a **"Snooze" ▾** split button, and a **"Dismiss"** button.
- Snooze menu (estimate values; Microsoft lists no durations, and a user Q&A thread asking for snoozes longer than 2 hours implies 2 hours is the current maximum): "5 minutes", "10 minutes", "15 minutes", "30 minutes", "1 hour", "2 hours". For events that haven't started, prepend "5 minutes before start" and "At start time" (estimate).
- Footer: "Dismiss all" (estimate; no Microsoft or third-party source mentions it for new Outlook).
- Keyboard (estimate, borrowed from classic, which documents Alt+S = snooze, Alt+D = dismiss, Alt+O = open item): bind the same keys inside the window.
- Ordering: overdue first (oldest first), then upcoming by start time.
- Setting "Automatically dismiss reminders for past events" (documented). It appears only when the notification style is "Reminder". When on, reminders for events that have ended are removed automatically.
- A sound plays on a new reminder when the sound option is on (documented as a choice of whether to play a notification sound; the exact label "Play a sound" is (unverified)). Use the freedesktop sound theme event `message-new-instant` (estimate).
- Graph: `snoozeReminder(newReminderTime)` and `dismissReminder`, user calendars only (Outlook doesn't support reminders for group events). To list due reminders across all calendars in one call, use `GET /me/reminderView(startDateTime=…,endDateTime=…)` (Graph v1.0 function; (unverified) in this pass, check before relying on it) and fall back to expanding `calendarView` locally.
- Window focus: it never steals keyboard focus from a compose window when it appears (estimate). The taskbar entry flashes via the urgency hint.

### 12.2 Reminder scheduling
- Compute fire times locally from the expanded occurrences in the next 48 h (estimate window). Re-arm on sync deltas, time-zone change, resume from suspend, and clock jumps (detect wall-clock vs monotonic drift greater than 60 s).
- After suspend, fire overdue reminders immediately as "N min overdue" (unless auto-dismissed).

---

## 13. Print and Share

### 13.1 Print dialog (Home › Print; Ctrl+P (estimate))
Preview dialog with a settings column on the left (280 px, estimate) and a live preview on the right:
- "Calendar" dropdown (which calendar).
- "View": "Day", "Work week", "Week", "Month".
- Time range: a start date (plus an end date for multi-range) (estimate).
- "Mini month" toggle (adds a mini month to the page header) and a "Detailed agenda" toggle (adds a per-day agenda list). Microsoft documents the choice of a mini month and a detailed agenda; the exact toggle labels are (estimate).
- Footer: "Print" (primary) → the system print dialog (GTK via the webview's print). "Cancel". The preview updates live with each choice (documented).
- Print CSS: white background, black text, event fills at 30% tint, Show-as patterns retained, no shadows (estimate).
- **Single events** print through the event "…" › "Print". Roadmap RM551199 (new Outlook and Teams, GA March 2026) adds basic print options for calendar events. Its documented option is an attendee-list checkbox that switches between printing all attendees and a shortened list of up to 10. The label "Show attendee list" is (estimate). This option belongs to the event print flow, not the calendar-range dialog above.

### 13.2 Sharing and permissions dialog
Modal 560 px (estimate):
- An input "Enter an email address or contact name" (estimate copy), a permission dropdown, and "Share".
- Permission levels (documented labels): "Can view when I'm busy", "Can view titles and locations", "Can view all details", "Can edit", "Delegate". Classic Outlook on the web restricts "Can edit" and "Delegate" to people inside the organisation. The new Outlook article lists them without that note but says sharing may be limited by organisation settings, so build the dropdown from Graph rather than hard-coding it. Only the owner can share (Graph `canShare`).
- **Graph mapping** (`calendarPermission.role`; label ↔ role pairing is (estimate) but follows the role descriptions): "Can view when I'm busy" = `freeBusyRead`, "Can view titles and locations" = `limitedRead`, "Can view all details" = `read`, "Can edit" = `write`, "Delegate" = `delegateWithoutPrivateEventAccess`, with a "Delegate can view private events" checkbox switching to `delegateWithPrivateEventAccess` (estimate UI). `custom` (set by classic Outlook) shows as "Custom" and is read-only in Axentra (estimate). Offer only the roles in that row's `allowedRoles`.
- Graph constraints: list, create, update and delete work only as the calendar owner. After creation only `role` can be updated; other fields need delete-and-recreate. The "My organization" row has a null address and `isRemovable = false`, so show it without a remove button. `isInsideOrganization` marks external people.
- Below that, the existing shares list: avatar, name, permission dropdown, and a remove (Delete icon) button.
- "Publish a calendar" lives in Settings › Calendar › Shared calendars (documented): pick a calendar and a detail level, then "Publish" / "Unpublish". It yields an HTML link (view in a browser) and an ICS link (subscribe or import); both are read-only.

---

## 14. Month view

### 14.1 Grid
- A weekday header row of 32 px (estimate) with full names ("Sunday") when columns are ≥ 120 px wide, otherwise abbreviated ("Sun") (estimate).
- The body has 5 or 6 week rows depending on the month (estimate). Rows share the height equally. Minimum row height is 96 px, and the grid scrolls when the window is shorter (estimate).
- **Week numbers:** when "Show week numbers" is on, a 32 px left column shows the week number (12 px, `colorNeutralForeground4`) following "Start the first week of the year on" ("First day of year", "First four-day week", "First full week"). Clicking it opens that week in Week view (estimate).
- Days outside the current month: bg `colorNeutralBackground2`, date number `colorNeutralForeground4` (estimate).
- Split view in Month (since 2024): one month grid per calendar, side by side (estimate layout as §6.12).

### 14.2 Cell anatomy
```
┌──────────────────────────┐
│ 19                       │  ← date label, 12 px; "Oct 1" on the 1st of each month (estimate)
│ ▌Team offsite (all day)  │  ← all-day / multi-day bar (filled, spans cells)
│ • 9:00 AM Project sync   │  ← timed event: colour dot + start time + title
│ • 1:30 PM 1:1 with Ana   │
│ +2 more                  │  ← overflow link
└──────────────────────────┘
```
- Date label at the top-left with 6 px padding (estimate). **Today**: the number sits in a 24 px `colorBrandBackground` circle with white semibold text, and the whole cell gets a `colorBrandBackground2` tint (estimate).
- **All-day and multi-day** events are filled bars (calendar fill colour, primary text) spanning the cells, 20 px tall with a 2 px gap and radius 4 px. They break at week rows with continuation notches. They are laid out above the timed events and keep a consistent vertical lane across days (estimate).
- **Timed** events are single lines, 20 px: a colour dot 8 px (Show as: hollow dot for Free, striped for Tentative) (estimate), then the start time in 12 px semibold, then the title in 12 px regular with ellipsis.
- **Overflow:** compute how many lanes fit. If events don't fit, the last visible lane becomes "+N more" (12 px semibold, `colorBrandForeground1`). Clicking it opens a **day popover** (320 px wide, `shadow16`) titled with the date (e.g. "Monday, October 19") that lists all events. Each item behaves like an event (click = peek, double-click = open). The popover also has a "New event" link (estimate copy).
- Interactions: click an empty cell area → select the day and open quick-create as an all-day event (estimate). Double-click → full editor, all-day (estimate). Click the date label → Day view for that date (estimate). Drag events between cells to change the date, keeping the time. Drag a multi-day bar edge to change the span (estimate).
- Keyboard: arrows move the selected day, Enter opens quick-create, Tab cycles events in the selected cell (estimate).

---

## 15. Keyboard shortcuts

### 15.1 New Outlook for Windows (Microsoft documentation, re-checked 2026-10-19)
| Action | Keys | Note |
|---|---|---|
| Go to Calendar | Ctrl+2 | Documented. (Ctrl+1 Mail, Ctrl+3 Copilot, Ctrl+4 People.) |
| New calendar item | Ctrl+N | Documented. |
| Open selected item | Enter | Documented. |
| Delete selected item | Delete | Documented. |
| Previous / next time period | Ctrl+Alt+Left / Ctrl+Alt+Right | Documented. |
| Previous / next week | **Axentra: Alt+Up / Alt+Down** | Microsoft's new Outlook table lists previous = Alt+Down and next = Alt+Up, the reverse of classic, which documents Alt+Up = same day previous week and Alt+Down = next week. The new Outlook entry looks swapped, so Axentra follows classic (estimate). New Outlook also documents Alt+Down as "open split buttons"; when focus is on a split button, that meaning wins. |
| Previous / next month | **Axentra: Alt+PageUp / Alt+PageDown** | Same inversion: the new Outlook table lists previous = Alt+PageDown and next = Alt+PageUp; classic documents Alt+PageUp = previous month. Axentra follows classic (estimate). |
| Go to today | Alt+Shift+Y or Home | Both documented. |
| Day view | Ctrl+Alt+1 | Documented. |
| Work week view | Ctrl+Alt+2 | Documented. |
| Week view | Alt+Minus (documented); Ctrl+Alt+3 (estimate) | Microsoft's new Outlook table lists "Ctrl+Alt+2 / Alt+-" for full Week and Ctrl+Alt+2 again for Work week, so Ctrl+Alt+2 cannot serve both. Ctrl+Alt+3 is classic's documented Week key and is the only free, consistent choice. |
| Month view | Ctrl+Alt+4 or Alt+= | Documented. |
| Three-day view | Alt+3 | Documented. |
| Show N days starting today | Alt+1 … Alt+7 | (estimate) Not in the new Outlook table; classic documents Alt+1…Alt+9 and Alt+0 for 1–10 days. Axentra caps at 7 to match the Day menu. |
| Move between calendar regions | F6 or Ctrl+Shift+Tab | Documented. |
| Next / previous event or area | Tab / Shift+Tab | Documented. |
| Open the keyboard shortcut list | ? | Documented (new Outlook general table). |
| Key tips | Alt | Documented. |
| Print | Ctrl+P | (estimate) Not in the new Outlook table. |

### 15.2 Outlook on the web shortcut set (offer it as an alternative keymap)
Microsoft documents that Outlook on the web lets users pick a shortcut set under Settings › General › Accessibility › Keyboard shortcuts: "Outlook", "Outlook.com", "Gmail", "Yahoo Mail", or off. Axentra mirrors that setting at the same path and ships the "Outlook" web set below plus the §15.1 desktop set (the default). The other sets are out of scope for v1. The table is the documented web "Outlook" set.
| Action | Keys |
|---|---|
| Go to Calendar | Ctrl+Shift+2 |
| New event | N |
| Previous / next period | Shift+Left / Shift+Right |
| Previous / next week, month | Same table and same inversion as §15.1 (Alt+Down/Up, Alt+PageDown/PageUp) |
| Go to today | Alt+Shift+Y |
| Three-day view | No shortcut |
| Day / Work week / Week / Month | Shift+Alt+1 / 2 / 3 / 4 |
| Move between regions | Ctrl+F6 / Ctrl+Shift+F6 |
| Save (event form) | Ctrl+S |
| Send (event form) | Ctrl+Enter or Alt+S |

### 15.3 Classic Outlook differences (reference only)
Ctrl+Shift+A new appointment from anywhere, Ctrl+Shift+Q new meeting, Ctrl+G go to date (in the calendar; inside an open appointment Ctrl+G opens recurrence), Alt+1…Alt+9 / Alt+0 one to ten days, Alt+Minus Week, Alt+= Month, Ctrl+Alt+3 Week, Ctrl+Alt+5 schedule view, Alt+Home / Alt+End start / end of week (date navigator), Home / End start / end of work hours (Day and Week views), Ctrl+Comma / Ctrl+Period previous / next appointment, Alt+S / Alt+D snooze / dismiss in the reminder window. Axentra binds **Ctrl+G = Go to date** (opens the title flyout) as an extension (estimate).

---

## 16. Calendar settings (Settings › Calendar)

### 16.1 Calendar appearance
Section name: Microsoft's 2025–2026 new Outlook article calls this page "Calendar appearance" (View › Calendar settings › Calendar appearance). The Outlook on the web settings article and Ablebits call it "View". Axentra uses "Calendar appearance".

| Setting | Options | Default |
|---|---|---|
| "Show the first day of the week as" | Sunday … Saturday (documented; it sets the first day in Week view) | Locale-derived: **Sunday for en-US** (Graph `firstDayOfWeek` default `sunday`). Monday for most of Europe (CLDR) (estimate source mapping). |
| "Show work week as" | Day checkboxes (documented) | Mon–Fri (estimate) |
| "Show hours in" / Time scale | Settings documents "Show hours in" with 15 or 30 minutes; the View tab's "Time scale" offers 5, 6, 10, 15, 30, 60. Axentra offers the full View-tab list in both places. | 30 |
| "Show week numbers" | Checkbox (documented; affects Month view) | Off (estimate) |
| "Start the first week of the year on" | "First day of year", "First four-day week", "First full week" (documented choices; exact strings (estimate)) | Locale (en-US: First day of year) (estimate) |
| "Turn on the birthday calendar" | Checkbox (documented) | Off (estimate) |
| "Dynamic column widths" | Checkbox (third-party) | Off (estimate) |
| Time zones | Primary zone dropdown + "Add time zone" (the "Other time zones" list + a label field; documented), with Move up / Move down (estimate) | System zone |
| Date format / Time format | Locale formats; time 12h or 24h. The quick Settings pane exposes time zone, date format, time format and first day of week (documented). | From the system locale; for Graph accounts, read `mailboxSettings.dateFormat` / `timeFormat` / `timeZone` |

### 16.2 Work hours and location
Path: Settings › Calendar › "Work hours and location" (documented). Per-day checkboxes, a start and end time per day, and a location per day: "Remote", an office (searched; building choices appear only if the organisation has set up a buildings directory), or "No location". Then "Share office location details" (when off, others see only in-office or remote) and "Share your location", then "Save" (labels documented). It requires a work or school account with a qualifying Microsoft 365 subscription (documented). Default 8:00 AM–5:00 PM, Mon–Fri: this is the documented Exchange default used by Graph `findMeetingTimes` (in the mailbox time zone), so use it when the server returns nothing. For accounts without the modern experience, a single "Working hours" start/end plus a time zone (Graph `mailboxSettings.workingHours`).

### 16.3 Events and invitations
- "Events you create" › "Default reminder" (15 min).
- "End appointments and meetings early" + duration choices.
- "Add online meeting to all meetings".
- "Delete invitations and responses that have been updated".
- "Delete notifications about forwarded events".
- "Save declined events" › "Show declined events on your calendar" (documented). Off by default. Work or school accounts only. Kept declined events show as Free (§6.9). Tenant admins can disable the feature (`Set-CalendarSettings -EnablePreserveDeclinedMeetings $false`, third-party: Petri); if the server reports it disabled, hide the toggle (estimate).
- Default meeting duration: 30 min (estimate as default; classic web had "Default meeting length in minutes" with 15-minute steps or a custom value; new Outlook documents no such setting).

### 16.4 Weather · Events from email · Shared calendars
- Weather: "Show weather" / "Hide weather", "Fahrenheit" / "Celsius", up to 5 locations. Microsoft's page also says that changing the weather location isn't yet available in the new version, which contradicts the five-location statement. Treat multi-location as (unverified) and ship one location in v1.
- Events from email: "Automatically add events to my calendar from email", "Mark events as private so only I can see them", plus per-type checkboxes. Out of scope for v1 except the settings stubs (estimate).
- Shared calendars: "Share a calendar", "Publish a calendar".

### 16.5 Notifications (Settings › General › Notifications)
Under "Notify me about": a Calendar toggle (documented). Expanding it shows the notification style (desktop notification vs "Reminder"; documented wording, called "Reminder popup" by a third party), the sound choice (exact label (unverified)), and "Automatically dismiss reminders for past events" (documented; shown only with the "Reminder" style). On the web, notifications only appear while the page is open (documented); Axentra, as a desktop app, has no such limit.

---

## 17. Accessibility

- Time grid: the root is `role="grid"` with `aria-label="Calendar, week of October 18, 2026"` (estimate). Each day column is `role="gridcell"`-hosting `role="group"` with `aria-label="Monday, October 19, 2026"`. Events are `role="button"`. Slot selection uses `aria-selected` on a virtual slot element.
- Event accessible name, in order (Microsoft documents that screen readers announce title, time, date, location, organizer): "<Title>, <start> to <end>, <weekday, date>, <location>, organizer <name>, <Show as>, <Recurring>, <Private>, <response status>".
- The peek card is `role="dialog"` with `aria-modal="false"`, labelled by the title. Quick-create and the editor are `role="dialog"`.
- Announce view changes and navigation through a polite live region: "Week of October 18 to 24, 2026" (estimate copy).
- Respect `prefers-reduced-motion`: no fade or scale on popovers, no smooth auto-scroll.
- Forced colours / high contrast: event fills → `Canvas`, border 1 px `CanvasText`, Show-as patterns replaced by text suffixes ("(Tentative)") because `background-image` patterns are dropped (estimate). The current-time line uses `Highlight`.
- Every colour meaning (calendar, Show as) has a non-colour cue: the pattern, an icon, or text in the peek card.
- Hit targets ≥ 24×24 px (WCAG 2.2 AA). Exception: event blocks shorter than 24 px stay keyboard-reachable through Tab.

---

## 18. Loading, empty, offline, and error states

| State | Behaviour |
|---|---|
| First load (no cache) | Render the grid skeleton right away (gutter, headers, shading). Show event placeholders as `colorNeutralStencil1` shimmer bars, 3 per day at random hours. Shimmer stops after 10 s with an inline error (estimate). |
| Navigating to an uncached range | Keep the old events faded at 50% for at most 300 ms, then show the skeleton (estimate). Prefetch ±1 range in the background. |
| Empty range | No empty-state illustration in the time grid (an empty grid is valid). Month view: nothing. Day popover: "No events" (estimate). |
| No calendars checked | Impossible (§3.4). |
| Offline | A banner under the surface header: "You're offline. Changes will sync when you're back online." (estimate copy). Create, edit and RSVP are queued. Free/busy and room search show "Unavailable offline". |
| Sync conflict (changeKey mismatch) | Toast "This event was changed elsewhere. Reload to see the latest version." with a "Reload" button (estimate). |
| Permission denied (read-only) | Editing fields are disabled. Banner "You have view-only access to this calendar." (estimate). |

---

## 19. Data model hints (Rust)

```rust
pub enum CalendarView { Day { days: u8 /*1..=7*/ }, WorkWeek, Week, Month }
pub enum ShowAs { Free, WorkingElsewhere, Tentative, Busy, Away /*oof*/, Unknown }
pub enum Sensitivity { Normal, Personal, Private, Confidential }
pub enum ResponseKind { None, Organizer, TentativelyAccepted, Accepted, Declined, NotResponded }
pub enum EventKind { SingleInstance, Occurrence, Exception, SeriesMaster }
pub enum WeekIndex { First, Second, Third, Fourth, Last }
pub enum RecurrencePatternKind {
    Daily, Weekly { days: WeekdaySet, first_day: Weekday },
    AbsoluteMonthly { day: u8 }, RelativeMonthly { days: WeekdaySet, index: WeekIndex },
    AbsoluteYearly { month: u8, day: u8 }, RelativeYearly { month: u8, days: WeekdaySet, index: WeekIndex },
}
pub enum RecurrenceEnd { NoEnd, EndDate(NaiveDate), Numbered(u32) }
pub struct TimeScale(u8); // 5,6,10,15,30,60
pub struct LayoutBox { event_id: EventId, day: NaiveDate, top_px: f32, height_px: f32, col: u16, col_span: u16, cols: u16 }
```
- **Show as ↔ iCalendar** (MS-OXCICAL). `X-MICROSOFT-CDO-BUSYSTATUS` documents only `FREE`, `TENTATIVE`, `BUSY` and `OOF`. There is **no documented token for Working elsewhere**. `TRANSP` export: Free and Working elsewhere → `TRANSPARENT`; Tentative, Busy and Away → `OPAQUE`. On import, use `X-MICROSOFT-CDO-BUSYSTATUS` when present; otherwise `TRANSP:TRANSPARENT` → Free and `OPAQUE` → Busy. On export of `workingElsewhere`, write `TRANSP:TRANSPARENT` and omit the busy-status property, or write `X-MICROSOFT-CDO-BUSYSTATUS:WORKINGELSEWHERE` only if interop testing shows Exchange accepts it (unverified). So a Working-elsewhere event round-trips through a third-party ICS feed as Free.
- Map `Private` ↔ `CLASS:PRIVATE`. Graph `personal` and `confidential` have no single obvious RFC 5545 value; follow the MS-OXCICAL `CLASS` section before shipping (unverified mapping).
- **Attendee type ↔ iCalendar** (MS-OXCICAL). Export: required → `ROLE` omitted (Exchange omits it; RFC 5545's default is `REQ-PARTICIPANT`), optional → `ROLE=OPT-PARTICIPANT`, resource → `ROLE=NON-PARTICIPANT` + **`CUTYPE=RESOURCE`** (Exchange does not emit `CUTYPE=ROOM`). Import, first match wins: `ROLE=CHAIR` or `REQ-PARTICIPANT` → required; `OPT-PARTICIPANT` → optional; `CUTYPE=RESOURCE` or `CUTYPE=ROOM` → resource; `ROLE=NON-PARTICIPANT` → resource; anything else → required. Keep `CUTYPE=ROOM` from third-party feeds so the UI can label the row "Room".
- PARTSTAT ↔ Graph response: `ACCEPTED` ↔ `accepted`, `TENTATIVE` ↔ `tentativelyAccepted`, `DECLINED` ↔ `declined`, anything else ↔ `none` / `notResponded` (MS-OXCICAL import table).

---

## 20. Implementation notes (must not miss)

1. **Time zones end to end.** Store instants as UTC plus the original zone. **Graph returns `start` / `end` in UTC by default** (with `timeZone: "UTC"`). Send `Prefer: outlook.timezone="<zone>"` to get them in another zone; it accepts Windows IDs and the documented IANA subset. The *original* zones come back separately in `originalStartTimeZone` / `originalEndTimeZone`, usually as Windows IDs and occasionally as `tzone://Microsoft/Custom`. Recommended: request UTC (no Prefer header), convert locally, and keep the original zone only for display and editing. Keep a Windows↔IANA map (CLDR `windowsZones`) and use `chrono-tz` for wall-clock maths. Never position events with naive local times.
2. **All-day end is exclusive** in storage and inclusive in the UI. Off-by-one bugs here show up in the Month bars, the all-day row, and the peek text. Graph requires all-day start and end at midnight in the same zone. Whether an all-day event fetched in UTC comes back at UTC midnight or shifted (for example 04:00Z) is (unverified). Never derive the date from the UTC instant: when `isAllDay` is true, take the calendar date from `start.dateTime` read in `originalStartTimeZone` (or fetch with the user's zone in `Prefer`), and add an integration test for it.
3. **Use `calendarView` (start/end window) for display, not `events`.** `events` returns single instances and series masters, not occurrences. `startDateTime` / `endDateTime` are read from the offset in the value, are not affected by the `Prefer` header, and count as UTC when no offset is given. Bodies come back as HTML unless `Prefer: outlook.body-content-type="text"`. Use `calendarView/delta` for incremental sync of a window. Prefetch ±1 range, and expand recurrences locally only for ICS/CalDAV accounts (RRULE engine + EXDATE + RECURRENCE-ID overrides).
4. **Overlap layout runs per day and is pure.** Make it a pure function `fn layout(day_events) -> Vec<LayoutBox>`, memoised by (day, event-set hash, slot height). Dioxus should re-render only the changed day columns. The current-time line should be its own component updating once a minute so the grid never re-renders on the tick.
5. **Pointer capture for drag/resize** needs `setPointerCapture` through a small JS shim (`eval`) in the Dioxus webview. Throttle pointermove to `requestAnimationFrame`, and snap in Rust from `(y_px / slot_h) * scale_minutes`.
6. **Slot height is a CSS variable.** Changing the time scale must keep the time under the viewport centre fixed when re-scrolling.
7. **DST days** have 23 or 25 real hours but 24 rendered rows. Test 2026-03-08 and 2026-11-01 (US) and 2026-03-29 / 2026-10-25 (EU).
8. **Midnight rollover and sleep/resume** must update "today" everywhere (mini month, headers, current-time line, reminders).
9. **First day of week** affects the mini month, the Week view, week numbers, the Repeat day-picker order, and `WKST`/`firstDayOfWeek` on weekly recurrences. Keep them all in sync from one setting.
10. **Split view is disabled with fewer than 2 checked calendars.** At least one calendar must remain checked.
11. **Send vs Save** depends on attendees being present. Organizer edits default to "Send update". Respect `isDraft`, and never silently send updates after a drag (always ask, §7.3).
12. **Response semantics:** `sendResponse=false` means the organizer never learns the answer. `proposedNewTime` requires `sendResponse=true` and `allowNewTimeProposals=true`, and is impossible for recurring meetings. Hide the UI, don't merely error.
13. **Private items** on shared or delegated calendars must be redacted *before* they reach the UI layer (no title in DOM or accessibility tree) unless `canViewPrivateItems`.
14. **Category colour overrides calendar colour** for fill and bar (estimate rule). Events with several categories use the first for colour and list all in the peek card.
15. **No product artwork:** use Fluent System Icons (MIT) such as `Video` for online meetings. Never a Teams logo. The toggle label "Teams meeting" is text only, shown only when the provider is `teamsForBusiness`.
16. **Board view is retired** (2023; the exact month is (unverified)). Do not build it. Schedule view and List view are not in new Outlook either (documented).
17. **Reminders survive restarts.** Persist snooze and dismiss state locally and reconcile with server `dismissReminder`/`snoozeReminder`. Desktop notifications need an app ID and the `.desktop` file for actions (Join / Snooze / Dismiss buttons).
18. **Fonts:** Segoe UI is unavailable on Linux. Bundle or fall back as in §2.2, and verify that numerals stay tabular in the gutter (`font-variant-numeric: tabular-nums`).
19. **Locale formatting:** generate every date/time string with ICU (`icu4x` date/time and interval formatters). Never concatenate "Month Day, Year" by hand. Respect the 12h/24h override independently of locale.
20. **Performance budget:** a Week view with 300 events must lay out in < 8 ms and paint in < 16 ms. Virtualise Month day popovers and the Scheduling Assistant rows (beyond 50 attendees).
21. **Stable IDs.** Graph event IDs change when an event moves to another calendar unless every request sends `Prefer: IdType="ImmutableId"`. Send it on every calendar call and key the local cache on that ID. Use `iCalUId` only to match occurrences across calendars; it differs per occurrence of a series.
22. **Optimistic concurrency.** Every event carries `changeKey`. Keep it with the cached copy and treat a mismatch on PATCH as the "changed elsewhere" case in §18.
23. **Graph permissions (delegated).** Least-privileged scopes, from the Microsoft Learn API pages unless marked:

    | Feature | Scope |
    |---|---|
    | Read own calendars and `calendarView` | `Calendars.ReadBasic` (documented for `calendarView`; it omits bodies and attachments (unverified)) or `Calendars.Read` |
    | Create, edit, delete, RSVP (`accept` / `tentativelyAccept` / `decline`), `cancel`, `forward`, `dismissReminder`, `snoozeReminder` | `Calendars.ReadWrite` (documented for `tentativelyAccept`; assumed for the others) |
    | Shared and delegated calendars of other people | `Calendars.Read.Shared` / `Calendars.ReadWrite.Shared` (both names appear on the `findMeetingTimes` page; that they are the right scopes for reading and writing other people's calendars is (unverified)) |
    | Scheduling Assistant free/busy (`getSchedule`) | `Calendars.ReadBasic`; work or school only |
    | Suggested times (`findMeetingTimes`) | `Calendars.Read.Shared`; work or school only |
    | Sharing dialog (`calendarPermissions`) | `Calendars.ReadWrite` (unverified; owner only) |
    | Working hours, time zone, date and time format (`mailboxSettings`) and the category master list | `MailboxSettings.Read` / `MailboxSettings.ReadWrite` (unverified) |
    | Room finder (`places`) | `Place.Read.All` (unverified) |

    Request the work-only scopes only for work or school accounts, because personal accounts fail consent for some of them.
24. **Group calendars** (Microsoft 365 groups) differ (documented): no attachments on their events, no reminders, no accept/decline (Outlook auto-accepts for the group), and they cannot be placed in a `calendarGroup`. Hide those controls for group calendars.

---

## 21. Open questions

Status after the 2026-10-19 fact-check:

1. **Partly resolved.** Repeat presets: only "Don't repeat" is documented. The newest third-party guides use "Daily" (older web builds: "Every day"), so the spec now uses "Daily". End options: Microsoft documents that new Outlook's Repeat dialog sets an end date or removes it, and no source shows an occurrence count. Axentra keeps "End after N" for round-tripping `numbered` series. Exact strings remain open.
2. **Open.** Pixel slot height, hour height and day-header layout are undocumented; every value is an estimate.
3. **Partly resolved.** Filter is documented on the View tab with "Hide declined events". A 2023 third-party source lists top-level entries All / Appointments / Meetings / Categories / Show as / Recurrence / Privacy. Sub-items remain (estimate).
4. **Open.** Snooze durations and "Dismiss all" have no Microsoft or third-party confirmation. Microsoft documents only that the window can snooze, dismiss and join.
5. **Resolved as far as sources allow.** Microsoft's new Outlook table does list Ctrl+Alt+2 for both Week and Work week, but it also lists Alt+Minus for Week. Axentra uses Alt+Minus (documented) plus Ctrl+Alt+3 (classic, estimate). The week and month navigation keys in that table are the reverse of classic; Axentra follows classic.
6. **Resolved.** Microsoft's availability page says its legend (pink cross-hatch for Out of office) applies to new Outlook's calendar and Scheduling Assistant. Purple is classic only.
7. **Resolved (behaviour); open (styling).** Documented: Settings › Calendar › Events and invitations › "Save declined events" › "Show declined events on your calendar", off by default, work or school only. Kept events show as Free and can be hidden with View › Filter › "Hide declined events". Visual styling is undocumented.
8. **Resolved (range); open (labels).** New Outlook's Day menu covers 1–7 days (two third-party sources). The "up to 10" figure is classic's Alt+0 shortcut. Menu item strings remain (estimate).
9. **New.** Whether Exchange skips or clamps monthly recurrences on day 29–31 in short months.
10. **New.** The correct `X-MICROSOFT-CDO-BUSYSTATUS` value (if any) for Working elsewhere in ICS export, and the `CLASS` mapping for `personal` / `confidential`.
11. **New.** Whether the new Outlook settings page is labelled "Calendar appearance" or "View" in the current build (Microsoft's own pages disagree).
12. **New.** The exact Graph scope names for shared calendars, `calendarPermissions`, `mailboxSettings`, master categories and `places` (marked (unverified) in §20 item 23).

---

## Sources

- https://support.microsoft.com/en-us/office/keyboard-shortcuts-for-outlook-3cdeb221-7ae5-4c1d-8c1d-9e63216c1efd
- https://support.microsoft.com/en-us/article/015951f7-6657-4864-b7a8-f2cfa296554d
- https://support.microsoft.com/office/calendar-settings-in-outlook-on-the-web-12cba5a4-4f95-4d00-bfc3-b694aa67ac8f
- https://support.microsoft.com/en-us/outlook/calendar/change-how-you-view-your-outlook-calendar
- https://support.microsoft.com/en-US/outlook/calendar/change-the-calendar-time-scale-in-outlook
- https://support.microsoft.com/en-us/office/manage-your-availability-in-your-outlook-calendar-fbbf6d5c-bdb8-4018-a2e0-2e2f60642117
- https://support.microsoft.com/en-us/outlook/calendar/schedule-a-meeting-or-event-in-outlook
- https://support.microsoft.com/topic/5c9877bc-ab91-4a7c-99fb-b0b68d7ea94f
- https://support.microsoft.com/en-gb/office/create-a-recurring-meeting-in-outlook-932e5256-9bfb-46b2-87b5-382efd8bb893
- https://support.microsoft.com/en-us/office/extend-a-recurring-meeting-in-outlook-364a6e1b-5d2a-403f-9eb4-8c79530b5aa7
- https://support.microsoft.com/en-US/outlook/create-modify-or-delete-a-meeting-request-or-appointment-in-outlook-on-the-web
- https://support.microsoft.com/en-us/outlook/calendar/propose-a-new-meeting-time-in-outlook
- https://support.microsoft.com/en-au/outlook/rsvp-instantly-in-outlook-on-the-web
- https://support.microsoft.com/en-us/office/use-a-screen-reader-to-read-and-reply-to-a-meeting-request-in-outlook-a71a4714-a1a9-43f9-8609-95457b9d9597
- https://support.microsoft.com/en-us/office/basic-tasks-using-a-screen-reader-with-the-calendar-in-outlook-857e6fd5-72b0-409d-973d-c8dc6b77a817
- https://support.microsoft.com/outlook/calendar/add-or-delete-notifications-or-reminders-in-outlook
- https://support.microsoft.com/en-us/outlook/add-a-calendar-in-outlook-com-or-outlook-on-the-web
- https://support.microsoft.com/en-us/outlook/import-or-subscribe-to-a-calendar-in-outlook-com-or-outlook-on-the-web
- https://support.microsoft.com//outlook/share-your-calendar-in-outlook-on-the-web
- https://support.microsoft.com/en-au/office/print-contacts-messages-or-other-outlook-items-d2c0b12b-e308-41ce-9016-a3089ebdbe38
- https://support.microsoft.com/office/set-your-work-hours-and-location-in-outlook-af2fddf9-249e-4710-9c95-5911edfd76f6
- https://support.microsoft.com/outlook/attendees-can-see-others-responses-to-a-meeting-invitation
- https://learn.microsoft.com/en-us/graph/api/resources/event?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/resources/recurrencepattern?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/resources/calendar?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/resources/outlookcategory?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/resources/workinghours?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/event-tentativelyaccept?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-oxcical/cd68eae7-ed65-4dd3-8ea7-ad585c76c736
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/alias/lightColor.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/alias/darkColor.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/alias/lightColorPalette.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/alias/darkColorPalette.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/brandColors.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/colors.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/fonts.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/borderRadius.ts
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/utils/shadows.ts
- https://github.com/microsoft/fluentui/blob/master/packages/react-components/react-calendar-compat/library/src/components/CalendarDayGrid/useCalendarDayGridStyles.styles.ts
- https://github.com/microsoft/fluentui/blob/master/packages/react-components/react-timepicker-compat/library/src/components/TimePicker/useTimePicker.tsx
- https://fluent2.microsoft.design/
- https://www.rfc-editor.org/rfc/rfc5545
- https://www.rfc-editor.org/rfc/rfc5546
- https://mc.merill.net/message/MC793320 (Split view in Month view, 2024)
- https://m365admin.handsontek.net/outlook-calendar-board-view-retirement/ (board view retirement; returned HTTP 403 during the 2026-10-19 check)
- https://mc.merill.net/message/RM501453 (multi-select categories)
- https://m365admin.handsontek.net/?p=12008 ("None" colour category)
- https://supersimple365.com/reminders-window-in-the-new-outlook-for-windows-preview/ (third-party)
- https://learn.microsoft.com/en-us/answers/a/9479684 (user report on reminder snooze limits)
- https://www.ablebits.com/office-addins-blog/outlook-calendar-view/ (third-party)
- https://www.ablebits.com/office-addins-blog/change-outlook-time-zone/ (third-party)
- https://nocal.app/help/outlook-calendar/create-recurring-meetings (third-party)
- https://www.uvm.edu/it/kb/article/show-as-status-setting-in-new-calendar-events/ (third-party)
- https://www.slipstick.com/outlook/calendar/understanding-outlooks-calendar-patchwork-colors (third-party, classic)
- https://kb.uwm.edu/47156 (third-party, Outlook on the web calendar overview)
- https://support.slcc.edu/TDClient/105/Portal/KB/PrintArticle?ID=4731 (third-party, New/Web Outlook calendars)
- https://wisechecker.com/classic-outlook-new-outlook-quick-event-creation/ (third-party)

### Added by the 2026-10-19 fact-check
- https://support.microsoft.com/en-us/office/show-a-declined-meeting-on-my-calendar-in-outlook-24e81b9b-3906-4c02-8b63-f97b02161763 ("Save declined events", "Hide declined events" filter)
- https://petri.com/microsoft-outlook-preserve-declined-meetings/ (third-party; default off, admin cmdlet)
- https://support.microsoft.com/office/set-your-work-hours-and-location-in-outlook-af2fddf9-249e-4710-9c95-5911edfd76f6 (re-checked: location values, sharing checkboxes, work plan icon)
- https://support.microsoft.com/en-us/outlook/import-or-subscribe-to-a-calendar-in-outlook-com-or-outlook-on-the-web (re-checked: "Select a calendar", refresh intervals)
- https://mc.merill.net/message/RM551199 (print options for calendar events, attendee list of up to 10, GA March 2026)
- https://mc.merill.net/message/MC793320 (re-checked: Split view needs several selected calendars; Month view rollout June–August 2024)
- https://mc.merill.net/message/RM501453 (re-checked: multiple categories per event, GA October 2025)
- https://learn.microsoft.com/en-us/graph/api/resources/datetimetimezone?view=graph-rest-1.0 (Windows IDs plus an IANA subset)
- https://learn.microsoft.com/en-us/graph/api/resources/recurrencerange?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/resources/calendarpermission?view=graph-rest-1.0 (sharing roles)
- https://learn.microsoft.com/en-us/graph/api/user-list-calendarview?view=graph-rest-1.0 (UTC default, Prefer headers, scopes)
- https://learn.microsoft.com/en-us/graph/api/calendar-getschedule?view=graph-rest-1.0 (interval 5–1440, work or school only)
- https://learn.microsoft.com/en-us/graph/api/user-findmeetingtimes?view=graph-rest-1.0 (default 30 min, default work hours Mon–Fri 8–5)
- https://learn.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-oxcical/fe87b9d5-c005-4b92-b54b-fe6f1504c3a0 (ATTENDEE import and export)
- https://learn.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-oxcical/b75dacb7-5f75-4a25-bf29-a6b1423a6676 (ROLE)
- https://learn.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-oxcical/67ff347b-f291-47be-a157-92e5d882502c (CUTYPE)
- https://learn.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-oxcical/54d3000c-a6b3-43c0-b4b0-543578f86fa0 (TRANSP)
- https://github.com/microsoft/fluentui/blob/master/packages/tokens/src/global/durations.ts
- https://chrismenardtraining.com/post/outlook-on-the-web-3-calendar-features-the-desktop-version-doesnt-have (third-party, May 2023; web Filter menu top-level entries)
- https://office-watch.com/2024/my-favorite-outlook-calendar-view-looks-to-the-future/ (third-party; Day menu in new Outlook; seen via search summary only)
- https://www.usecarly.com/blog/how-to-schedule-a-recurring-meeting-in-outlook (third-party; Repeat preset wording; seen via search summary only)
- https://www.ucl.ac.uk/isd/how-to/create-repeated-meeting-outlook-web-access-owa (third-party; older web Repeat wording; seen via search summary only)
- https://m365admin.handsontek.net/?p=7704 (third-party; reminders window preview, MC638133; seen via search summary only)
