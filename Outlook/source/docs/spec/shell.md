# Axentra Outlook: app shell, design tokens and settings

Spec section: **Shell**. Covers everything that is not module content: design tokens, the window frame, the header bar, the left app rail, the ribbon, the Settings dialog, and the feedback surfaces (toasts, undo bar, banners, loading and offline states). The message list, reading pane, compose, calendar surface and people views have their own sections. This section owns the tokens and the chrome they sit in.

Reference product: the **new Outlook for Windows / Outlook on the web** (Fluent 2 era, 2024-2026). Where classic Outlook for Windows differs, a note says so. Axentra Outlook copies layout and behaviour. It does not copy logos, illustrated theme artwork, product icons or prose.

## 0. How to read this spec

### 0.1 Source tags

Each fact carries a source tag so engineers can tell documented facts from conventions.

| Tag | Meaning |
|---|---|
| **[F]** | Taken from the Fluent 2 design site or the MIT-licensed `microsoft/fluentui` token source. Exact values. |
| **[D]** | Documented by Microsoft (support.microsoft.com, learn.microsoft.com, Microsoft 365 release notes or roadmap). |
| **[3P]** | Comes from a reputable third-party write-up (how-to sites, community answers). Usually accurate, but it may lag the product. |
| **[U]** | Unverified. This is our reading of the product's layout conventions, with no public doc found. Treat it as provisional and keep it easy to change (config or constants, not scattered literals). |
| **(estimate)** | A number (px, ms, %) that no doc gives. It is our best value consistent with Fluent 2 guidance. It is never a documented fact. |
| **[O, build, date]** | Observed by hand in a licensed, running install of the reference (build number and date given). Used only to replace [U] after the UI-freeze check in §13.3. No decompiling or resource extraction. |

Variants used in the text: **[D, web]** = documented for Outlook on the web only; **[D, classic]** = documented for classic Outlook for Windows only; **[D, older]** = documented in an article that newer articles contradict. Roadmap items are cited by their Microsoft 365 roadmap ID and count as [D] for intent, not for shipped labels.

### 0.2 Naming conventions in code

- Global tokens use CSS custom properties that keep Fluent names: `--colorNeutralForeground1`, `--spacingHorizontalM`, `--borderRadiusMedium`, `--shadow16`, and so on. Engineers can then cross-check against Fluent docs one-to-one.
- Axentra-only aliases use the `--ax-` prefix, for example `--ax-header-bg` or `--ax-rail-width`.
- Theme, mode and density are attributes on `<html>`: `data-theme="blue|neutral|..."`, `data-mode="light|dark"`, `data-density="roomy|cozy|compact"`, `data-ribbon="simplified|classic"`, `data-chrome="cards|flat"` (Classic look, §1.15), `data-contrast="normal|high"`.

---

## 1. Design tokens

### 1.1 Token architecture

Three layers, matching Fluent UI v9 [F]:

1. **Global palette.** Raw ramps: the brand ramp (16 steps), the grey ramp (2-98), shared colours (red, green, orange, cranberry, yellow, and so on), plus global size scales.
2. **Alias (semantic) tokens.** `colorNeutralBackground1`, `colorBrandForeground1`, and so on. Light and dark map these to different palette steps. **Components consume only alias tokens.**
3. **Axentra component aliases.** `--ax-*`. They map to alias tokens, never to raw hex.

Rule: no hex literal is allowed in component CSS. The only hex values in the codebase live in `tokens/palette.css` (generated) and in the theme ramp files.

### 1.2 Brand ramp: "Outlook blue" (Fluent `brandWeb`) [F]

This is the default Axentra theme ramp. `brand[80]` = **#0F6CBD** is the primary brand colour of Fluent 2 web (`webLightTheme` / `webDarkTheme` are built from `brandWeb`) [F]. That new Outlook ships exactly this ramp is our reading of the product, not a documented fact [U].

Note for engineers reading the Fluent source: the `// #0078d4` style comments next to `brand[80]` etc. in `lightColor.ts` / `darkColor.ts` are stale comments from the old Fluent 1 ramp. Resolve every alias through `brandWeb` (table below), never through those comments.

| Step | Hex | Step | Hex |
|---|---|---|---|
| 10 | `#061724` | 90 | `#2886DE` |
| 20 | `#082338` | 100 | `#479EF5` |
| 30 | `#0A2E4A` | 110 | `#62ABF5` |
| 40 | `#0C3B5E` | 120 | `#77B7F7` |
| 50 | `#0E4775` | 130 | `#96C6FA` |
| 60 | `#0F548C` | 140 | `#B4D6FA` |
| 70 | `#115EA3` | 150 | `#CFE4FA` |
| 80 | `#0F6CBD` | 160 | `#EBF3FC` |

Notes:
- Fluent 1-era Outlook on the web (and the Fluent UI v8 default palette) used `#0078D4` as its primary blue. The new Outlook has an "Outlook" theme that brings back the classic blue scheme [3P]. Axentra's **Classic blue** theme (§1.15) should generate a 16-step ramp around `#0078D4` with the same ramp algorithm as the other colour themes.
- Other ramps in the Fluent source (`brandTeams`, `brandOffice`) exist. Do **not** ship them under product names.

### 1.3 Neutral (grey) ramp [F]

`grey[n]` steps. White = `#FFFFFF`, black = `#000000`.

| n | Hex | n | Hex | n | Hex | n | Hex | n | Hex |
|---|---|---|---|---|---|---|---|---|---|
| 2 | `#050505` | 22 | `#383838` | 42 | `#6B6B6B` | 62 | `#9E9E9E` | 82 | `#D1D1D1` |
| 4 | `#0A0A0A` | 24 | `#3D3D3D` | 44 | `#707070` | 64 | `#A3A3A3` | 84 | `#D6D6D6` |
| 6 | `#0F0F0F` | 26 | `#424242` | 46 | `#757575` | 66 | `#A8A8A8` | 86 | `#DBDBDB` |
| 8 | `#141414` | 28 | `#474747` | 48 | `#7A7A7A` | 68 | `#ADADAD` | 88 | `#E0E0E0` |
| 10 | `#1A1A1A` | 30 | `#4D4D4D` | 50 | `#808080` | 70 | `#B3B3B3` | 90 | `#E6E6E6` |
| 12 | `#1F1F1F` | 32 | `#525252` | 52 | `#858585` | 72 | `#B8B8B8` | 92 | `#EBEBEB` |
| 14 | `#242424` | 34 | `#575757` | 54 | `#8A8A8A` | 74 | `#BDBDBD` | 94 | `#F0F0F0` |
| 16 | `#292929` | 36 | `#5C5C5C` | 56 | `#8F8F8F` | 76 | `#C2C2C2` | 96 | `#F5F5F5` |
| 18 | `#2E2E2E` | 38 | `#616161` | 58 | `#949494` | 78 | `#C7C7C7` | 98 | `#FAFAFA` |
| 20 | `#333333` | 40 | `#666666` | 60 | `#999999` | 80 | `#CCCCCC` | 99 | `#FCFCFC` |

Step 99 exists in the Fluent source but no alias token in §1.4 uses it; keep it in the generated palette for completeness [F].

### 1.4 Neutral and brand alias tokens: light and dark [F]

Values resolve against the brand ramp (§1.2) and grey ramp (§1.3). With a different theme ramp, the `brand[...]` references re-resolve automatically.

#### Foregrounds

| Token | Light | Dark | Typical use |
|---|---|---|---|
| `colorNeutralForeground1` | grey14 `#242424` | white `#FFFFFF` | Primary text |
| `colorNeutralForeground2` | grey26 `#424242` | grey84 `#D6D6D6` | Secondary text, rest icons |
| `colorNeutralForeground2Hover` / `Pressed` / `Selected` | `#242424` | `#FFFFFF` | Icon hover |
| `colorNeutralForeground2BrandHover` / `Selected` | brand80 `#0F6CBD` | brand100 `#479EF5` | Icon on hover (brand tint) |
| `colorNeutralForeground2BrandPressed` | brand70 `#115EA3` | brand90 `#2886DE` | Icon pressed (rail, ribbon; used in §4.2 and §6.1) |
| `colorNeutralForeground3Hover` / `Pressed` / `Selected` | grey26 `#424242` | grey84 `#D6D6D6` | Tertiary text on hover |
| `colorNeutralForeground3` | grey38 `#616161` | grey68 `#ADADAD` | Tertiary text, placeholders, group labels |
| `colorNeutralForeground4` | grey44 `#707070` | grey60 `#999999` | Low-emphasis meta text |
| `colorNeutralForegroundDisabled` | grey74 `#BDBDBD` | grey36 `#5C5C5C` | Disabled text and icons |
| `colorNeutralForegroundOnBrand` | `#FFFFFF` | `#FFFFFF` | Text and icons on brand fill (header) |
| `colorNeutralForegroundInverted` | `#FFFFFF` | grey14 `#242424` | Text on inverted surfaces (key tips, tooltips) |
| `colorNeutralForegroundInverted2` | `#FFFFFF` | grey14 `#242424` | Info icon and secondary text on inverted toasts |
| `colorBrandForeground1` | brand80 `#0F6CBD` | brand100 `#479EF5` | Brand text, selected icon |
| `colorBrandForeground2` | brand70 `#115EA3` | brand110 `#62ABF5` | Stronger brand text |
| `colorBrandForegroundLink` | brand70 `#115EA3` | brand100 `#479EF5` | Links |
| `colorBrandForegroundLinkHover` | brand60 `#0F548C` | brand110 `#62ABF5` | |
| `colorBrandForegroundLinkPressed` | brand40 `#0C3B5E` | brand90 `#2886DE` | |
| `colorCompoundBrandForeground1` | brand80 `#0F6CBD` | brand100 `#479EF5` | Checked toggles, selected tab text |
| `colorCompoundBrandForeground1Hover` | brand70 `#115EA3` | brand110 `#62ABF5` | |
| `colorCompoundBrandForeground1Pressed` | brand60 `#0F548C` | brand90 `#2886DE` | |

#### Backgrounds

| Token | Light | Dark | Typical use |
|---|---|---|---|
| `colorNeutralBackground1` | `#FFFFFF` | grey16 `#292929` | Cards: message list, reading pane, dialogs, menus |
| `colorNeutralBackground1Hover` | grey96 `#F5F5F5` | grey24 `#3D3D3D` | |
| `colorNeutralBackground1Pressed` | grey88 `#E0E0E0` | grey12 `#1F1F1F` | |
| `colorNeutralBackground1Selected` | grey92 `#EBEBEB` | grey22 `#383838` | |
| `colorNeutralBackground2` | grey98 `#FAFAFA` | grey12 `#1F1F1F` | Secondary surfaces |
| `colorNeutralBackground2Hover` / `Pressed` / `Selected` | `#F0F0F0` / `#DBDBDB` / `#E6E6E6` | `#333333` / `#141414` / `#2E2E2E` | |
| `colorNeutralBackground3` | grey96 `#F5F5F5` | grey8 `#141414` | App canvas behind cards |
| `colorNeutralBackground3Hover` / `Pressed` / `Selected` | `#EBEBEB` / `#D6D6D6` / `#E0E0E0` | `#292929` / `#0A0A0A` / `#242424` | |
| `colorNeutralBackground4` | grey94 `#F0F0F0` | grey4 `#0A0A0A` | |
| `colorNeutralBackground5` | grey92 `#EBEBEB` | black `#000000` | |
| `colorNeutralBackground6` | grey90 `#E6E6E6` | grey20 `#333333` | |
| `colorNeutralBackgroundInverted` | grey16 `#292929` | `#FFFFFF` | Key tips, tooltip (inverted) |
| `colorNeutralBackgroundDisabled` | grey94 `#F0F0F0` | grey8 `#141414` | Disabled inputs and buttons |
| `colorSubtleBackground` | transparent | transparent | Ribbon buttons, rail items, menu items at rest |
| `colorSubtleBackgroundHover` | grey96 `#F5F5F5` | grey22 `#383838` | |
| `colorSubtleBackgroundPressed` | grey88 `#E0E0E0` | grey18 `#2E2E2E` | |
| `colorSubtleBackgroundSelected` | grey92 `#EBEBEB` | grey20 `#333333` | Toggled ribbon button, selected rail item |
| `colorBrandBackground` | brand80 `#0F6CBD` | brand70 `#115EA3` | Primary button, header (light) |
| `colorBrandBackgroundHover` | brand70 `#115EA3` | brand80 `#0F6CBD` | |
| `colorBrandBackgroundPressed` | brand40 `#0C3B5E` | brand40 `#0C3B5E` | |
| `colorBrandBackgroundSelected` | brand60 `#0F548C` | brand60 `#0F548C` | |
| `colorBrandBackground2` | brand160 `#EBF3FC` | brand20 `#082338` | Selected list row tint, info chips |
| `colorBrandBackground2Hover` | brand150 `#CFE4FA` | brand40 `#0C3B5E` | |
| `colorBrandBackground2Pressed` | brand130 `#96C6FA` | brand10 `#061724` | |
| `colorCompoundBrandBackground` | brand80 `#0F6CBD` | brand100 `#479EF5` | Checked switch/checkbox fill |
| `colorCompoundBrandBackgroundHover` | brand70 `#115EA3` | brand110 `#62ABF5` | |
| `colorCompoundBrandBackgroundPressed` | brand60 `#0F548C` | brand90 `#2886DE` | |
| `colorNeutralStencil1` | grey90 `#E6E6E6` | grey34 `#575757` | Skeleton blocks |
| `colorNeutralStencil2` | grey98 `#FAFAFA` | grey20 `#333333` | Skeleton shimmer highlight |
| `colorBackgroundOverlay` | `rgba(0,0,0,0.40)` | `rgba(0,0,0,0.50)` | Modal scrim |

#### Strokes

| Token | Light | Dark | Typical use |
|---|---|---|---|
| `colorNeutralStroke1` | grey82 `#D1D1D1` | grey40 `#666666` | Input borders, outlined buttons |
| `colorNeutralStroke1Hover` | grey78 `#C7C7C7` | grey46 `#757575` | |
| `colorNeutralStroke1Pressed` | grey70 `#B3B3B3` | grey42 `#6B6B6B` | |
| `colorNeutralStroke1Selected` | grey74 `#BDBDBD` | grey44 `#707070` | |
| `colorNeutralStroke2` | grey88 `#E0E0E0` | grey32 `#525252` | Dividers, ribbon group separators |
| `colorNeutralStroke3` | grey94 `#F0F0F0` | grey24 `#3D3D3D` | Very subtle dividers |
| `colorNeutralStrokeAccessible` | grey38 `#616161` | grey68 `#ADADAD` | Input bottom border (3:1 contrast) |
| `colorNeutralStrokeAccessibleSelected` | brand80 `#0F6CBD` | brand100 `#479EF5` | |
| `colorNeutralStrokeDisabled` | grey88 `#E0E0E0` | grey26 `#424242` | |
| `colorBrandStroke1` | brand80 `#0F6CBD` | brand100 `#479EF5` | Brand outlines |
| `colorBrandStroke2` | brand140 `#B4D6FA` | brand50 `#0E4775` | Soft brand outlines |
| `colorCompoundBrandStroke` | brand80 `#0F6CBD` | brand100 `#479EF5` | Selected tab indicator, input focus underline |
| `colorCompoundBrandStrokeHover` | brand70 `#115EA3` | brand110 `#62ABF5` | |
| `colorCompoundBrandStrokePressed` | brand60 `#0F548C` | brand90 `#2886DE` | |
| `colorStrokeFocus1` | `#FFFFFF` | `#000000` | Inner focus ring |
| `colorStrokeFocus2` | `#000000` | `#FFFFFF` | Outer focus ring |
| `colorTransparentStroke` | transparent | transparent | Reserves border space so layout does not shift |

#### Shadow colours

| Token | Light | Dark |
|---|---|---|
| `colorNeutralShadowAmbient` | `rgba(0,0,0,0.12)` | `rgba(0,0,0,0.24)` |
| `colorNeutralShadowKey` | `rgba(0,0,0,0.14)` | `rgba(0,0,0,0.28)` |
| `colorNeutralShadowAmbientLighter` | `rgba(0,0,0,0.06)` | `rgba(0,0,0,0.12)` |
| `colorNeutralShadowKeyLighter` | `rgba(0,0,0,0.07)` | `rgba(0,0,0,0.14)` |
| `colorNeutralShadowAmbientDarker` | `rgba(0,0,0,0.20)` | `rgba(0,0,0,0.40)` |
| `colorNeutralShadowKeyDarker` | `rgba(0,0,0,0.24)` | `rgba(0,0,0,0.48)` |
| `colorBrandShadowAmbient` / `Key` | `rgba(0,0,0,0.30)` / `rgba(0,0,0,0.25)` | same |

### 1.5 Semantic (status) colours [F]

The Fluent v9 status mapping is **success = green**, **warning = orange**, **danger = cranberry** [F]. The values below are computed from those palette families and the Fluent generation rules, light overrides and dark overrides included.

| Token | Danger light | Danger dark | Success light | Success dark | Warning light | Warning dark |
|---|---|---|---|---|---|---|
| `…Background1` (banner fill) | `#FDF3F4` | `#3B0509` | `#F1FAF1` | `#052505` | `#FFF9F5` | `#4A1E04` |
| `…Background2` | `#EEACB2` | `#6E0811` | `#9FD89F` | `#094509` | `#FDCFB4` | `#8A3707` |
| `…Background3` (solid badge) | `#C50F1F` | `#C50F1F` | `#107C10` | `#107C10` | `#F7630C` | `#F7630C` |
| `…Foreground1` (banner icon/text) | `#B10E1C` | `#DC626D` | `#0E700E` | `#54B054` | `#BC4B09` | `#FAA06B` |
| `…Foreground2` | `#6E0811` | `#EEACB2` | `#094509` | `#9FD89F` | `#8A3707` | `#FDCFB4` |
| `…Foreground3` | `#C50F1F` | `#EEACB2` | `#107C10` | `#9FD89F` | `#BC4B09` | `#F98845` |
| `…Border1` (banner border) | `#EEACB2` | `#C50F1F` | `#9FD89F` | `#107C10` | `#FDCFB4` | `#F7630C` |
| `…Border2` | `#C50F1F` | `#DC626D` | `#107C10` | `#9FD89F` | `#BC4B09` | `#F98845` |

Extras: `colorStatusDangerBackground3Hover` = `#B10E1C` and `…Pressed` = `#960B18` (both modes) [F]. Full token names follow the pattern `colorStatusDangerBackground1`, `colorStatusSuccessForeground1`, and so on.

Two more status tokens per family exist and are needed for inverted surfaces (a status icon on a dark tooltip or an inverted toast) [F]:

| Token | Danger light / dark | Success light / dark | Warning light / dark |
|---|---|---|---|
| `…ForegroundInverted` | `#DC626D` / `#B10E1C` | `#54B054` / `#0E700E` | `#FAA06B` / `#BC4B09` |
| `…BorderActive` | `#C50F1F` / `#DC626D` | `#107C10` / `#54B054` | `#F7630C` / `#FAA06B` |

**Informational** banners use neutral tokens: fill `colorNeutralBackground3`, icon `colorNeutralForeground3`, border `colorNeutralStroke1` [F: Fluent v9 MessageBar base styles]. (An earlier draft said `colorNeutralStroke2`; the Fluent source uses Stroke1.)

### 1.6 Axentra app-semantic aliases

These are colours that Outlook's UI needs and Fluent does not name. The owning module spec may refine them. The shell defines them so every module agrees.

| Alias | Light | Dark | Notes |
|---|---|---|---|
| `--ax-canvas` | `colorNeutralBackground3` `#F5F5F5` | `colorNeutralBackground3` `#141414` | Area behind the floating panes (estimate) |
| `--ax-pane-bg` | `colorNeutralBackground1` `#FFFFFF` | `colorNeutralBackground1` `#292929` | Ribbon card, message list, reading pane |
| `--ax-header-bg` | `colorBrandBackground` `#0F6CBD` | `colorNeutralBackground2` `#1F1F1F` | Dark mode uses a neutral header (estimate) |
| `--ax-header-fg` | `#FFFFFF` | `colorNeutralForeground1` `#FFFFFF` | |
| `--ax-unread-accent` | `colorCompoundBrandForeground1` | same | Unread bar and unread subject tint (estimate) |
| `--ax-row-selected` | `colorBrandBackground2` `#EBF3FC` | `colorBrandBackground2` `#082338` | Selected row tint (estimate) |
| `--ax-flag-fg` | cranberry `#C50F1F` | cranberry tint30 `#DC626D` | Flag icon (estimate) |
| `--ax-flagged-row-bg` | yellow tint50 `#FFFAD6` | yellow shade50 `#282400` | Flagged/pinned row tint (estimate) |
| `--ax-badge-bg` | `colorStatusDangerBackground3` `#C50F1F` | same | Notification count badge (estimate) |
| `--ax-offline-pill-bg` | `colorNeutralBackgroundInverted` `#292929` | `#FFFFFF` | Offline pill in header (estimate) |

### 1.7 Typography [F]

#### Font families

| Token | Fluent value [F] | Axentra Linux stack |
|---|---|---|
| `fontFamilyBase` | Segoe UI, then Segoe UI Web (West European), -apple-system, BlinkMacSystemFont, Roboto, Helvetica Neue, sans-serif | `"Segoe UI Variable Text", "Segoe UI", "Selawik", "Noto Sans", "Cantarell", "Ubuntu", Roboto, "Helvetica Neue", Arial, sans-serif` |
| `fontFamilyMonospace` | Consolas, Courier New, Courier, monospace | `Consolas, "Cascadia Mono", "DejaVu Sans Mono", "Noto Sans Mono", "Courier New", monospace` |
| `fontFamilyNumeric` | Bahnschrift, then the base stack | `Bahnschrift, <base stack>` |

- **Segoe UI / Segoe UI Variable** are Microsoft fonts that cannot be redistributed. Use them only if the user already has them installed (`local()`).
- **Selawik** is Microsoft's open-source (SIL OFL 1.1) replacement for Segoe UI, © 2015, with **Reserved Font Name "Selawik"** [D: Microsoft Typography font list]. Third parties describe it as metric-compatible with Segoe UI but with different kerning, and note it has seen no meaningful updates for years [3P]. Microsoft's own page makes no metric-compatibility claim; verify line metrics against Segoe UI ourselves (see §12.3). It ships five styles: Light, Semilight, Regular, Semibold, Bold [D]. **Bundle Selawik** (Light 300, Semilight 350, Regular 400, Semibold 600, Bold 700) so the layout metrics in this spec hold on stock Linux systems.
  - **Latin only.** Selawik covers code pages 1252, 1250, 1254 and 1257 (Western, Central European, Turkish, Baltic) and nothing else [D]. Greek, Cyrillic, Hebrew, Arabic, CJK and so on fall through to the next family in the stack, so keep `"Noto Sans"` (and system CJK fonts via fontconfig) directly after Selawik. Test mixed-script subjects in the message list for baseline jumps.
  - **Licence hygiene.** Ship the upstream TTF/WOFF files unmodified. Under the OFL Reserved Font Name clause, a subset or otherwise modified build must not be called "Selawik"; if the build pipeline subsets fonts, rename the family (for example "Axentra Sans") and keep the OFL text in the About page's licence list.
- Segoe UI Variable has optical-size variants. Fluent 2's Windows ramp pairs Caption with "small", Body Strong with "text" and Subtitle and larger with "display" [F]. If the variable font is present, use `font-optical-sizing: auto`. If not, the static fonts are fine.

#### Weights [F]

`fontWeightRegular` 400, `fontWeightMedium` 500, `fontWeightSemibold` 600, `fontWeightBold` 700.

#### Size and line-height scale [F]

| Token step | Font size | Line height |
|---|---|---|
| 100 | 10px | 14px |
| 200 | 12px | 16px |
| 300 | 14px | 20px |
| 400 | 16px | 22px |
| 500 | 20px | 28px |
| 600 | 24px | 32px |
| Hero 700 | 28px | 36px |
| Hero 800 | 32px | 40px |
| Hero 900 | 40px | 52px |
| Hero 1000 | 68px | 92px |

#### Type ramp: web (the default for Axentra) [F]

| Style | Size / line height | Weight |
|---|---|---|
| Caption 2 | 10 / 14 | Regular |
| Caption 2 Strong | 10 / 14 | Semibold |
| Caption 1 | 12 / 16 | Regular |
| Caption 1 Strong / Stronger | 12 / 16 | Semibold / Bold |
| Body 1 | 14 / 20 | Regular |
| Body 1 Strong / Stronger | 14 / 20 | Semibold / Bold |
| Body 2 | 16 / 22 | Regular |
| Subtitle 2 | 16 / 22 | Semibold |
| Subtitle 2 Stronger | 16 / 22 | Bold |
| Subtitle 1 | 20 / **28** | Semibold |
| Title 3 | 24 / 32 | Semibold |
| Title 2 | 28 / 36 | Semibold |
| Title 1 | 32 / 40 | Semibold |
| Large Title | 40 / 52 | Semibold |
| Display | 68 / 92 | Semibold |

**Subtitle 1 line height.** The Fluent 2 design site's web-ramp table lists Subtitle 1 as 20/26, but the shipped token source (`typographyStyles.subtitle1` = `fontSizeBase500` / `lineHeightBase500`) gives **20/28**, and there is no 26px line-height token [F]. Axentra follows the code: 20/28. (An earlier draft of this spec used 20/26.)

Body 2 (16/22 regular) is in the Fluent token source but not in the design site's web table [F]. Use it for long-form settings descriptions only if Body 1 proves too small.

The Windows (Segoe UI Variable) ramp differs slightly: Caption 12/16, Body 14/20, Body Strong 14/20, Body Large 18/24, Subtitle 20/28, Title 28/36, Large Title 40/52, Display 68/92 [F]. Axentra uses the web ramp because the new Outlook is a web-rendered client.

#### Shell usage map (all estimate)

| Element | Style |
|---|---|
| Header app name | Subtitle 2 (16/22 semibold) (estimate) |
| Header search text and placeholder | Body 1 (estimate) |
| Ribbon tab labels | Body 1. Selected tab is Body 1 Strong (estimate) |
| Simplified ribbon command labels | Body 1 (estimate) |
| Classic ribbon large-button labels | Caption 1 (estimate) |
| Classic ribbon group labels | Caption 1, `colorNeutralForeground3` (estimate) |
| Rail tooltips and key tips | Caption 1 / Caption 1 Strong (estimate) |
| Settings dialog title | Subtitle 1 (20/28) (estimate) |
| Settings page heading | Subtitle 2 (estimate) |
| Settings section heading | Body 1 Strong (estimate) |
| Settings option text and descriptions | Body 1 / Caption 1 in `colorNeutralForeground3` (estimate) |
| Toast title and body | Body 1 Strong / Body 1 (estimate) |
| Badge count | Caption 2 Strong (estimate) |

### 1.8 Corner radii [F]

| Token | Value | Shell usage |
|---|---|---|
| `borderRadiusNone` | 0 | Header, window edges when maximised |
| `borderRadiusSmall` | 2px | Badges with square corners, key tips (estimate) |
| `borderRadiusMedium` | 4px | Buttons, inputs, search box, menu items, focus ring default. Fluent v9 also uses it for **Menu popovers, Popover surfaces, Tooltips, Toasts and MessageBars** [F] |
| `borderRadiusLarge` | 6px | Floating panes (ribbon card, list, reading pane) (estimate) |
| `borderRadiusXLarge` | 8px | **Dialogs** (Fluent DialogSurface default) [F]. Axentra also uses 8px for header flyouts (search suggestions, Account Manager, app launcher) and the undo bar; this departs from Fluent's 4px popover default and is our reading of new Outlook's look (estimate) |
| `borderRadius2XLarge` / `3XLarge` / `4XLarge` / `5XLarge` / `6XLarge` | 12 / 16 / 24 / 32 / 40px | Not used in the shell |
| `borderRadiusCircular` | 10000px | Avatars, count badges, pill indicators, rail selection pill |

### 1.9 Shadows and elevation [F]

Each shadow has two layers, ambient and key [F]:

| Token | CSS (light: ambient 0.12 / key 0.14) | Dark (0.24 / 0.28) |
|---|---|---|
| `shadow2` | `0 0 2px rgba(0,0,0,.12), 0 1px 2px rgba(0,0,0,.14)` | `0 0 2px rgba(0,0,0,.24), 0 1px 2px rgba(0,0,0,.28)` |
| `shadow4` | `0 0 2px rgba(0,0,0,.12), 0 2px 4px rgba(0,0,0,.14)` | `0 0 2px rgba(0,0,0,.24), 0 2px 4px rgba(0,0,0,.28)` |
| `shadow8` | `0 0 2px rgba(0,0,0,.12), 0 4px 8px rgba(0,0,0,.14)` | `0 0 2px rgba(0,0,0,.24), 0 4px 8px rgba(0,0,0,.28)` |
| `shadow16` | `0 0 2px rgba(0,0,0,.12), 0 8px 16px rgba(0,0,0,.14)` | `0 0 2px rgba(0,0,0,.24), 0 8px 16px rgba(0,0,0,.28)` |
| `shadow28` | `0 0 8px rgba(0,0,0,.12), 0 14px 28px rgba(0,0,0,.14)` | `0 0 8px rgba(0,0,0,.24), 0 14px 28px rgba(0,0,0,.28)` |
| `shadow64` | `0 0 8px rgba(0,0,0,.12), 0 32px 64px rgba(0,0,0,.14)` | `0 0 8px rgba(0,0,0,.24), 0 32px 64px rgba(0,0,0,.28)` |

All six steps take the same ambient and key colours. Only the blur and offset change; `shadow28` and `shadow64` widen the ambient blur from 2px to 8px [F]. The "Lighter" and "Darker" shadow colours (§1.4) are separate tokens for custom use. Axentra uses `colorNeutralShadowAmbientDarker`/`KeyDarker` only for the Settings dialog when it sits over another dialog (estimate). Brand variants (`shadow2Brand` and so on) use the same geometry with `colorBrandShadow*`. Use them only on coloured surfaces.

Elevation map for shell surfaces. The shadow steps for Tooltip (`shadow8`), Menu (`shadow16`), Toast (`shadow8`) and Dialog (`shadow64`) match the Fluent v9 component sources [F]; the pane, side-pane and z-index choices are ours [U]:

| Surface | Shadow | z-index (estimate) |
|---|---|---|
| Floating panes (ribbon card, message list, reading pane) | `shadow2` in light. None in dark; use a 1px `colorNeutralStroke3` border instead (estimate) | 0 |
| Tooltip | `shadow8` | 1000 |
| Ribbon dropdown menus, split-button menus, Move to, Categorize | `shadow16` | 900 |
| Header flyouts (app launcher, account manager, overflow) | `shadow16` | 900 |
| My Day and Notifications side panes | `shadow28` | 800 |
| Toasts and undo bar | `shadow8` [F: Fluent Toast] | 1100 |
| Settings and other dialogs | `shadow64` plus scrim | 1200 |

### 1.10 Spacing scale [F]

Each step has a horizontal and a vertical token, for example `spacingHorizontalM` and `spacingVerticalM`.

| Key | Value | Key | Value |
|---|---|---|---|
| None | 0 | M | 12px |
| XXS | 2px | L | 16px |
| XS | 4px | XL | 20px |
| SNudge | 6px | XXL | 24px |
| S | 8px | XXXL | 32px |
| MNudge | 10px | | |

Shell gutters: 4px gap between floating panes, 8px inner padding in panes, 16px padding inside dialogs and side panes, 24px dialog edge padding (all estimate).

### 1.11 Stroke widths [F]

| Token | Value | Usage |
|---|---|---|
| `strokeWidthThin` | 1px | Borders and dividers |
| `strokeWidthThick` | 2px | Focus ring, input focus underline |
| `strokeWidthThicker` | 3px | Selected-tab indicator, rail selection pill (estimate) |
| `strokeWidthThickest` | 4px | Unread indicator bar in the message list (estimate) |

### 1.12 Focus ring

Fluent v9 implementation [F]:
- The ring is drawn by an `::after` pseudo-element with `position:absolute; inset: -2px; border: 2px solid var(--colorStrokeFocus2); border-radius: var(--borderRadiusMedium); pointer-events:none; z-index:1`. The host needs `position: relative`.
- The native `outline` is suppressed (`outline: none`). The ring shows **only for keyboard focus**. Fluent uses a `data-fui-focus-visible` attribute; Axentra uses `:focus-visible`.
- The width is a literal 2px (Fluent itself hardcodes 2px rather than `strokeWidthThick`).
- An optional outline offset pushes the ring further out, per side if needed.
- Under `forced-colors: active` the ring colour becomes `Highlight`.

Axentra rules:
- On brand-filled surfaces (the header in light mode, primary buttons), add a 1px inner ring in `colorStrokeFocus1` inside the 2px `colorStrokeFocus2` ring, so the ring stays visible on blue [U].
- In the header (blue background, light mode), use ring colour `#FFFFFF` (estimate).
- Text inputs (search box, settings fields) show focus with a 2px bottom underline in `colorCompoundBrandStroke`, not the ring. This is the Fluent Input pattern [U].
- List rows (rail items, settings nav items, menu items) use an inset ring (`inset: 0`) so it is not clipped by `overflow: hidden` parents [U].

### 1.13 Motion [F]

| Duration token | Value | Curve token | Value |
|---|---|---|---|
| `durationUltraFast` | 50ms | `curveAccelerateMax` | `cubic-bezier(0.9,0.1,1,0.2)` |
| `durationFaster` | 100ms | `curveAccelerateMid` | `cubic-bezier(1,0,1,1)` |
| `durationFast` | 150ms | `curveAccelerateMin` | `cubic-bezier(0.8,0,0.78,1)` |
| `durationNormal` | 200ms | `curveDecelerateMax` | `cubic-bezier(0.1,0.9,0.2,1)` |
| `durationGentle` | 250ms | `curveDecelerateMid` | `cubic-bezier(0,0,0,1)` |
| `durationSlow` | 300ms | `curveDecelerateMin` | `cubic-bezier(0.33,0,0.1,1)` |
| `durationSlower` | 400ms | `curveEasyEaseMax` | `cubic-bezier(0.8,0,0.2,1)` |
| `durationUltraSlow` | 500ms | `curveEasyEase` | `cubic-bezier(0.33,0,0.67,1)` |
| | | `curveLinear` | `cubic-bezier(0,0,1,1)` |

Shell usage (estimate):
- Hover colour changes: `durationFaster` with `curveEasyEase`.
- Menu and flyout enter: `durationNormal`, `curveDecelerateMid`, fade plus 4px slide. Exit: `durationFast`, `curveAccelerateMid`.
- Side pane (My Day, Notifications) slide-in: `durationGentle` with `curveDecelerateMax`.
- Toast enter: `durationNormal` slide up 8px. Exit: `durationFast` fade.
- Dialog enter: `durationGentle` scale from 0.98 plus fade.
- `prefers-reduced-motion: reduce` turns every slide or scale into a fade of `durationFaster` or less. Skeleton shimmer becomes a static block.

### 1.14 Density

**Current names**: **Roomy**, **Cozy**, **Compact**, set from **View > Density** [D: Microsoft's "Change the look" article, which applies to Outlook.com; a 2025 third-party guide shows the same names and path in new Outlook [3P]]. Earlier builds used **Full / Medium / Compact**, and some third-party guides (including one dated June 2026) still use those names and also point to Settings > General > Appearance [3P]. Show only Roomy, Cozy and Compact. Accept the old names (Full = Roomy, Medium = Cozy) when importing settings.

| Density | Documented behaviour [D] | Shell token effects (all estimate) |
|---|---|---|
| **Roomy** | Shows sender images, attachment previews and message preview text | Folder row 36px, settings list row 44px, rail item 48px |
| **Cozy** (default [3P]; Microsoft does not document the default) | Shows message preview text only, with some spacing | Folder row 32px, settings list row 40px, rail item 48px |
| **Compact** | Shows message preview text only, with minimal spacing | Folder row 28px, settings list row 36px, rail item 44px |

Notes:
- Density mainly changes the message-list row; the message list spec owns those heights. The ribbon and header do not change with density (estimate).
- The individual toggles (preview text on or off and lines, sender images, attachment previews) still exist in Settings and the View tab (§6.5, §7.4). Density sets their **defaults**. If the user changes a toggle afterwards, the change wins and the density menu shows the closest preset with no checkmark (estimate).
- Since July 2026 the message list can show up to **two lines** of preview text [D: release notes, July 31, 2026]. The matching roadmap item (564610) describes three choices: preview text on **2 lines**, on **1 whole line**, or on **0 additional lines** (the preview shares a line with other content) [D]. The UI labels for these three choices are not documented [U].

### 1.15 Themes and dark mode

**What the reference product has** [D/3P]:
- Settings > General > Appearance holds the theme picker and the Dark mode choice [D].
- Theme groups: **Modern themes** (Settings > General > Appearance > Modern themes; four were added in November 2024 [D]; "pick a theme, then a colour scheme" is [3P]), classic **Themes** (including **Outlook**, the classic blue) [3P], and **Neutral**, a mica-like neutral scheme that matches Windows 11 [3P].
- **Dark mode** options: third-party guides name **Dark** and **Use system settings** and describe following the Windows colour mode as the default [3P]. A separate **Light** choice is our assumption [U]. Microsoft's own page only says the mode is chosen under General > Appearance [D].
- The reading pane can keep a **light background while dark mode is on** (February 2025) [D]. A sun/moon button in the reading pane header switches the message background [U].
- An October 2, 2026 update made available "a UI style closer to the classic Outlook design" [D: release notes]. No Microsoft page found (release notes, What's new, roadmap feed as of 2026-10-19) says whether it is a toggle, where it lives, or what it changes. Roadmap item 574115 ("Upcoming visual improvements", in development) concerns **classic** Outlook's icons and spacing and is a different change. Axentra plan: implement an opt-in **Classic look** switch in Appearance (estimate), default off, that sets `data-chrome="flat"` on `<html>`: no pane gaps, no pane radius or shadow, panes separated by 1px `colorNeutralStroke2` dividers, ribbon flush with the header. Because chrome is token-driven, this costs only an alias set. Revisit the label and default once Microsoft documents the control.

**Axentra theme catalogue.** These are colour-only themes. Never ship Microsoft illustrated theme artwork.

| Theme | Ramp | Header (light) | Notes |
|---|---|---|---|
| Blue (default) | `brandWeb` (§1.2) | brand80 | |
| Classic blue | ramp generated around `#0078D4` | ramp step 80 | Stands in for Microsoft's "Outlook" theme |
| Neutral | `brandWeb` for accents | `colorNeutralBackground3` with dark text | No mica on Linux; flat neutral (estimate) |
| Teal, Purple, Pink, Red, Orange, Green, Graphite | 16-step ramps generated from one key colour each (estimate) | ramp step 80 | Check contrast: white on step 80 must be at least 4.5:1 |

Generating a ramp: use the Fluent theme-designer approach. Take the key colour as step 80 and interpolate in a perceptual space (OKLCH) towards near-black at step 10 and near-white at step 160. Then run a contrast check on each alias pair that §1.4 uses (estimate).

**Mode resolution:** `Use system settings` reads the XDG Desktop Portal `org.freedesktop.appearance` / `color-scheme` key (0 = no preference, 1 = prefer dark, 2 = prefer light; unknown values count as 0) [D: portal spec]. With 0 (no preference) resolve to light. It falls back to the GTK theme name ending in `-dark` only when the portal is unavailable. Watch the portal's `SettingChanged` signal and re-theme live, without a restart.

The same portal namespace has three more keys Axentra should read [D: portal spec]:
- `contrast` (0 = normal, 1 = higher contrast): drives High contrast when that setting is "Follow system".
- `reduced-motion` (0 = no preference, 1 = reduce): drives Reduce motion when that setting is "Follow system" (§7.3 Accessibility).
- `accent-color` (sRGB triple, 0-1): optional. A future "System accent" theme can feed it to the ramp generator. Out-of-range values mean "unset".

**High contrast:** Linux desktops have no OS forced-colours mode that reaches WebKitGTK, so `@media (forced-colors: active)` cannot be relied on [U: we found no WebKitGTK documentation either way]. Offer an explicit **High contrast** mode under Appearance > Accessibility (Off / On / Follow system via the portal `contrast` key). It maps all foregrounds to black or white, strokes to 2px, and removes tints. Keep the Fluent `forced-colors` rules (they are harmless) so that a future WebKitGTK that supports them still works.

### 1.16 Iconography

- Use **Fluent UI System Icons** (MIT, `microsoft/fluentui-system-icons`). Use the Regular style at rest and the Filled style for selected or checked states. Default sizes: 20px in ribbon, header and rail, 16px in menus and inline (estimate). Classic-ribbon large buttons use 32px or 28px (estimate).
- **Never** use Microsoft product logos (Outlook, Teams, Word, Excel, OneDrive, Microsoft 365) in the rail, the launcher or the About page. Axentra's own app icon is the only brand mark.

---

## 2. Window anatomy

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ HEADER 48px: [Launcher][App name] ···· [ Search (centred) ] ···· [btns][Avatar][–][□][×] │
├──────┬────────────────────────────────────────────────────────────────────────┤
│ RAIL │ RIBBON CARD: tabs row + command row                                    │
│ 48px ├──────────────┬─────────────────────────┬───────────────────────────────┤
│      │ FOLDER PANE  │ MESSAGE LIST (card)     │ READING PANE (card)           │
│ Mail │ (module nav) │                         │                               │
│ Cal  │              │                         │                               │
│ Ppl  │              │                         │                               │
│ …    │              │                         │                               │
│ More │              │                         │                               │
└──────┴──────────────┴─────────────────────────┴───────────────────────────────┘
                     (Toasts and the undo bar float bottom-left of the content area)
```

| Region | Size | Resizable | Notes |
|---|---|---|---|
| Header | 48px tall (estimate) | No | Doubles as the window title bar (client-side decorations) |
| App rail | 48px wide (estimate) | No | Always visible. Never moves to the bottom in new Outlook [3P] |
| Ribbon card | Simplified: 36px tabs row + 40px commands = 76px. Classic: 36px + 92px = 128px (estimate) | No | Can collapse to the tabs row only (§6.2) |
| Folder pane | 228px default, 160-400px (estimate) | Drag splitter | Collapsible from View > Folder pane [D] |
| Message list | 360px default, 280px min (estimate) | Drag splitter | |
| Reading pane | Fills the rest, 400px min (estimate) | Drag splitter | Position Right / Bottom / Fill screen / Popout only (§7.5) |
| Pane gaps | 4px (estimate) | | `--ax-canvas` shows through |

**Responsive behaviour** (window widths, all estimate):

| Width | Behaviour |
|---|---|
| 1280px and wider | Full layout |
| 1024-1279px | Folder pane narrows to 200px. Simplified ribbon starts dropping labels |
| 860-1023px | Folder pane collapses to an overlay opened by a hamburger button at the start of the ribbon. The search box shrinks to 240px |
| 640-859px | Reading pane forced to **Fill screen** behaviour (opening a message replaces the list; Back returns). The search box becomes an icon button that expands over the header |
| Under 640px | Minimum window width is 640px; scroll horizontally below that (estimate) |

Minimum window size: 640 × 480 (estimate).

---

## 3. App header bar

### 3.1 Anatomy, left to right

| # | Element | Label / tooltip | Size (estimate) | Behaviour | Src |
|---|---|---|---|---|---|
| 1 | **App launcher** | Tooltip "App launcher" | 48×48 hit area, 20px 3×3 dot-grid icon | Opens a flyout under the button listing Axentra modules (Mail, Calendar, People, Groups, To Do, Files) and **All apps** (configured web links). Esc closes it | [U] |
| 2 | **App name** | "Axentra Outlook" | Subtitle 2, 12px left padding | Click: goes to the current account's Inbox in Mail (like Outlook on the web's product name link). Hidden below 860px | [U] |
| 3 | Drag region | none | flexible | Mouse-down starts a window move. Double-click toggles maximise (respect desktop settings) | [U] |
| 4 | **Search box** | Placeholder "Search" | 32px tall. Width `clamp(240px, 36vw, 560px)`, centred on the **window**, not the free space | See §3.2 | [U] |
| 5 | Drag region | none | flexible | | |
| 6 | **Meet now** | Tooltip "Meet now" | 40×48, 20px video icon | Hidden unless an online-meeting provider is configured for the active account. Starts an instant meeting and copies the join link | [U] |
| 7 | **My Day** | Tooltip "My Day" | 40×48, 20px calendar-check icon | Toggles the My Day side pane (§3.4). Sits next to Notifications [D] | [D] |
| 8 | **Notifications** | Tooltip "Notifications" | 40×48, 20px bell icon with count badge | Toggles the Notifications pane (§3.5). A red badge shows the unread count [D/3P] | [D] |
| 9 | **Settings** | Tooltip "Settings" | 40×48, 20px gear icon | Opens the Settings dialog (§7) at the last-visited page. Gear at the top right [D] | [D] |
| 10 | **More** (overflow) | Tooltip "More options" | 40×48, `…` icon | Menu: **Sync** (manual mailbox refresh [D, October 2026]), **Help**, **Feedback**, **What's new**, **Keyboard shortcuts**, **About Axentra Outlook** | [D]/[U] |
| 11 | **Account avatar** | Tooltip: account display name plus email | 32px circle in a 48×48 hit area | Opens the **Account Manager** flyout (§3.6) [D: release notes June 18, 2026; roadmap 497546, launched] | [D]/[U] |
| 12 | Window controls | "Minimize", "Maximize"/"Restore", "Close" | 46×48 each (estimate) | Follow the desktop's button layout (GNOME `button-layout`, which can put them on the left) | [U] |

Help: in the reference product, Help mostly lives on the ribbon's **Help** tab. The Tips pane moved from the header to the Help tab in January 2025 [D]. Axentra keeps Help in both places: the More menu and the Help tab.

Overflow: the October 2, 2026 release moved "secondary suite header actions" into an overflow menu and added a **Sync** button there [D]. Microsoft does not say which actions count as secondary. Getting-started docs name only **Settings** (gear) and **My Day** as upper-right buttons, and the My Day article places My Day next to **Notifications** [D]. Axentra therefore keeps My Day, Notifications and Settings standalone and moves everything else into More [U: our reading of "secondary"]. Whether Meet now stays standalone for Teams-enabled accounts is not documented; Axentra shows it standalone only when a meeting provider is configured, otherwise it is absent (not in More) [U].

Visual states (header in light mode, on brand):

| State | Button background | Icon |
|---|---|---|
| Rest | transparent | `#FFFFFF` |
| Hover | `colorBrandBackgroundHover` `#115EA3` | `#FFFFFF` |
| Pressed | `colorBrandBackgroundPressed` `#0C3B5E` | `#FFFFFF` |
| Selected (its pane is open) | `colorBrandBackgroundSelected` `#0F548C` | `#FFFFFF`, Filled icon variant |
| Focused | Rest plus a white 2px ring with a 1px inner black ring, radius 4px | |
| Disabled | transparent | `#FFFFFF` at 40% opacity (estimate) |

In dark mode (neutral header `#1F1F1F`), use subtle tokens: hover `#383838`, pressed `#2E2E2E`, selected `#333333`, icon `colorNeutralForeground2` `#D6D6D6`, and `#FFFFFF` on hover.

### 3.2 Search box

- Rest (light, on brand): background `#FFFFFF` at 92% opacity (estimate), no border, radius 4px, leading 16px search icon in `colorNeutralForeground3`, placeholder **"Search"** in `colorNeutralForeground3` [U for the placeholder text].
- Hover: background `#FFFFFF`.
- Focus: background `#FFFFFF`, 2px bottom underline in `colorCompoundBrandStroke`. The box grows to `min(720px, 50vw)` over `durationNormal` (estimate). A **filter button** appears at the trailing edge, which opens From / To / Subject / Keywords / Date range / Has attachments filters [3P]. A **clear (×)** button appears once there is text.
- Dark: background `colorNeutralBackground1` `#292929`, placeholder `#ADADAD`.
- Typing shows a suggestion flyout under the box (`shadow16`, radius 8px). Sections: recent searches, People, Mail, Files; suggestions can be scoped to these entities [D, November 2024]. Up/Down moves through suggestions, Enter runs the search, Esc clears the text, a second Esc blurs the box and **cancels the search** [D: Esc cancels a search].
- Running a search shows a contextual **Search** ribbon tab with scope and refiners [D, December 2025]. Scope options: **All folders**, **Current folder** (with a subfolder option), **Current mailbox** [3P/D: folder and subfolder scope since February 2026]. The default scope is a setting (§7.3, General > Search).
- The placeholder is module-aware: Mail "Search", Calendar "Search", People "Search" (estimate: same text everywhere, the scope chip changes). Focus shortcuts: **Alt+Q** (documented for Outlook on the web [D]; the new Outlook section of the shortcut page lists no mail-search key) and **Ctrl+E** (classic Outlook search, also F3 [D, classic]; documented for People search in new Outlook [D]). `/` is not bound by default.
- Coming in the reference (roadmap, in development, not yet shipped): **Multi Account Search** with results from several mailboxes in one ranked list (570441), and an **All Accounts** unified-inbox view (564609) [D]. Design the scope control so an "All accounts" scope can be added without a layout change.
- Offline: search still works over locally saved items [D]. Show a Caption 1 hint under the suggestions: "Showing results saved on this device".

### 3.3 Notification badge

- Circular, 16px minimum height, 4px horizontal padding, Caption 2 Strong white text on `--ax-badge-bg`. Placed at the icon's top-right, offset (+6px, −6px) (estimate).
- Counts above 99 show **"99+"**. A count of 0 hides the badge. The badge has a 1px ring in the header background colour so it separates from the icon (estimate).

### 3.4 My Day pane

- Right-side pane under the header, 320px wide (estimate), full height, `shadow28`, `colorNeutralBackground1`. It pushes nothing; it overlays the reading pane.
- Header row: title "My Day", close button (×). Tabs **Calendar** and **To Do** [D].
- Calendar tab: today's agenda. Past and completed events are hidden [D, December 2024]. A date control at the top changes the day shown [D]. **New event** button [D]. A **Switch views** (more) menu toggles **agenda** vs **day** view and picks which extra calendars to show [D]. Selecting an event shows its details with **Back** and **View full event in new window** [D]; online meetings show **Join** [D]. Empty state: "Nothing scheduled for today" (Axentra copy).
- To Do tab: today's task list, **Add a task** (plus icon; Enter adds), a star to **mark as important**, a circle to **mark as completed**, drag handles to reorder, and the buttons **Open To Do** and **Manage all tasks**, which open the To Do module [D]. My Day's task list clears at midnight; the tasks themselves remain in their lists [D].
- Drag and drop [D]: dragging a task onto the calendar creates an event; dragging a message into the To Do tab creates a task. Axentra supports both when the target module exists for the account.
- Opening it from any module keeps the module state. The pane's open state persists per window.

### 3.5 Notifications pane

- Same container as My Day (they are mutually exclusive; opening one closes the other).
- Header: title "Notifications", an account dropdown to pick which account's notifications show [D, January 2026], **Dismiss all** [D, November 2024: dismiss individual or all notifications]. The exact button label "Dismiss all" is [U].
- Cards: avatar, title (Body 1 Strong), snippet (Caption 1), relative time, an importance icon where it applies [D, October 2025]. Cards from the same thread or document are **batched** into one group [D, April 2025]. Hovering a card shows **Dismiss** [D].
- Clicking a card goes to the item and marks the notification read. Unread cards have a 4px `--ax-unread-accent` left bar (estimate).
- Empty state: an illustration-free icon (bell with a check) and the text "You're all caught up" (Axentra copy).

### 3.6 Account Manager flyout

- Anchored under the avatar, 320px wide (estimate), `shadow16`, radius 8px.
- Content: the current account (avatar 48px, display name, email, nickname if set [D: account nicknames]); a list of other accounts with avatars and unread totals; **Add account**; **Manage accounts** (opens Settings > Accounts > Email accounts) [D: the Account Manager lets you view your profile and picture, manage accounts, and add accounts when the organization's policy allows it]. Axentra equivalent of that policy: hide **Add account** when a system policy file forbids extra accounts (estimate).
- No Microsoft sign-in links. Axentra does not offer "Sign out" for the app. Per-account removal lives in Settings.

### 3.7 Header keyboard

- Tab order: Launcher → App name → Search → Meet now → My Day → Notifications → Settings → More → Avatar → window controls.
- **F6** / **Shift+F6** cycle the landmark regions: Header → Rail → Ribbon → Folder pane → Message list → Reading pane → (open side pane) [D: F6 moves between regions in the calendar; extended to all modules here].
- Esc in any flyout closes it and returns focus to its invoker.

---

## 4. Left app rail (module bar)

### 4.1 Contents

Reference order in new Outlook (2026): **Mail**, **Calendar**, *(Copilot, where licensed)*, **People**, then optional pinned apps (Groups, To Do, Office apps), then **More apps** [D: Ctrl+1 Mail, Ctrl+2 Calendar, Ctrl+3 Copilot, Ctrl+4 People; **More apps** adds Word, Excel, PowerPoint or OneNote to the bar; 3P: pin and unpin behaviour]. Mail and Calendar cannot be unpinned [3P]. People, Groups, To Do and others can be pinned, unpinned and reordered by dragging. Right-click → **Pin** / **Unpin** [3P]. The rail is vertical on the left and cannot be moved [3P: Microsoft Q&A; Microsoft's own page says classic Outlook's bar will "eventually" be fixed to the left too].

Outlook on the web uses **Ctrl+Shift+1 / 2 / 3 / 4** for Mail / Calendar / Copilot / People, because plain Ctrl+digit switches browser tabs [D]. Axentra is a desktop app and follows new Outlook (Ctrl+digit); accept the Ctrl+Shift variants as silent aliases for users coming from the web.

Current classic Outlook (Microsoft 365) uses the same Ctrl+1 to Ctrl+4 map plus Ctrl+5 To Do, Ctrl+6 Org Explorer, Ctrl+7 Loop, Ctrl+8 Tasks [D]. Axentra's Ctrl+5 = To Do therefore matches classic muscle memory.

**Ctrl+3 decision (resolves the earlier open question):** leave **Ctrl+3 unbound** (reserved). Ctrl+4 = People in new Outlook, in current classic Outlook and in Axentra, so users never need Ctrl+3 for People; binding it would give People two keys and would have to be taken back if an assistant module is ever added.

**Axentra rail, default order:**

| # | Item | Tooltip / aria-label | Icon (Fluent System Icons) | Unpinnable | Shortcut | Src |
|---|---|---|---|---|---|---|
| 1 | Mail | "Mail" | `mail` | No | Ctrl+1 | [D] |
| 2 | Calendar | "Calendar" | `calendar_ltr` | No | Ctrl+2 | [D] |
| 3 | People | "People" | `people` | Yes | Ctrl+4 (Ctrl+3 stays unbound) | [D] |
| 4 | Groups | "Groups" | `people_team` | Yes; shown only for Microsoft 365 work/school accounts whose tenant has consented to the group scopes (§13.7) | Ctrl+6 (Axentra extension) | [3P] |
| 5 | To Do | "To Do" | `checkmark_circle` / `task_list_ltr` | Yes | Ctrl+5 (matches classic Outlook [D, classic]) | [3P] |
| 6 | Files | "Files" (attachment hub) | `document_multiple` | Yes | Ctrl+7 (Axentra extension) | [U] |
| — | (flexible spacer) | | | | | |
| 7 | More apps | "More apps" | `apps_add_in` / `grid_dots` | n/a; hidden when nothing is left to pin [3P] | none | [D] |

### 4.2 Visual spec (all estimate)

- Rail width 48px. Background `--ax-canvas` (the rail sits on the canvas, not on a card). 4px top padding below the header.
- Item: 48×48 hit area (44px in Compact), centred 20px icon. Text labels are not shown by default. Tooltip after a 500ms hover, placed to the right, `shadow8`.
- States:

| State | Icon | Background | Indicator |
|---|---|---|---|
| Rest | Regular, `colorNeutralForeground2` | transparent | none |
| Hover | Regular, `colorNeutralForeground2BrandHover` | `colorSubtleBackgroundHover` in a 40×40 radius-4 box | none |
| Pressed | Regular, `colorNeutralForeground2BrandPressed` | `colorSubtleBackgroundPressed` | none |
| Selected (active module) | **Filled**, `colorCompoundBrandForeground1` | `colorSubtleBackgroundSelected` | 3px × 20px pill, `borderRadiusCircular`, `colorCompoundBrandStroke`, flush to the rail's left edge, vertically centred; animated with `durationNormal` when the module changes |
| Focused | as rest/selected | | Inset 2px focus ring, radius 4px |
| Disabled (account type lacks the module) | `colorNeutralForegroundDisabled` | transparent | none; tooltip says why |

- Fluent reference for the pill: a vertical medium Fluent Tab draws a 3px-wide `colorCompoundBrandStroke` indicator inset 8px from the top and bottom of the item, radius circular [F]. On a 48px rail item that would be 32px tall; Axentra's 20px pill is a deliberate, shorter choice (estimate).
- Unread hint: a module item may show a 6px dot badge (`--ax-badge-bg`) at its top-right when it has new activity that is not visible on screen. To Do does not show one by default (estimate).
- Drag-reordering pinned items shows a 2px brand insertion line. Mail and Calendar are fixed at the top.

### 4.3 Rail behaviour

- Switching module keeps the previous module's state (selection, scroll, open drafts). Switching takes under 100ms for cached modules (estimate). Show a skeleton if data is not ready (§9.1). The reference also keeps selection and scroll position per folder when you switch folders (March 2026) [D]; the shell's state store must key list state by account + folder, not only by module.
- **More apps** opens a flyout grid of unpinned modules and configured web links. Right-click an item → **Pin**.
- Context menu on a pinned item: **Unpin**, **Open in new window** (Calendar and People) [U].
- Since July 2026 the taskbar icon reflects the active module [D]. Axentra: update the window title to "Mail - Axentra Outlook", "Calendar - Axentra Outlook", and so on, and set the X11/Wayland app icon variant if the desktop supports it (optional).
- Keyboard: Up/Down move between rail items while focus is in the rail. Enter/Space activates. Home/End go to the first/last item.

---

## 5. Module-to-ribbon tab matrix

| Module | Tabs (left to right) | Src |
|---|---|---|
| Mail (reading) | **File**, **Home**, **View**, **Help** | [D: "a File tab next to Home", What's new, November 2025; Help tab named in the January 2025 release notes (Tips pane moved there)] |
| Mail (searching) | adds **Search** (contextual) after Help, selected automatically | [D] |
| Mail (inline compose/reply active) | adds **Message**, **Insert**, **Format text**, **Draw**, **Options** (owned by the compose spec) | [U] |
| Calendar | **File**, **Home**, **View**, **Help** | [U] |
| People | **Home**, **View**, **Help** | [U] |
| Groups / To Do / Files | **Home**, **Help** | [U] |

---

## 6. Ribbon

### 6.1 Anatomy (both layouts)

- The ribbon is a floating card (`--ax-pane-bg`, radius 6px, `shadow2` in light) spanning from the rail to the window's right edge, with 4px margins (estimate).
- **Tabs row**, 36px (estimate): Fluent TabList, medium size. Tab label Body 1, rest colour `colorNeutralForeground2` [F]. Selected tab text is `colorNeutralForeground1` semibold (estimate). The selected indicator is a 3px (`strokeWidthThicker`) bar in `colorCompoundBrandStroke` (hover `…StrokeHover`, pressed `…StrokePressed`) at the bottom of the tab, inset 12px (`spacingHorizontalM`) from each side of the tab box, radius circular [F: Fluent medium horizontal Tab]. Hover on an unselected tab shows `colorNeutralForeground2Hover` text and the same 3px bar in `colorNeutralStroke1Hover` (pressed: `colorNeutralStroke1Pressed`) [F]. Medium tab padding is 12px vertical × 10px horizontal [F].
- **Command row**: buttons in Fluent Toolbar style, `colorSubtleBackground*` states, radius 4px.
- **Layout switch** at the far right of the command row: a downward caret that third parties call **Switch layouts** [3P]; the tooltip text is [U]. A support page describes it as a dropdown arrow on the right that switches between the larger classic and the smaller simplified ribbon [D]. In current builds one click toggles Classic ↔ Simplified rather than opening a menu [3P, 2025-2026]. Axentra: a single toggle button whose icon and tooltip reflect the target layout. The choice persists per window and is shared across modules [3P: "Outlook will remember your choice"].
- **Collapse the ribbon**: a checkable item on the **right-click menu of a ribbon tab** [3P]. When it is checked, only the tabs row shows (§6.2 "Collapsed state"). Double-clicking a tab also toggles it (estimate, classic Office convention).
- **More options (…)** overflow button just before the layout switch, in Simplified only.

Button states (command row):

| State | Background | Icon / label |
|---|---|---|
| Rest | `colorSubtleBackground` | icon `colorNeutralForeground2`, label `colorNeutralForeground1` |
| Hover | `colorSubtleBackgroundHover` | icon `colorNeutralForeground2BrandHover` (blue) |
| Pressed | `colorSubtleBackgroundPressed` | icon `colorNeutralForeground2BrandPressed` |
| Checked (toggle on, e.g. Pin while pinned) | `colorSubtleBackgroundSelected` | Filled icon, `colorCompoundBrandForeground1` |
| Disabled | transparent | `colorNeutralForegroundDisabled` |
| Focused | as current | 2px focus ring, radius 4px |

**Split buttons**: the primary part and the 20px chevron part hover separately, divided by a 1px `colorNeutralStroke2` line on hover. **Alt+Down** opens the menu [D]. **Dropdown buttons** (chevron only, no default action) open the menu on click, Enter, Space or Alt+Down.

### 6.2 Simplified vs Classic

| Aspect | Simplified (default) [3P] | Classic |
|---|---|---|
| Rows | One row of commands | Large and small buttons in **labelled groups** |
| Command height | 32px buttons, 20px icons, label to the right (estimate) | Large: 32px icon over a 2-line Caption 1 label, about 72px wide max. Small: 16px icon with the label to its right, 3 per column (estimate) |
| Labels | "Show labels as space permits" (default), "Show labels always", "Hide labels always" [3P: Customize dialog] | Always shown |
| Overflow | Commands that do not fit, or that the user removed, go into **More options (…)** [3P] | Groups shrink: large → small → icon-only → group collapses into a dropdown named after the group (estimate) |
| Customisation | **Customize** (from … on the Home or View tab) add/remove/reorder buttons, add/rename/delete groups, **Reset all changes**, **Save** [3P; the getting-started page confirms a **Customize** entry for the simplified ribbon [D]]. Only the Mail ribbon can be customised; settings are per account [3P] | Rolling out from October 2026: **right-click the ribbon** and choose which commands and groups appear on the **Home** and **View** tabs [D: roadmap 570664, "Rolling out", GA October CY2026]. The roadmap gives no menu label; "Customize the ribbon…" is our placeholder [U] |
| Collapsed state | Tabs row only; clicking a tab opens the command row as an overlay that closes after a command runs (estimate) | Same |

### 6.3 Mail > Home: command catalogue

The "Default" column marks commands visible by default in the Simplified ribbon. "Add" marks commands only available through Customize or the overflow. The order is the Simplified left-to-right order. The classic group appears in the last column.

| # | Command (label) | Type | Menu items / behaviour | Shortcut | Enabled when | Default | Classic group | Src |
|---|---|---|---|---|---|---|---|---|
| 1 | **New mail** | Split | Primary: new message. Menu: **Mail**, **Mail from template** [D, November 2025: templates are used via New > Mail from template], **Event**, **Group** (Microsoft 365 only) | Ctrl+N | Always | Yes | New | [U] (Ctrl+N [D]; template item [D]) |
| 2 | **Delete** | Split | Primary: move to Deleted Items. Menu: **Delete**, **Permanently delete**, **Empty folder** (on folders that support it) | Delete; Shift+Delete = permanent [D] | Selection | Yes | Delete | [U]/[D] |
| 3 | **Archive** | Button | Moves to the account's Archive folder [D] | none [U] | Selection, and not already in Archive | Yes | Delete | [D] |
| 4 | **Report** | Split/dropdown | **Report phishing**, **Report junk**; in the Junk folder, **Not junk**. Junk options now live under Report [3P] | none | Selection | Yes | Delete | [3P] |
| 5 | **Sweep** | Button | Opens the Sweep dialog: delete all from this sender, keep only the latest, delete older than 10 days, and similar. Unavailable in Junk Email, Drafts, Sent Items and Deleted Items [D]. Standing Sweep rules run **once a day**, not instantly [D]; the dialog must say so | none | Single selection | Yes | Delete | [D] |
| 6 | **Move to** | Dropdown | Folder search field "Search for a folder", recent folders, **Inbox**, **Archive**, **Other folder…** (opens the Go to folder dialog), **Copy to…** [U]. For accounts with Focused Inbox on, in the Inbox: **Move to Focused inbox** / **Move to Other inbox** and **Always move to Focused inbox** / **Always move to Other inbox** [D: Focused Inbox article, new Outlook and web] | Ctrl+Shift+V [U; it is the classic Outlook key [D, classic]] | Selection | Yes | Move | [D]/[U] |
| 7 | **Reply** | Button | | Ctrl+R [D] | Single selection | Yes | Respond | [D] |
| 8 | **Reply all** | Button | | Ctrl+Shift+R [D] | Single selection | Yes | Respond | [D] |
| 9 | **Forward** | Split | **Forward**, **Forward as attachment** | Ctrl+F [D] (while the reading pane does not have focus; Ctrl+F in the reading pane is Find [D]) | Selection | Yes | Respond | [D]/[U] |
| 10 | **Quick steps** | Split/dropdown | The user's quick steps in order, then **Manage quick steps** [D]. Settings creates them with **+ New quick step** [D] | Per step: **Ctrl+Shift+5** to **Ctrl+Shift+9** are the assignable keys in new Outlook and the web [D: Quick steps article] | Depends on the step | Yes (Classic shows them as a gallery) [D] | Quick steps | [D] |
| 11 | **Read / Unread** | Toggle | Label and icon swap to match the selection state | Ctrl+Q / Ctrl+U [D] | Selection | Yes | Tags | [D] |
| 12 | **Categorize** | Dropdown | Search field, category list with checkboxes (several allowed [D, March 2026]), **New category**, **Manage categories**, **Clear categories** | none | Selection | Yes | Tags | [D]/[U] |
| 13 | **Flag / Unflag** | Split | Primary toggles the flag. Menu: **Today**, **Tomorrow**, **This week**, **Next week**, **No date**, **Mark complete**, **Clear flag** | Insert [D] | Selection | Yes | Tags | [D]/[U] |
| 14 | **Pin / Unpin** | Toggle | Pinned items stay at the top of the list [D]; undoable [D, April 2026] | none | Selection | Yes | Tags | [D] |
| 15 | **Snooze** | Dropdown | **Later today**, **Tomorrow**, **This weekend**, **Next week**, **Choose a date** (date/time picker) | none | Selection in Inbox | Yes | Tags | [D] (items [U]) |
| 16 | **Policy** | Dropdown | Retention and archive policies from the server | none | Exchange accounts only | No (Add) | Tags | [D] |
| 17 | **Rules** | Dropdown | **Create rule** (from the selected message), **Manage rules** | none | Selection | No (Add) | Move | [3P] |
| 18 | **Block** | Dropdown | **Block sender**, **Never block sender**, **Never block sender's domain** | none | Selection | No (Add) | Delete | [3P] (items [U]) |
| 19 | **Meeting** (reply with meeting) | Button | Creates an event inviting the message's participants | none | Single selection | No (Add) | Respond | [3P] |
| 20 | **Print** | Button | Also on the File tab [D] | Ctrl+P [U] | Single selection | No (overflow) | (File) | [D] |
| 21 | **Immersive reader** | Button | Opens the message in a distraction-free reader | none | Single selection | No (Add) | (View) | [U] |
| 22 | **Discard** / **Send** | n/a | These appear only on the compose tabs, not on Home | | | | | — |

Enablement rules:
- With **no selection**, only New mail, Quick steps (steps that need no selection) and Sweep (disabled) render. Everything else is disabled, not hidden. Hidden commands would make the ribbon jump.
- With a **multi-selection**, Reply, Reply all, Forward (Forward as attachment stays enabled), Sweep and Meeting are disabled.
- **Offline**: commands that need the server are disabled with the tooltip suffix "(not available offline)". These are Sweep, Policy and Block (estimate: the reference does not list them either way). The reference documents these as working offline (queued until reconnect) [D: Work offline article]: Archive, Categorize, Copy to folder, Delete, Flag/Unflag, Ignore, Mark as read/unread, Move to Focused/Other, Move to folder, Pin, **Report junk / Not junk**, **Report phishing**, RSVP and Snooze. (An earlier draft wrongly disabled Report phishing offline.)
- In **Drafts**, Reply, Reply all and Forward are disabled. In **Junk Email**, Report becomes **Not junk** as its primary action.

### 6.4 Mail > Home: Classic layout groups

| Group | Contents (large = L, small = S) |
|---|---|
| **New** | New mail (L, split) |
| **Delete** | Delete (L, split), Archive (L), Report (S, split), Sweep (S), Block (S, if added) |
| **Respond** | Reply (L), Reply all (L), Forward (L, split), Meeting (S, if added) |
| **Move** | Move to (L, dropdown), Rules (S, if added) |
| **Quick steps** | Gallery of 3 visible steps plus a scroll-down chevron and Manage quick steps |
| **Tags** | Read/Unread (S), Categorize (S), Flag/Unflag (S), Pin/Unpin (S), Snooze (S), Policy (S, Exchange) [D: Tags section] |
| **Find** | Discover groups (Exchange) [U] |

Group names: "Tags" is documented [D]. New, Delete, Respond, Move and Quick steps follow the classic Outlook Home groups [3P]. Group separators: 1px `colorNeutralStroke2`, 8px horizontal margin. The group label sits centred at the bottom in Caption 1 `colorNeutralForeground3` (estimate).

### 6.5 Mail > View tab

| # | Command | Type | Options | Src |
|---|---|---|---|---|
| 1 | **View settings** | Button | Opens the Settings dialog (gear icon on the button). Current support articles route through it: "View > View Settings > Mail > Message List" and "View > View Settings > General > Notifications" [D]. Axentra opens Mail > Message list when invoked from Mail | [D] |
| 2 | **Folder pane** | Dropdown | **Show** / **Hide** (radio), separator, **Sort folders** submenu: **Custom folder order**, **Sort A to Z** | [D] (Folder pane > Sort folders > Custom folder order, February 2025 release notes) / [U] (rest) |
| 3 | **Reading pane** | Dropdown | **Show on the right**, **Show on the bottom**, **Fill screen**, **Popout only** [D: reading pane article, new Outlook]. The Outlook.com article still lists **Hide** instead of the last two | [D] |
| 4 | **Density** | Dropdown | **Roomy**, **Cozy**, **Compact** | [D] |
| 5 | **Conversations** | Dropdown | **Group messages by conversation**, **Group messages by branches within conversations**, **Do not group messages**; separator; **Newest on top**, **Newest on bottom** (reading pane order) | [D] (grouping labels) / [U] (order) |
| 6 | **Message preview** | Dropdown | **Show preview text**, **Hide preview text** [D: "Hide preview text" is in the View tab's Messages group]; separator; preview length: **2 lines**, **1 line**, **No extra line** (July 2026; roadmap 564610 describes 2 lines / 1 whole line / 0 additional lines) | [D] behaviour / [U] labels for the line options |
| 7 | **Sender image** | Toggle | Shows or hides sender images in the list | [U] |
| 8 | **Zoom** | Dropdown | Minus, current % (Caption 1), plus, **Reset**; applies to the reading pane | [D] |
| 9 | **Immersive reader** | Button | See §6.3 #21 | [U] |

View-tab controls apply **immediately** (no Save) and update the same settings that the Settings pages show (§7.5).

Classic layout groups: **Settings** (View settings) | **Layout** (Folder pane, Reading pane, Density) | **Messages** (Conversations, Message preview, Sender image) | **Zoom** (Zoom) | **Immersive** (Immersive reader). Layout (containing Reading pane) and Messages (containing Conversations and Hide preview text) group names are documented [D]; the others are [U].

The Simplified **Customize** dialog can also be opened from the View tab's More options (…) [3P], so View-tab commands must be declared in the same command table as Home (§12.7).

### 6.6 Calendar > Home tab

| # | Command | Type | Options | Shortcut | Src |
|---|---|---|---|---|---|
| 1 | **New event** | Split | **Event**, **Meeting templates** (personal and admin templates) [D, March 2026], **Mail** | Ctrl+N [D] | [D]/[U] |
| 2 | **Day** | Split | Primary: Day view. Menu: **1 day** … **7 days** [3P: the Day view can show 1-7 days] | Ctrl+Alt+1 [D] | [D]/[3P] |
| 3 | **Work week** | Toggle | | Ctrl+Alt+2 [D] | [D] |
| 4 | **Week** | Toggle | | Ctrl+Alt+3 [U]; the docs list conflicting keys (§10) | [D] |
| 5 | **Month** | Toggle | | Ctrl+Alt+4 or Alt+= [D] | [D] |
| 6 | **Split view** | Toggle | Side-by-side vs overlaid multiple calendars [3P] | none | [3P] |
| 7 | **Filter** | Dropdown | **Show all**, **Appointments**, **Meetings**, **Recurring**, **Categories ›** (category list), **Show as ›** (Free/Busy/Tentative/Out of office) | none | [U] |
| 8 | **Share** | Dropdown | List of the user's calendars → opens the sharing dialog | none | [U] |
| 9 | **Print** | Button | Opens print preview for the current range | Ctrl+P [U] | [U] |
| 10 | **Meet now** | Button | Only with a configured meeting provider (classic Outlook puts it on Calendar Home next to Teams meeting [3P]) | none | [3P] |

Day / Work week / Week / Month form a **radio group**: exactly one is checked. The Today button and the previous/next arrows live in the calendar surface header (calendar spec), not in the ribbon [U].

### 6.7 Calendar > View tab

| # | Command | Type | Options | Src |
|---|---|---|---|---|
| 1 | **Calendar settings** | Button | Opens Settings > Calendar > View [3P] | [3P] |
| 2 | **Day / Work week / Week / Month** | Toggles | Same radio group as Home | [3P] |
| 3 | **Saved views** | Dropdown | Saved views list, **Save current view** | [3P] |
| 4 | **Time scale** | Dropdown | **60 minutes**, **30 minutes**, **15 minutes**, **10 minutes**, **6 minutes**, **5 minutes** [3P] | [3P] |
| 5 | **Split view** | Toggle | | [3P] |
| 6 | **Folder pane** | Dropdown | Show / Hide | [U] |

### 6.8 People > Home tab

| # | Command | Type | Options | Shortcut | Src |
|---|---|---|---|---|---|
| 1 | **New contact** | Split | **New contact**, **New contact list** [D] | Ctrl+N [D, People section]; Ctrl+L = new contact list [D] | [D] |
| 2 | **Edit** | Button | Edits the selected contact or list [D] | Shift+E [D] | [D] |
| 3 | **Delete** | Button | Deletes the contact or list (up to 100 contacts at once, August 2026 [D]) | Delete or Ctrl+D [D] | [D] |
| 4 | **Add to favorites** / **Remove from favorites** | Toggle | Favourites with an email address also appear in Mail's folder pane [D] | none | [D] |
| 5 | **Add to list** | Dropdown | Existing contact lists, **New contact list** | none | [U] |
| 6 | **Categorize** | Dropdown | Same as Mail. Since September 2026 several categories can be edited or deleted at once in People [D] | none | [D]/[U] |
| 7 | **Email** | Button | New message to the selected contacts | none | [U] |
| 8 | **Manage contacts** | Dropdown | **Import contacts** (CSV/vCard), **Export contacts**, **Restore deleted contacts** | none | [U] |

People > View tab (optional): **Sort by** (First name / Last name), **Show** (Contacts / Contact lists / All), **Folder pane**. All [U].

Other documented People keys the shell must not steal [D, new Outlook People section]: **Ctrl+E** search people, **Ctrl+A** select all contacts, **Shift+?** list People shortcuts.

### 6.9 File, Help and Search tabs

**File tab** (in new Outlook it is a ribbon tab, not a Backstage view) [D: November 2025]:

| Command | Notes | Src |
|---|---|---|
| **Print** | Prints the selected item | [D] |
| **Manage accounts** | Opens Settings > Accounts | [D] |
| **Save as template** / **Load template** | Message templates (Settings > Files). The reference saves a new message as a template, or forwards it as an `.oft` file, from the File tab (January 2026) [D] | [D] |
| **Save as** | Saves the selected message as `.eml` (Axentra). The reference saves `.msg` from the message's right-click menu (**Save as > Save as MSG**, May 2025) and `.oft` from the File tab [D] | [D]/[U] |
| **Import / Export** | Opens Settings > Files (Axentra: mbox, eml, ics, vcf) | [D]/[U] |

**Help tab** [U for most items; Tips moved here in January 2025 [D]]: **Help**, **Tips**, **Feedback**, **Support** [D: Help → Support or Feedback], **What's new**, **Keyboard shortcuts**, **Diagnostics** (log export). There is no "Go to classic Outlook" command; Axentra has no classic counterpart.

**Search tab** (contextual) [D: December 2025]: **Scope** dropdown (All folders / Current folder / Current mailbox / include subfolders), **From**, **To**, **Subject**, **Has attachments** (toggle), **Unread** (toggle), **Flagged** (toggle), **Date** (From/To pickers), **Clear filters**, **Exit search**. Refiner labels are [3P].

### 6.10 Customize dialog (Simplified ribbon)

Opened from **More options (…) → Customize** on the Mail Home or View tab [3P]:
- Left: **Buttons**, a checkbox list of every Mail Home command in §6.3. Ticking adds a command, unticking removes it [3P]. Where a newly ticked command lands (start or end of its group) is not documented; Axentra appends it to the end of its default group (estimate).
- Right: **Customize** preview. A × on each button removes it. Drag to reorder within or across groups. **Add group**, plus a pencil (rename) and, on hover, a trash icon (delete) on each group [3P]. A deleted group cannot be restored [3P]; Axentra asks for confirmation.
- Label mode select at the bottom: **Show labels as space permits** (default), **Show labels always**, **Hide labels always** [3P; the source capitalises every word].
- Footer: **Reset all changes** (bottom left; confirm first, it cannot be undone [3P]), **Cancel**, **Save** (primary).
- Scope: customisations apply to the account whose folder is selected [3P]. Store them per account.

### 6.11 Overflow algorithm (Simplified)

1. Measure the command row width (`ResizeObserver`). Reserve the overflow and layout-switch buttons (about 72px, estimate).
2. Each command has a priority. Defaults, high to low: New mail, Delete, Archive, Reply, Reply all, Forward, Move to, Read/Unread, Flag, Report, Categorize, Pin, Snooze, Sweep, Quick steps, the rest (estimate).
3. If the row is too wide in "as space permits" mode: first drop labels from the lowest-priority labelled commands, then move whole commands, lowest priority first, into **More options**.
4. The overflow menu lists the hidden commands in ribbon order, with their icons. Split buttons become submenu items whose first entry is the primary action.
5. Recompute on resize with a single `requestAnimationFrame`. Hysteresis of 8px prevents flicker (estimate).

### 6.12 Ribbon keyboard

| Key | Action | Src |
|---|---|---|
| **Alt** | Shows key tips (Alt alone since August 2024; before that Win+Alt). Can be turned off in Settings (key tips setting, June 2025) | [D] |
| **Alt, H** | Focuses the Home tab | [U] for new Outlook; the shortcut page lists Alt+H / Alt+V only for classic Outlook [D, classic] |
| **Alt, V** | Focuses the View tab | [U] for new Outlook (classic convention) |
| **Alt, F** / **Alt, Y** (Help) / **Alt, S** (Search) | Focus the File / Help / Search tabs | [U] |
| **Up / Down** | Move between the tabs row and the command row | [D, February 2025] |
| **Left / Right** | Move between tabs or commands (wrapping) | [U] |
| **Alt+Down** | Opens a split or dropdown menu | [D] |
| **Enter / Space** | Activates | [U] |
| **Esc** | Closes the menu; next press leaves key tips; next returns focus to the content | [U] |

Key tips are small badges (min 16px tall, Caption 1 Strong, `colorNeutralBackgroundInverted` with `colorNeutralForegroundInverted` text, radius 2px), centred under each target (estimate). Second-level tips appear after the tab tip is typed (for example Alt, H, then **N** for New mail). Letters are configurable in a table, not hardcoded in components.

---

## 7. Settings dialog

### 7.1 Shell anatomy

- Modal over a `colorBackgroundOverlay` scrim. Size `min(1080px, 92vw)` × `min(760px, 90vh)` (estimate). Radius 8px, `shadow64`, `colorNeutralBackground1`.
- Three columns (estimate):
  1. **Category nav**, 200px: the dialog title "Settings" (Subtitle 1) at the top, a **Search settings** box under it, then the categories with 20px icons.
  2. **Page nav**, 220px: the selected category's pages.
  3. **Content**: page title (Subtitle 2), sections and fields, scrolling on its own.
- A **Close (×)** button at the top right. Esc closes the dialog if nothing is unsaved; otherwise it shows the discard prompt below.
- Footer, sticky at the bottom of the content column: **Save** (primary, disabled until something changes) and **Discard** (secondary) [D: pages end with "Save"].
- Navigating away from a page with unsaved changes asks: "Discard unsaved changes?" with buttons **Discard** and **Keep editing** (Axentra copy).
- The selected nav item uses `colorSubtleBackgroundSelected`, Body 1 Strong, and a 3px brand pill on its left edge (same component as the rail).
- **Search settings**: filters all page titles and option labels. Results list "Page › Section". Enter jumps to the page and briefly highlights the option with a `colorBrandBackground2` flash over `durationSlow` (estimate). An example from the docs is searching for "Mark as read" [3P].
- Account-scoped pages show an **account picker** dropdown at the top of the content area, for example Focused Inbox [D: choose the account], Categories [D], Signatures and Automatic replies.
- Entry points: the header gear and the View tab's **View settings** button both open this dialog [D].
- List pages (Rules, Conditional formatting, Quick steps, Search folders) get a **filter-by-name** box above the list [D: roadmap 557678, rolling out]. Provide it as a shared settings-list component.
- Admin-locked values: the reference is adding Cloud Policy defaults that admins can lock (roadmap 571199, in development) [D]. Axentra cannot read that service; keep a generic "Managed by your organization" disabled state in the settings components for future use (for example `/etc/axentra-outlook/policy.toml`) (estimate).

### 7.2 Category and page tree

| Category | Pages (in order) | Src |
|---|---|---|
| **Accounts** | **Email accounts** ("Your accounts": list, Add account, Manage, nickname, Remove), **Signatures**, **Categories**, **Automatic replies**, **Shared with me**, **Storage** | [D] getting-started (Settings > Accounts > Add account); [D] Your accounts (March 2025), nicknames and custom account description (October 2025), Signatures page redesign (release notes January 2026; What's new lists it under March 2026), Categories (April 2025), Shared with me (August 2025); Storage [U] |
| **General** | **Language and time**, **Appearance**, **Notifications**, **Accessibility**, **Search**, **Offline**, **Privacy and data**, **Distribution groups** (Exchange), **About Axentra Outlook** | [D] Language and time, Appearance, Notifications, Offline, Privacy and data, About Outlook; [D, Outlook on the web] Accessibility (holds the keyboard-shortcut set); [3P] Search; [U] Distribution groups |
| **Mail** | **Message list** and **Reading pane** (older articles: one **Layout** page, see §7.4), **Folder pane**, **Compose and reply**, **Attachments**, **Rules**, **Sweep**, **Junk email**, **Quick steps**, **Customize actions**, **Message handling**, **Forwarding**, **Conditional formatting**, **Search folders**, **S/MIME**, **Retention policies** (Exchange), **Groups** (Exchange) | [D] Message list (Focused Inbox article), Reading pane (conversations article), Layout (older articles), Folder pane (September 2026), Compose and reply, Rules, Sweep, Junk email, Quick steps, Message handling (mark as read), Conditional formatting (April 2026), Search folders (moved into Settings, March 2026), S/MIME; [U] Attachments, Customize actions, Forwarding, Retention policies, Groups |
| **Calendar** | **View** (Calendar appearance, Work hours and location, Time zones), **Events and invitations**, **Weather**, **Shared calendars**, **Customize actions**, **Events from email** | [D] Events and invitations (March 2025); [3P] View subsections; [U] the others |
| **People** | **Contacts** (sort and display by first or last name) | [D] |
| **Files** | **Import**, **Export**, **Templates** | [D] Settings > Files: Import (March 2026), templates (November 2025), auto-export (September 2025) |
| *(Copilot)* | *Not built.* The reference has **Copilot › Copilot control** with **Turn on Copilot** [D]. Axentra has no assistant; omit the category | [D] |

Page-detail ownership: this shell section specifies the pages in §7.3-§7.6. Other pages (Rules, Sweep, Signatures, Junk, Calendar subpages, and so on) are owned by the module specs, but they must use this dialog's layout components.

### 7.3 General pages

#### Appearance [D/3P]

| Section | Control | Options / default |
|---|---|---|
| **Theme** | Swatch grid (64×48 tiles, radius 4, selected tile has a 2px brand ring and a check badge) (estimate) | Blue (default), Classic blue, Neutral, Teal, Purple, Pink, Red, Orange, Green, Graphite (§1.15) |
| **Dark mode** | Radio group | **Light** [U], **Dark** [3P], **Use system settings** (default) [3P] |
| | Checkbox (shown when Dark is in effect) | **Use a light background for messages** (reading pane stays light in dark mode) [D: February 2025] [U label] |
| **Density** | Radio group with previews | **Roomy**, **Cozy** (default [3P]), **Compact** [D names; the View tab is the documented control, this Settings copy is [3P]] |
| **Layout style** | Switch | **Classic look** (flat panes, no floating cards), default off (Axentra; §1.15) [U] |
| **Title bar** | Checkbox | **Use the system title bar**, default off: the desktop's own title bar instead of the header (§12.4). Applies at the next start (Axentra) [U] |
| **Accessibility** | Radio | **High contrast**: Off (default) / On / Follow system (portal `contrast` key, §1.15) (Axentra) [U] |

Theme and mode preview live as the user picks them and **revert on Discard** (estimate).

#### Notifications [D]

| Section | Control | Notes |
|---|---|---|
| Account | Dropdown | Notifications and sounds can be enabled per account [D, November 2024] |
| **Notify me about** | Switches: **Mail**, **Calendar**, **Documents** [D] | Axentra may hide Documents if no document notifications exist |
| **Mail** (expandable) | **Play sound** (sound picker; custom sounds for new mail **and for reactions** [D, September 2025; roadmap 491019]); **Desktop notifications** on/off; **For messages:** Inbox / Focused Inbox only / Favourite people [3P, 2023 guide; option labels U]; **Notify about reactions** [U label] | Section heading in docs: "New Email and Reactions" [D] |
| **Calendar** (expandable) | **Play sound** for event reminders, custom sound allowed [D]; alert type **Desktop notification** / **Reminder popup** [3P, 2023 guide]; **Send notifications when Outlook is closed** (background reminders) [D, January 2025] | Section heading in docs: "Event reminders" [D]. Axentra label: "Send notifications when Axentra Outlook is closed" |
| **Batching** | **Combine multiple new-mail notifications into one summary** (default on) | Behaviour [D, release notes September 25, 2026]. Microsoft does not say it is user-switchable; the switch and its label are Axentra's [U] |

Linux implementation: desktop notifications go through `org.freedesktop.Notifications` with action buttons (§8.4). "When closed" requires an Axentra background service (systemd user unit or autostart entry). If the app does not run in the background, disable the toggle and explain why.

Sounds (resolves the earlier open question): by default use the **freedesktop sound theme** by name, passed as the `sound-name` hint: `message-new-email` for new mail and `alarm-clock-elapsed` for reminders [D: sound naming spec; both are standard names]. Bundle a **small Axentra-owned set** (2-4 CC0 or commissioned sounds) because many desktops ship a minimal or empty sound theme; play a bundled file through the `sound-file` hint when the user picks one, and allow a user-chosen file (parity with custom sounds). Never ship or imitate Microsoft's notification sounds: Microsoft's trademark guidelines list **sounds** among its protected brand assets [D].

#### Accessibility [D, Outlook on the web: Settings > General > Accessibility > Keyboard shortcuts, then Save; the rest U]

| Control | Options |
|---|---|
| **Keyboard shortcuts** | **Outlook** (default), **Outlook.com**, **Gmail**, **Yahoo Mail**, **Turn off keyboard shortcuts**. Outlook on the web documents the Outlook.com, Yahoo Mail, Gmail and Outlook sets plus an off choice [D, web]; whether new Outlook for Windows shows the same picker is not documented [U]. Axentra v1 ships the Outlook set and keeps the others as table-driven extensions |
| **Show key tips when Alt is pressed** | On (default) [D: key tips setting, June 2025; label U] |
| **Reduce motion** | Follow system (default; portal `reduced-motion` key, then GTK `gtk-enable-animations`) / On / Off (Axentra) |

#### Search [3P]

| Control | Options |
|---|---|
| **Search results** scope default | **All folders**, **Current folder**, **Current mailbox** (default All folders) [3P] |
| **Include results from Deleted Items** | Checkbox (default off) [U] |
| **Search history** | **Clear history** button [U] |

#### Offline [D]

| Control | Options / default |
|---|---|
| **Enable offline email, calendar, and people** | Switch, default **On** [D] |
| **Folders to save** | **Default (Inbox, sent, drafts, archive, deleted and snoozed)**; **Default and favorites** (default); **Default, favorites, and recently used** [D] |
| **Days of email to save** | **7**, **30** (default since May 2025; was 7), **90**, **180** [D]. Roadmap 560535 (in development) adds **1 year** and **2 years**; build the select from a list so they can be added [D]. Roadmap 570069 (in development) lets tenant admins set and lock this value [D] |
| **Sync in the background when the app is closed** | Switch, default On. The reference syncs in the background since April 2026 and documents turning it off on this page [D]; the exact label is [U] |
| Storage readout | "Using 1.2 GB on this device" plus a **Clear offline data** button (Axentra) [U] |

#### Language and time [D]

**Language (Country/Region)** dropdown [D], **Date format**, **Time format**, **Current time zone** [U]. Changing the language prompts for a restart of the UI (soft reload, estimate).

### 7.4 Mail: Layout (or Message list + Reading pane)

What the docs say (checked 2026-10-19):
- **Mail > Message list**: the Focused Inbox article, which already uses the current "View > View Settings" wording, for both new Outlook and the web [D].
- **Mail > Reading pane**, with **Message organization** on it: the conversations article [D].
- **Mail > Layout** (one page holding Focused Inbox, Message organization, Message list format, Reading pane and the after-move choice): the message-list display article, the Outlook.com "Change the look" article and a Microsoft moderator answer from June 2026 [D].

Reading: the single Layout page is being (or has been) split into **Message list** and **Reading pane**; articles are updated at different times. Axentra uses **two pages**, **Message list** and **Reading pane**, and registers "Layout" as a settings-search alias for both. Following the conversations article, **Message organization** sits on the Reading pane page. Re-check against a licensed install before UI freeze (§13.3).

**Message list** page:

| Section | Control | Options / default | Src |
|---|---|---|---|
| **Focused Inbox** (per account) | Checkbox | **Sort messages into Focused and Other** (default on for supported accounts), then Save [D]. Older Outlook.com wording was a radio with **Don't sort my messages** [D, older]. Only Inbox is affected. Supported only for Microsoft 365, Exchange and Outlook.com accounts; not for shared mailboxes (by design) [D]. For IMAP, POP, Gmail and other accounts show it disabled with the reason | [D] |
| **Message list format** | Radio | **Sender name first** (default [U]), **Subject first** | [D] labels |
| **Text preview** | Checkbox + select | **Show preview text** (default on); preview length **2 lines / 1 line / No extra line** (default 1 line) | [D] behaviour (July 2026, roadmap 564610) / [U] labels and default |
| **Sender image** | Checkbox | **Show sender images** (default on in Roomy) | [U] label |
| **Attachment previews** | Checkbox | **Show attachment previews** (default on in Roomy) | [U] label |
| **Density** | Link | "Change density in Appearance" | [U] |

**Reading pane** page:

| Section | Control | Options / default | Src |
|---|---|---|---|
| **Message organization** | Radio "How do you want your message list organized?" | **Group messages by conversation** (default [U]), **Group messages by branches within conversations**, **Do not group messages** | [D] labels and placement (conversations article) |
| **Reading pane position** | Radio | **Right** (default [U]), **Bottom**, **Fill screen** (no persistent pane; the message replaces the list), **Popout only** (messages open in their own window) | [D] labels (new Outlook) |
| **After moving or deleting an item** | Radio | **Open the next item**, **Open the previous item**, **Return to the message list** (default for Fill screen) | [D] labels (new Outlook only; the web section does not list it) |
| **Conversation contents** | Radio "What do you want to see in your Reading Pane?" | **Show all messages from the selected conversation** (default) / **Show only the selected message** | [D] question; [U] option labels |
| **Conversation order** | Radio | **Newest on top** (default), **Newest on bottom** | [U] for new Outlook. The only Microsoft source is the legacy Outlook Web App "Display settings" page (Exchange-era OWA), which offers newest or oldest on top; classic Outlook always sorts newest on top within a conversation [D, classic] |
| **Deleted items in conversations** | Radio "Do you want to see deleted items in your conversations?" | **Show deleted items** / **Hide deleted items** (default) | [D] question; [U] option labels |

**Message handling** page (mark-as-read part only, since it affects the shell's unread counts). Path: Settings > Mail > Message handling > **Mark as read**, then Save [D]. Microsoft does not publish the option labels or the default for new Outlook; all labels below are ours:

| Option | Notes | Src |
|---|---|---|
| **Mark as read when the item is opened in the reading pane** (default) | | [U] |
| **Mark as read after N seconds in the reading pane** (number box, 0-60, default 5) | Classic Outlook has the same idea ("Wait _ seconds before marking item as read") [D, classic: File > Options > Advanced > Outlook panes > Reading Pane] | [U] |
| **Mark as read when the selection changes** | Classic label: "Mark item as read when the selection changes" | [D, classic] / [U] |
| **Don't automatically mark items as read** | | [3P] |

**Compose and reply** (only the part the shell owns): **Undo send** delay. The maximum went from 10 to 30 seconds in January 2026 [D]. Axentra: a slider with stops **Off**, **5**, **10**, **15**, **20**, **25**, **30** seconds, default **10** (estimate for the default; [3P] for the 5/10/15/30 steps).

**Folder pane** page (the page exists since September 2026 [D]; its contents are not documented): **Folder order** (Custom folder order / A to Z), **Show favorites section**, **Show unread counts** (Inbox only / All folders / Off) [U]. Related documented behaviour: the pinned Favorites section can be resized (September 2026) [D]; folders can be reordered by drag and given icon colours (2025) [D].

### 7.5 Parity between View-tab controls and Settings

| Setting | View tab control | Settings location | Scope |
|---|---|---|---|
| Reading pane position | Reading pane | Mail > Reading pane | Global (estimate) |
| Density | Density | General > Appearance [3P] | Global |
| Conversations grouping | Conversations | Mail > Reading pane > Message organization [D, conversations article] | Global (estimate) |
| Conversation order | Conversations (bottom section) | Mail > Reading pane | Global |
| Preview text and lines | Message preview | Mail > Message list > Text preview | Global |
| Sender images | Sender image | Mail > Message list | Global |
| Folder pane shown | Folder pane | (none; window state) | Per window |
| Focused Inbox | **none** in new Outlook. Classic Outlook has **Show Focused Inbox** on the View tab [D] | Mail > Message list [D] | Per account |
| Ribbon layout | Layout switch | (none; window state) | Per window |
| Classic look | (none) | General > Appearance (Axentra) | Global |

Changing a value in one place immediately updates the other. If the Settings dialog is open with unsaved edits to the same key, the dialog shows a non-blocking banner "This setting was changed elsewhere", and Save overwrites (estimate).

### 7.6 People: Contacts page [D]

**Sort contacts by**: **First name** / **Last name**. **Display contacts as**: "First Last" / "Last, First" [U for the second control].

---

## 8. Feedback surfaces

### 8.1 Undo bar (after delete, move, archive and similar)

- Shown after any reversible action: Delete, Archive, Move to, Report junk, Pin/Unpin [D: pin and unpin are undoable], Flag, Categorize, Snooze, Sweep (single run), Mark read/unread on several items.
- Placement: bottom-left of the content area, 16px from the rail edge and 16px from the bottom (estimate). Width auto, from 280px to 480px (estimate). Fluent Toast styling [F]: `colorNeutralBackground1`, `shadow8`, 12px padding, 1px `colorTransparentStroke` border, radius `borderRadiusMedium` (4px). Axentra rounds the undo bar to 8px and pads it 12px × 16px to match its flyouts (estimate; a deliberate departure from Fluent's 4px).
- Content: 16px status icon + text in Body 1 + an **Undo** link button (Body 1 Strong, `colorBrandForegroundLink`) + a dismiss ×.
  - Text patterns (Axentra copy): "Deleted 1 item", "Deleted 12 items", "Moved to Archive", "Moved to {Folder}", "Reported as junk", "Pinned", "Snoozed until {when}", "Marked as read".
- Lifetime: 6s (estimate). The timer pauses while the pointer is over the bar or focus is inside it. A new reversible action **replaces** the bar, and the previous action stays undoable with Ctrl+Z.
- **Ctrl+Z** undoes the last action, even after the bar is gone [D]. Redo of list actions is **Ctrl+Shift+Z** in Axentra, because outside text editing new Outlook uses Ctrl+Y for Go to folder (§10) [D keys / U resolution]. Keep an undo stack of 20 actions per session (estimate). Navigating to another account clears it.
- On server failure after an optimistic action, revert the UI and show an error toast: "Couldn't move 3 items. Try again." with a **Retry** button.
- Accessibility: `role="status"`, `aria-live="polite"`. The Undo button is reachable with **F6** (the bar counts as a region while visible) and **Ctrl+Z**.

### 8.2 Undo send bar

- After Send with a non-zero delay: the bar shows "Sending…" plus a countdown ring (16px) and **Undo**. Clicking **Undo** reopens the draft [3P]. It works offline [D, June 2025].
- It follows the same placement as the undo bar and stacks **above** it if both are visible (8px gap). It never auto-dismisses before the delay ends; when the delay ends it changes to "Sent" for 2s (estimate).

### 8.3 In-app toasts (non-undo)

Fluent Toast pattern, bottom-left stack, newest at the bottom, at most 3 visible (estimate).

| Intent | Icon | Accent | Default lifetime (estimate) |
|---|---|---|---|
| Info | `info` in `colorNeutralForeground2` [F: Fluent ToastTitle info intent] | none | 6s |
| Success | `checkmark_circle` in `colorStatusSuccessForeground1` [F] | none | 4s |
| Warning | `warning` in `colorStatusWarningForeground1` [F] | none | 8s |
| Error | `error_circle` in `colorStatusDangerForeground1` [F] | none | stays until dismissed |

On an inverted toast (`colorNeutralBackgroundInverted`), Fluent switches the icons to the `…ForegroundInverted` status tokens (§1.5) and info to `colorNeutralForegroundInverted2` [F].

New-mail toasts inside the app (when the window is focused and the message is not visible): top-right below the header, 360px wide, showing avatar, sender, subject and preview, with hover actions **Reply**, **Flag**, **Delete** (estimate). These are off when **Desktop notifications** is on and the window is not focused, to avoid duplicates.

### 8.4 Desktop (OS) notifications

- Use `org.freedesktop.Notifications.Notify`. App name "Axentra Outlook". Icon: the Axentra app icon. Summary = sender (mail) or event title. Body = subject plus snippet (mail) or time and location (calendar).
- Actions: Mail: `default` (open), **Reply**, **Mark as read**, **Delete**. Calendar: `default`, **Snooze** (5 min), **Dismiss**, **Join** (if there is an online meeting URL). Hints: `category` = `email.arrived` / `x-axentra.calendar.reminder` (vendor categories take the form `x-vendor.class.name`), `desktop-entry` = the app ID, `sound-name` or `sound-file` per the Notifications settings.
- Batching: several new messages within 10s (estimate) combine into "{n} new messages" [D: summary alerts, September 2026].
- Respect the desktop's do-not-disturb (the server may drop notifications; never fall back to in-app toasts in that case).

### 8.5 Message bars (inline, persistent)

Fluent MessageBar [F]: min height 36px, 12px (`spacingHorizontalM`) left padding, radius 4px, 1px border. Fills and borders per intent: info `colorNeutralBackground3` / `colorNeutralStroke1`; error, warning and success `…Background1` / `…Border1`. Icon colours per intent: info `colorNeutralForeground3`, error `colorStatusDangerForeground1`, success `colorStatusSuccessForeground1`, **warning `colorStatusWarningForeground3`** (not Foreground1; in dark mode that is `#F98845` rather than `#FAA06B`). Text Body 1 `colorNeutralForeground1`. Full width of the owning pane, optional action buttons on the right and a dismiss × (estimate). Used for: offline (§9.3), sync errors, account needs attention, Focused Inbox hints, encrypted to non-encrypted transitions [D: informational banner, February 2026].

---

## 9. Loading, empty, offline and error states

### 9.1 Loading

| Situation | Treatment |
|---|---|
| App start (cold) | Window opens immediately with header and rail painted, the ribbon in a skeleton state and panes as skeletons. No full-screen splash. Show content within 1s for cached accounts (estimate) |
| App start (first run / no cache) | Centred card: Axentra icon 64px, text "Setting up your mailbox…" (Subtitle 2), a 2px indeterminate brand progress bar 240px wide (estimate) |
| Module switch (not cached) | Skeleton rows in the list (`colorNeutralStencil1` blocks, shimmer gradient from `colorNeutralStencil1` to `colorNeutralStencil2` over 1.5s linear, infinite (estimate)) |
| Background refresh / Sync | A thin 2px indeterminate bar at the top edge of the message list card. The header **Sync** item shows "Syncing…" and is disabled until done (estimate) |
| Long operation (export, sweep run) | Determinate progress in a toast with **Cancel** |
| Spinner sizes | Fluent Spinner [F]: extra-tiny 16, tiny 20, extra-small 24, small 28 (2px stroke); medium 32, large 36, extra-large 40 (3px stroke); huge 44px (4px stroke). Use medium in panes and tiny inline (estimate). Fluent's spinner already has a `prefers-reduced-motion` variant |

Skeletons appear only if loading takes longer than 300ms (estimate), to avoid flashing.

### 9.2 Empty states

| Surface | Icon (24px, `colorNeutralForeground3`) | Title (Subtitle 2) | Body (Body 1, `colorNeutralForeground3`) |
|---|---|---|---|
| Empty folder | `mail_inbox` | "All done for the day" | "Enjoy your empty inbox." (Inbox) / "Nothing in this folder" (others) |
| Focused tab empty | `mail_inbox` | "You're all caught up" | "Check Other for more messages." with a link that switches to Other |
| Search, no results | `search` | "We didn't find anything" | "Try different words or change the search scope." + **Search all folders** button |
| Notifications pane | `alert_badge` | "You're all caught up" | (none) |
| No accounts | `person_add` | "Add an account to get started" | primary button **Add account** |
| Rail module unavailable for this account | `info` | "{Module} isn't available for this account" | "Switch to an account that supports it." |

All copy is Axentra's own wording. Engineers must keep the strings in the i18n catalogue.

### 9.3 Offline

- Detection: NetworkManager D-Bus `Connectivity` (Full / Limited / Portal / None) plus a failed-request heuristic (3 consecutive network failures within 30s, estimate). Clear the offline state on the first successful request.
- Indicators:
  1. Header: an **Offline** pill (Caption 1 Strong, `--ax-offline-pill-bg`, radius circular, 20px tall) right after the app name, with tooltip "You're offline. Changes will sync when you reconnect." (Axentra copy; the reference does not document a banner).
  2. A one-time warning-intent MessageBar at the top of the message list: "You're offline. You can keep reading and working with saved items." with a dismiss ×.
- Behaviour while offline [D: Work offline article, corrected by later release notes where they conflict]:
  - Works: read, compose, reply, forward (queued to Outbox), drafts, adding attachments and inline images to drafts (May 2026), opening synced attachments (August 2025), search of saved items, folder create/rename/move/delete, viewing search folders and shared mailboxes that were added as accounts, archive, categorize, copy to folder, delete, flag, ignore, read/unread, move, move to Focused/Other, pin, snooze, report junk / not junk, report phishing (all queued), RSVP, undo send, Focused/Other views (since August 2026; the support article still lists Other as "planned"), viewing calendars and people, deleting and cancelling events (February 2026) and creating/editing events (roadmap 490059, launched), and these settings pages: About, Automatic replies, Categories, Email accounts, People view, Signatures (July 2025).
  - Does not work (disable with the "(not available offline)" tooltip): changing sort order, notification and calendar settings, shared folders, email and calendar groups, favourite people, creating/editing/deleting contacts, the full global address list, add-ins.
  - Unsupported actions attempted anyway show an error toast **or** revert to the last saved state [D].
  - Low disk space: the reference saves fewer items, and may turn offline access off entirely [D]. Axentra shows the §9.4 warning instead of silently disabling.
- Queued sends sit in the Outbox with a clock badge. On reconnect they send automatically and a success toast says "{n} messages sent".
- Starting the app offline: the reference historically could not start offline [3P]. Axentra **must** start offline from the local store and show the indicators above.

### 9.4 Error states

| Error | Surface |
|---|---|
| Authentication expired | Danger MessageBar in the affected account's folder pane section: "{Account}: sign-in needed" + **Fix** (opens Settings > Accounts) |
| Server throttling (HTTP 429/503) | Silent retry with backoff. After 2 minutes (estimate), a warning MessageBar "Having trouble connecting to {account}" |
| Storage full (offline cache) | Warning toast: "Not enough space to save all items offline" + **Change offline settings** [D: fewer items get saved] |
| Crash recovery | On restart after a crash: an info MessageBar "Axentra Outlook closed unexpectedly. Your drafts were restored." |

---

## 10. Global keyboard map (shell-owned)

| Action | Shortcut | Src |
|---|---|---|
| Go to Mail | Ctrl+1 | [D] |
| Go to Calendar | Ctrl+2 | [D] |
| Go to Copilot (reference) / **unbound** (reserved) in Axentra | Ctrl+3 | [D] |
| Go to People | Ctrl+4 | [D] |
| Go to To Do / Groups / Files | Ctrl+5 / Ctrl+6 / Ctrl+7. Ctrl+5 = To Do matches current classic Outlook [D, classic]; Ctrl+6 / Ctrl+7 are Org Explorer / Loop in classic, so Groups / Files there are Axentra extensions | [D]/[U] |
| Web aliases | Ctrl+Shift+1 / 2 / 4 (Outlook on the web's module keys [D, web]) accepted as silent aliases; Ctrl+Shift+3 unbound | [U] |
| Go to the folder pane / Go to folder dialog | Ctrl+Y: the shortcut page labels it "Go to the folder pane"; What's new (August 2024), the release notes (August 14, 2026) and roadmap 559801 describe it opening **Go to folder**, a type-ahead folder search, also reachable from the account's context menu [D] | [D] |
| New item for the current module | Ctrl+N | [D] |
| Open selected item in a new window | Shift+Enter | [D] |
| Run quick step 1-5 | Ctrl+Shift+5 … Ctrl+Shift+9 (as assigned per step) | [D, Quick steps article] |
| Focus search | Alt+Q; Ctrl+E | [D, web] / [D, People; classic mail search] |
| Cancel search | Esc | [D] |
| Undo / Redo | Ctrl+Z / Ctrl+Y. The new-Outlook shortcut page lists Ctrl+Y twice: "Go to the Folder pane" and, in the text-editing group, repeat/redo [D]. Axentra: Ctrl+Y = Redo only while a text editor (compose, settings fields) has focus; everywhere else Ctrl+Y = Go to folder. Redo of list actions (delete, move…) uses **Ctrl+Shift+Z** (Axentra extension) | [D] keys / [U] resolution |
| Show keyboard shortcuts | ? (when focus is not in a text field); Shift+? in People lists People shortcuts | [D] |
| Key tips | Alt | [D] |
| Next / previous region | F6 / Shift+F6 (also Ctrl+Shift+Tab, documented for the calendar) | [D] |
| Open Settings | Ctrl+, (Axentra extension) | [U] |
| Toggle My Day / Notifications | none by default | [U] |
| Zoom the reading pane | Ctrl+= / Ctrl+- / Ctrl+0 (Axentra extension, scoped to the reading pane) | [U] |
| Calendar: Day / Work week / Month | Ctrl+Alt+1 / Ctrl+Alt+2 / Ctrl+Alt+4 or Alt+= | [D] |
| Calendar: Week | Ctrl+Alt+3 in Axentra. The docs list both Ctrl+Alt+2 and Alt+- for full week, which conflicts | [D] conflict / [U] |
| Calendar: Go to today | Alt+Shift+Y or Home | [D] |

Classic Outlook: the current Microsoft 365 shortcut page lists Ctrl+1 Mail, Ctrl+2 Calendar, Ctrl+3 Copilot, Ctrl+4 People, Ctrl+5 To Do, Ctrl+6 Org Explorer, Ctrl+7 Loop, Ctrl+8 Tasks [D, classic]. Older perpetual versions used Ctrl+3 Contacts, Ctrl+4 Tasks, Ctrl+5 Notes, Ctrl+6 Folder list, Ctrl+7 Shortcuts [3P, older versions]. (An earlier draft gave only the older map.)

Other documented new-Outlook keys owned by module specs, listed here so the shell does not bind them: Ctrl+< / Ctrl+> previous/next message, Home / End top/bottom of a conversation, Ctrl+Alt+Left / Right previous/next calendar period, Ctrl+F forward (shortcut page) vs find in the reading pane (release notes August 2025) [D].

The **?** dialog: a modal (720×560, estimate) listing shortcuts by category, generated from the same keymap table the input handler uses. The text search filters it. The table carries the source tags so docs do not drift.

---

## 11. Differences from classic Outlook (for reference only)

| Area | New Outlook / Axentra | Classic Outlook for Windows |
|---|---|---|
| Settings | Gear → Settings dialog [D] | File → Options (Backstage) |
| File menu | File **ribbon tab** with Print, Manage accounts and templates [D] | Full Backstage view |
| Density | View > Density: Roomy / Cozy / Compact [D] | View > Change View: Compact / Single / Preview [3P] |
| Reading pane off | **Fill screen** / **Popout only** [D] (Outlook.com article: **Hide**) | Reading Pane > **Off** [3P] |
| Focused Inbox toggle | Settings > Mail > Message list, checkbox "Sort messages into Focused and Other" [D] | View tab > **Show Focused Inbox** [D] |
| Conversations | Group by conversation / by branches / do not group [D] | View > Show as Conversations (scope: All mailboxes or This folder); Conversation Settings: Show Messages from Other Folders, Show Senders Above the Subject, Always Expand Selected Conversation, Use Classic Indented View [D]. Within a conversation, newest is always on top [D] |
| Mark as read | Settings > Mail > Message handling > Mark as read [D] | File > Options > Advanced > Outlook panes > Reading Pane [D] |
| Ribbon customisation | Simplified: Mail only, per account [3P]; Classic: right-click the ribbon, Home and View tabs, rolling out from October 2026 [D] | Full Customize the Ribbon dialog, any tab |
| Navigation | Vertical app rail on the left, fixed [3P] | Navigation bar; File > Options > Advanced > Outlook panes > **Show Apps in Outlook** moves it to the left (clear it to put it below the folder pane); Microsoft says it will eventually be fixed on the left [D] |
| Module keys | Ctrl+1 Mail, Ctrl+2 Calendar, Ctrl+3 Copilot, Ctrl+4 People [D] | Same, plus Ctrl+5 To Do, Ctrl+6 Org Explorer, Ctrl+7 Loop, Ctrl+8 Tasks [D] |
| Key tips | Alt (since August 2024; before that Win+Alt) [D] | Alt |
| Sort/view setup | Separate Sort and Filter buttons in the list header [D] | View Settings dialog (Columns, Group By, Other Settings) |

---

## 12. Implementation notes (do not miss)

1. **Tokens are generated, not hand-typed.** Write a small Rust build script or `xtask` that emits `palette.css` (ramps) and `theme-{light,dark}.css` (alias maps) from one data file. Each alias token is a reference such as `brand[80]` or `grey[14]`. That keeps §1.4 and the code in sync and makes new colour themes a one-line change. Components use only alias and `--ax-` variables.
2. **Theme switching is attribute-driven.** `<html data-mode="dark" data-theme="teal" data-density="compact" data-chrome="cards|flat" data-contrast="normal|high">`. Switching must not re-render the Dioxus tree; it only flips CSS variables. Persist the mode as `light | dark | system`; resolve `system` at runtime through the XDG portal and re-resolve when it signals a change.
3. **Fonts.** Bundle Selawik (OFL, unmodified, or renamed if subset; §1.7) and refer to Segoe UI only through `local()`. Never ship Segoe files (Microsoft's trademark guidelines also list fonts among its brand assets [D]). Check that line heights match the §1.7 ramp under Selawik; set `line-height` explicitly on every text style so fallback fonts cannot shift the layout. Selawik is Latin-only, so test Cyrillic, Greek and CJK subjects with the Noto fallback.
4. **Client-side decorations.** The header is the title bar. Use `decorations(false)` on the window. Mark drag regions and call the window drag API on primary-button mouse-down in non-interactive header areas only. Read the GNOME/GTK button layout (`org.gnome.desktop.wm.preferences button-layout`) to put the window controls left or right. Support double-click to maximise, the desktop's middle-click action, and Super+Up/Down from the compositor. Draw a 1px `colorNeutralStroke2` window border and use 8px corner radius when not maximised (estimate) if the compositor does not.
5. **Focus visibility.** Use `:focus-visible` only. Clicking must not show rings. Every interactive element needs a `position: relative` host for the `::after` ring. Inside `overflow: hidden` containers, use the inset variant.
6. **F6 regions.** Implement a region manager that knows the order Header → Rail → Ribbon → Folder pane → List → Reading pane → open side pane → visible toast. Each region remembers its last focused element.
7. **Ribbon is data-driven.** Commands are declared once (id, label, icon, shortcut, key tip, enablement predicate, menu items, priority, group). Simplified, Classic, overflow menus, the Customize dialog, the shortcut sheet and context menus all render from that one table. Customisation is stored per account as an ordered list of command ids per group.
8. **Never hide commands for state.** Disable instead (§6.3), so muscle memory and positions stay stable. The exceptions are capability-gated commands (Policy, Groups, Meet now), which are absent when the account cannot support them.
9. **Settings model.** A typed `Settings` struct with three scopes: global, per account and per window. View-tab controls write the same keys as the Settings pages (§7.5). Settings pages edit a draft copy and commit on **Save**. The View tab commits immediately. Store the file under `$XDG_CONFIG_HOME/axentra-outlook/` and keep server-synced settings (Focused Inbox, automatic replies, signatures for Exchange) in the account layer with conflict handling.
10. **Offline first.** Every ribbon command declares `offline: Supported | Queued | Unsupported`. The shell disables Unsupported ones while offline. The app must start offline from the local store (the reference historically could not).
11. **Optimistic actions with undo.** Delete, move and similar actions update the UI first, queue the server call, and register an undo closure. Failures revert and toast. The undo stack is per session, 20 entries (estimate).
12. **Notifications.** Use `org.freedesktop.Notifications` with actions and handle `ActionInvoked` and `NotificationClosed`. Background notifications need a separate lightweight process or a "keep running in tray/background" option. Linux has no universal tray, so prefer the Background portal (`org.freedesktop.portal.Background`) under Flatpak.
13. **Reduced motion and high contrast.** Read the portal `org.freedesktop.appearance` keys `reduced-motion` and `contrast` first, then GTK `gtk-enable-animations`, then `prefers-reduced-motion`. Provide the explicit High contrast setting, because no OS forced-colours mode reaches WebKitGTK on Linux (§1.15).
14. **RTL.** Mirror the rail (to the right edge), the ribbon order, the overflow position, the toast placement and directional icons (`_ltr`/`_rtl` variants). Do not mirror media controls or the clock.
15. **Copy and branding.** All UI strings in this spec that are Microsoft labels are factual UI labels used for parity. Product-specific names become Axentra names: "About Axentra Outlook", "Send notifications when Axentra Outlook is closed". Never show Microsoft logos, icons, illustrated theme art or sounds; Microsoft's trademark guidelines say its logos and icons may not be used in any manner and list trade dress, fonts and sounds among its brand assets [D]. Keep the product name in **one constant** (`PRODUCT_NAME`, plus the app ID, desktop file, D-Bus names and config directory derived from it) because the name may have to change (§13.8).
16. **Estimates are constants.** Every "(estimate)" number lives in `shell_metrics.rs` (or `--ax-` CSS variables) with a comment pointing to this section, so design review can tune them without hunting through components.
17. **HiDPI.** Use CSS px throughout and let WebKitGTK scale. Test at 100%, 125% (fractional on Wayland), 150% and 200%. Icons must come from SVG sources, never raster.
18. **Accessibility roles.** Header `role="banner"`. Rail `<nav aria-label="Apps">` with `aria-current="page"` on the active module. Ribbon `role="toolbar"` inside a `tablist`/`tabpanel` pair. Settings `role="dialog" aria-modal="true" aria-labelledby`. Toasts and the undo bar `role="status"`. Error toasts `role="alert"`. Badges carry `aria-label="{n} unread notifications"`.

---

## 13. Open questions

Status after the 2026-10-19 fact-check. "Decided" means the spec now carries a decision; "Open" means a human or a licensed install is still needed.

1. **Classic-style UI (October 2, 2026). Open (partly decided).** The release notes say only that new Outlook "now offers" a UI style closer to classic Outlook [D]. No Microsoft page, What's new entry or roadmap item (feed checked 2026-10-19) names a toggle, a location or a default. The wording "offers" suggests an option rather than a forced default, but that is our reading [U]. Decided for Axentra: an opt-in **Classic look** switch in General > Appearance, default off, implemented as `data-chrome="flat"` (§1.15, §7.3). Rename it to match Microsoft's label once one is documented.
2. **Ctrl+3. Decided:** unbound and reserved (§4.1, §10). Ctrl+4 is People in new Outlook, in current classic Outlook and in Axentra [D], so parity costs nothing.
3. **Simplified-ribbon labels, visibility and order. Open; answer is yes.** Microsoft does not publish them; every [U] row in §6.3-§6.9 must be checked against a licensed, current-channel install of new Outlook (and Outlook on the web) before UI freeze. Method: observe and write down labels, menu items, default visibility and order by hand from the running UI, noting the build and date; do not decompile, scrape or copy resources. Record findings by changing tags from [U] to "[O, build, date]" (observed) rather than [D].
4. **Header buttons after the October 2026 overflow change. Open (decided provisionally).** Microsoft does not list which actions are "secondary" [D]. Axentra keeps My Day, Notifications and Settings standalone (the only header buttons named in current support articles) and shows Meet now only when a meeting provider is configured (§3.1). Verify on a Teams-enabled licensed install.
5. **Default density. Decided: Cozy** [3P]. Microsoft documents the three names but not the default; third parties report Medium/Cozy as the default. Keep it a constant so it can change.
6. **Settings layout. Decided, with evidence (§7.4):** the current Focused Inbox article uses **Mail > Message list**, the conversations article uses **Mail > Reading pane** (with Message organization on it), and older articles and a June 2026 Microsoft moderator answer still say **Mail > Layout** [D]. Axentra ships Message list + Reading pane with "Layout" as a search alias. Confirm on a licensed install (item 3).
7. **Groups and Files. Open (product decision); facts gathered.** Microsoft 365 group conversations via Graph (`GET /groups/{id}/conversations`) need delegated `Group-Conversation.Read.All` (or `Group.Read.All` / `Group.ReadWrite.All`), and **all of these require tenant admin consent**; personal Microsoft accounts are not supported [D: Graph API and permissions reference]. Listing the user's own groups (`GET /me/memberOf`) works with `User.Read` but returns limited data [D]. So Groups cannot work for a work account unless the tenant admin approves Axentra's app registration. Recommendation: v1 ships Groups as a hidden, capability-gated module that appears only when the token actually carries a group-conversation scope; plan the full module for v2. **Files**: recommend v1 = an **attachments hub** built from mail already synced (no new scopes, works for IMAP, offline-capable). A OneDrive browser would need `Files.Read` (no admin consent) or `Files.Read.All` (delegated, no admin consent) [D] and belongs in a later version.
8. **Trademark and UI strings. Open; needs counsel.** Microsoft's Trademark and Brand Guidelines tell third parties not to use Microsoft brand assets, which include product names, in the name of their own product or app, and give "Contoso OneDrive software" as a prohibited example. Truthful compatibility statements ("works with Outlook.com and Microsoft 365") are allowed if less prominent than the own brand and without implying endorsement [D]. **"Axentra Outlook" therefore conflicts with the published guidelines.** Recommendation for legal review: rename the product (for example "Axentra Mail") and use a compatibility line in About and store listings; keep the name in one constant (§12.15). Exact UI labels: short functional labels ("Reply all", "Sweep", "Focused Inbox") are used here for parity. Trade dress is a listed brand asset, and "Focused Inbox" and "Sweep" may be feature names Microsoft treats as marks [U], so counsel should also review the label set and the overall look before release.
9. **Notification sounds. Decided (§7.3 Notifications):** use freedesktop sound-theme names by default (`message-new-email`, `alarm-clock-elapsed` [D]), bundle a small Axentra-owned set, and allow user-chosen files. Never use Microsoft's sounds, which the guidelines list as brand assets [D].
10. **New (from roadmap):** Unified inbox (564609), multi-account search (570441), 1-year/2-year offline windows (560535) and admin-locked settings (570069, 571199) are in development in the reference [D]. Decide whether Axentra v1 reserves UI space for them (search scope "All accounts", folder-pane "All accounts" node) or adds them later.

---

## Sources

- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/global/brandColors.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/global/colors.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/alias/lightColor.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/alias/darkColor.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/alias/lightColorPalette.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/alias/darkColorPalette.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/statusColorMapping.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/sharedColorNames.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/global/fonts.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/global/borderRadius.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/global/spacings.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/global/strokeWidths.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/utils/shadows.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/global/durations.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/global/curves.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/react-components/react-tabster/src/focus/createFocusOutlineStyle.ts
- https://github.com/microsoft/fluentui-system-icons
- https://fluent2.microsoft.design/typography
- https://fluent2.microsoft.design/elevation
- https://fluent2.microsoft.design/accessibility
- https://learn.microsoft.com/en-gb/typography/font-list/selawik
- https://wiki.archlinux.org/title/Metric-compatible_fonts
- https://www.geekwire.com/2015/microsoft-selawik-is-a-new-alternative-to-a-key-windows-font-and-its-open-source/
- https://learn.microsoft.com/en-us/officeupdates/release-notes-outlook-new
- https://support.microsoft.com/en-us/outlook/getstarted/what-s-new-in-new-outlook-for-windows
- https://support.microsoft.com/en-us/outlook/getstarted/getting-started-with-the-new-outlook-for-windows
- https://support.microsoft.com/topic/66b195df-08b9-48bd-b464-d6edf95813e4
- https://support.microsoft.com/office/change-the-look-of-your-mailbox-in-outlook-com-and-outlook-on-the-web-b41c2ecb-f23c-42b3-b7f8-659646d5e58c
- https://support.microsoft.com/en-us/topic/b5eaf108-0a37-4750-ae79-24087eecde77
- https://support.microsoft.com/en-us/outlook/mail/change-how-the-message-list-is-displayed-in-outlook
- https://support.microsoft.com/en-US/Outlook/mail/view-email-messages-by-conversation-in-outlook
- https://support.microsoft.com/en-us/outlook/use-and-configure-the-reading-pane-to-preview-messages-in-outlook
- https://support.microsoft.com/en-us/outlook/mark-a-message-as-read-or-unread-in-outlook
- https://support.microsoft.com/help/f714d94d-9e63-4217-9ccb-6cb2986aa1b2
- https://support.microsoft.com/en-us/office/keyboard-shortcuts-for-outlook-3cdeb221-7ae5-4c1d-8c1d-9e63216c1efd
- https://support.microsoft.com/en-us/office/customize-the-navigation-bar-35a16e69-24a2-48e5-831f-77d596a5dd20
- https://support.microsoft.com/office/use-my-day-with-to-do-in-outlook-6de6ee52-751b-4405-b389-850572b15306
- https://support.microsoft.com/office/work-offline-in-outlook-2460e4a8-16c7-47fc-b204-b1549275aac9
- https://support.microsoft.com/en-US/outlook/getstarted/turn-new-message-alert-pop-up-on-or-off-in-outlook
- https://support.microsoft.com/en-us/office/organize-your-inbox-with-archive-sweep-and-other-tools-in-outlook-on-the-web-49b26f63-6399-4b4a-a580-14b9b1efe96d
- https://support.microsoft.com/en-us/outlook/calendar/change-how-you-view-your-outlook-calendar
- https://support.microsoft.com/office/create-view-and-edit-contacts-and-contact-lists-in-outlook-com-5b909158-036e-4820-92f7-2a27f57b9f01
- https://support.microsoft.com/en-au/office/quick-steps-in-outlook-d8efbdbd-5098-491b-9d2e-8c7b6f45a9b4
- https://support.microsoft.com/en-us/outlook/display-settings-for-mail
- https://learn.microsoft.com/en-us/answers/questions/5922085/conversation-settings
- https://learn.microsoft.com/en-us/answers/questions/5819271/the-way-my-email-displays-has-changed-how-do-i-cha
- https://learn.microsoft.com/en-us/answers/questions/1503068/in-new-outlook-how-do-i-move-the-app-bar-to-the-bo
- https://devblogs.microsoft.com/microsoft365dev/prepare-your-addin-for-outlook-ribbon-improvements/
- https://mc.merill.net/message/RM570664
- https://techcommunity.microsoft.com/blog/microsoft365insiderblog/new-notifications-pane-in-outlook-helps-you-stay-on-task-/4215376
- https://www.ablebits.com/office-addins-blog/new-outlook-ribbon/
- https://www.ablebits.com/office-addins-blog/outlook-calendar-view/
- https://www.usecarly.com/blog/how-to-change-the-inbox-view-in-outlook/
- https://www.slashgear.com/1952317/how-to-fix-microsoft-outlook-email-view
- https://pureinfotech.com/enable-dark-mode-new-outlook-app-windows-11/
- https://pureinfotech.com/?p=154102
- https://pureinfotech.com/manage-notifications-new-outlook-app-windows-11/
- https://www.elevenforum.com/t/pin-or-unpin-apps-on-navigation-bar-in-outlook-for-windows.31269
- https://office-watch.com/2021/meet-now-buttons-where-you-need-them-in-outlook/
- https://bowdoin.teamdynamix.com/TDClient/1814/Portal/KB/ArticleDet?ID=170909
- https://specifications.freedesktop.org/notification/latest/hints.html
- https://specifications.freedesktop.org/notification/latest/categories.html
- https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html
- https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Background.html

Note on an existing source: `support.microsoft.com/en-us/outlook/display-settings-for-mail` applies to the legacy **Outlook Web App** (Exchange-era OWA), not to new Outlook or today's Outlook on the web. Treat anything taken only from it as [U] for the current product.

### Added in the 2026-10-19 fact-check

Fluent source (exact values re-checked against `master`):
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/global/typographyStyles.ts (Subtitle 1 = 20/28; Body 2)
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/global/colorPalette.ts (status colour families)
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/react-components/react-message-bar/library/src/components/MessageBar/useMessageBarStyles.styles.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/react-components/react-toast/library/src/components/Toast/useToastStyles.styles.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/react-components/react-toast/library/src/components/ToastTitle/useToastTitleStyles.styles.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/react-components/react-tabs/library/src/components/Tab/useTabStyles.styles.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/react-components/react-dialog/library/src/components/DialogSurface/useDialogSurfaceStyles.styles.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/react-components/react-menu/library/src/components/MenuPopover/useMenuPopoverStyles.styles.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/react-components/react-popover/library/src/components/PopoverSurface/usePopoverSurfaceStyles.styles.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/react-components/react-tooltip/library/src/components/Tooltip/useTooltipStyles.styles.ts
- https://raw.githubusercontent.com/microsoft/fluentui/master/packages/react-components/react-spinner/library/src/components/Spinner/useSpinnerStyles.styles.ts
- https://github.com/microsoft/fluentui-system-icons/tree/main/assets (every icon name used in this section exists at 20px Regular)

Microsoft product documentation:
- https://support.microsoft.com/en-us/office/focused-inbox-for-outlook-f445ad7f-02f4-4294-a82e-71d8964e3978 (Mail > Message list; checkbox label; supported account types; Move to Focused/Other)
- https://support.microsoft.com/en-us/outlook/getstarted/what-s-new-in-new-outlook-for-windows (dated feature list back to July 2024: Alt key tips, Ctrl+Y, Modern themes, Mail from template, custom sounds, Shared with me)
- https://learn.microsoft.com/en-us/officeupdates/release-notes-outlook-new (re-read in full; page dated 2026-10-16)
- https://www.microsoft.com/releasecommunications/api/v1/m365 (Microsoft 365 roadmap feed; items 570664, 574115, 564610, 564609, 570441, 560535, 570069, 571199, 557678, 559801, 497546, 491019, 490059)
- https://learn.microsoft.com/en-us/answers/questions/5922085/conversation-settings (Microsoft moderator, June 2026: Settings > Mail > Layout)

Microsoft Graph (Groups and Files decision, §13.7):
- https://learn.microsoft.com/en-us/graph/api/group-list-conversations?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/user-list-memberof?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/permissions-reference (Group-Conversation.Read.All, Group.Read.All, GroupMember.Read.All: admin consent required; Files.Read, Files.Read.All delegated: no admin consent)

Legal and branding (§13.8, §12.15):
- https://www.microsoft.com/en-us/legal/intellectualproperty/trademarks (Microsoft Trademark and Brand Guidelines)

Linux platform:
- https://specifications.freedesktop.org/sound-naming/latest/ (`message-new-email`, `alarm-clock-elapsed`)
- https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html (re-checked: `contrast`, `reduced-motion`, `accent-color` keys)

Third-party (lower confidence):
- https://www.usecarly.com/blog/how-to-change-the-inbox-view-in-outlook/ (June 2026; Full/Medium/Compact names, Medium default, Settings > General > Appearance)
