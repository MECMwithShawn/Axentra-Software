# Axentra Outlook — Account Connectivity Spec (Microsoft Graph, IMAP/SMTP, OAuth, CalDAV/CardDAV)

Status: Draft v1 · Owner: Sync & Accounts · Audience: Rust engineers (Dioxus desktop app, Linux)
File: `docs/spec/protocols.md`

This section defines how Axentra Outlook connects to mail, calendar, contact and task services, how it
authenticates, how it keeps a local SQLite store in sync, and how it protects credentials on Linux. It
also defines the small amount of UI that belongs to account setup and connection state.

Conventions used in this document:

- **(estimate)** marks a number or behaviour that public documentation does not state exactly. It is our
  best judgement, consistent with Fluent 2 and protocol guidance. Treat it as a tunable default, never
  as a documented fact.
- **(verify)** marks a documented-but-volatile fact (dates, provider behaviour) that must be re-checked
  with a smoke test or the cited page before release.
- **(unverified)** marks a claim that no first-party (Microsoft / Google / IETF / freedesktop) page
  confirmed during the 2026-10-19 fact-check pass. It rests on community reports, third-party library
  docs, or our own reasoning. Do not ship behaviour that depends on it without a smoke test.
- Fact-check pass 2026-10-19: claims without a tag were checked against the pages in **Sources**
  (Microsoft Learn pages dated up to Oct 2026). Corrections from that pass are written in place.
- `Code style` is used for wire-level names: HTTP paths, JSON fields, IMAP commands, D-Bus names.
- Graph paths are relative to `https://graph.microsoft.com/v1.0`. Do not use `/beta` in shipped code.
- All timestamps in SQLite are UTC Unix epoch milliseconds (`INTEGER`) unless noted.

---

## 1. Scope and provider matrix

### 1.1 What this spec covers

| In scope | Out of scope (handled elsewhere or not supported) |
|---|---|
| Microsoft 365 work/school and personal Microsoft accounts (Outlook.com, Hotmail, Live) via Microsoft Graph | Exchange Web Services (EWS) — Microsoft has announced its retirement for Exchange Online; do not build on it |
| OAuth 2.0 for Microsoft identity platform and Google | MAPI/HTTP, Exchange ActiveSync |
| Generic IMAP4rev1/IMAP4rev2 + SMTP submission (incl. Gmail, Outlook.com via IMAP as fallback) | POP3 (not supported in v1; decision) |
| CalDAV/CardDAV for non-Microsoft calendars and contacts | Graph change-notification webhooks (need a public HTTPS endpoint; not viable for a desktop app) |
| Secure local credential storage (Secret Service over D-Bus, portal fallback) | Composer, reading pane, calendar grid UI (see their own spec sections) |

### 1.2 Protocol selection per account type

| Account type (detected) | Mail | Calendar | Contacts | Tasks | Auth |
|---|---|---|---|---|---|
| Microsoft work/school (Entra ID tenant) | Graph | Graph | Graph | Graph To Do | Microsoft identity platform OAuth (PKCE loopback; device code fallback) |
| Personal Microsoft account (outlook.com, hotmail.com, live.com, msn.com, custom domains on MSA) | Graph | Graph | Graph | Graph To Do | Same as above, `/common` or `/consumers` authority |
| Microsoft account, Graph unavailable (e.g. `MailboxNotEnabledForRESTAPI`) | IMAP + SMTP (XOAUTH2) | none (show notice) | none | none | Microsoft OAuth with Outlook resource scopes |
| Gmail / Google Workspace | IMAP + SMTP (XOAUTH2) | CalDAV (OAuth only; no VTODO) | CardDAV (OAuth only; vCard 3.0) | none in v1 (Google CalDAV does not serve VTODO) | Google OAuth (PKCE loopback, §3.7) |
| Generic IMAP provider | IMAP + SMTP | CalDAV if discovered | CardDAV if discovered | CalDAV VTODO if offered | Password / app password over TLS (SASL PLAIN), or OAuth if the provider documents it |

Classic vs new Outlook note: new Outlook for Windows supports Microsoft accounts, work/school accounts,
and third-party accounts such as Gmail, Yahoo and iCloud, plus other IMAP accounts (Microsoft's
"Getting started with the new Outlook" page). It synchronises third-party accounts through Microsoft's
cloud (unverified in this pass; not stated on that page). Axentra Outlook deliberately does
**not** relay through any cloud service; every connection goes directly from the device to the
provider. Classic Outlook connects to Exchange with MAPI/HTTP and EWS and keeps OST/PST files; Axentra
has no PST/OST support.

### 1.3 Account type detection (step 2 "Looking up your account…")

The spec previously named the detection step but not how it works. Order (decision; each step has a
3 s timeout (estimate), and the first conclusive answer wins):

1. **Provider presets by domain** (§5.1): `gmail.com`/`googlemail.com` → Google; `outlook.com`,
   `hotmail.*`, `live.*`, `msn.com` → personal Microsoft account.
2. **Entra tenant probe**: `GET https://login.microsoftonline.com/{domain}/v2.0/.well-known/openid-configuration`.
   A 200 response means the domain belongs to an Entra tenant: Microsoft's OIDC reference says the
   `{tenant}` segment can be the tenant's domain name (its example is `contoso.onmicrosoft.com`) or its
   GUID; that every verified custom domain resolves the same way is common practice (unverified). The
   `issuer` value carries the tenant GUID. A 400 with AADSTS90002 (tenant not found) means "not a work/school domain".
   A hit means "try Microsoft first", not "has an Exchange Online mailbox" — Graph can still answer
   `MailboxNotEnabledForRESTAPI` (on-premises or unlicensed mailbox) and the app then offers IMAP.
3. **MX lookup** (estimate of usefulness): MX under `mail.protection.outlook.com` → Microsoft 365;
   MX under `google.com` / `googlemail.com` → Google Workspace.
4. **IMAP/SMTP autodiscovery** (§5.1) → manual form.

A user can always override with "Work or school account" / "Other account" links on step 1 (decision).

---

## 2. Account setup UI (connectivity-owned surfaces)

Entry points (mirroring new Outlook): **Add account** at the bottom of the folder pane, and
**Settings › Accounts › Email accounts › Add account**. Both open the same modal dialog. Microsoft's
"Getting started with the new Outlook" page confirms "Add account" at the bottom of the folder list and
"Add account" under Settings › Accounts; the intermediate "Email accounts" page name is (unverified).

### 2.1 Dialog anatomy

| Part | Spec |
|---|---|
| Surface | Modal dialog, width 480 px (estimate; Fluent UI v9 `DialogSurface` caps width at 600 px), padding 24 px, corner radius 8 px (`borderRadiusXLarge`), 1 px transparent border, background `colorNeutralBackground1`, shadow `shadow64` — the last four are Fluent UI React v9 `DialogSurface` defaults |
| Title | Fluent "Subtitle 1" style: 20/28 px semibold (`fontSizeBase500`/`lineHeightBase500`). Text per step (below) |
| Body text | Fluent "Body 1": 14/20 px regular, `colorNeutralForeground2` |
| Inputs | Fluent v9 medium `Input`: min height 32 px, full width, 1 px `colorNeutralStroke1` border, 4 px radius (`borderRadiusMedium`) |
| Buttons | Fluent v9 medium `Button`: height 32 px, min width 96 px. Primary (brand) on the right, secondary to its left; 8 px gap (`DIALOG_GAP` in `DialogActions`) |
| Close | "X" icon button, top-right, 32×32 px hit area (estimate); same as Cancel |

These metrics come from the public Fluent UI React v9 source (`react-dialog`, `react-button`,
`react-input`, `tokens`). That new Outlook uses the same values in its account dialog is (unverified).

### 2.2 Steps and exact labels

The strings below are **Axentra's own copy** (decision), written in Outlook's voice. They are not a
transcription of new Outlook's add-account wizard, whose exact wording was not verified; new Outlook
hands most of the flow to Microsoft's or Google's web sign-in pages anyway.

| Step | Title | Content | Primary / secondary buttons |
|---|---|---|---|
| 1. Enter address | Add an account | Field label "Email address", placeholder "someone@example.com" | **Continue** (disabled until the value parses as an address) / Cancel |
| 2. Detecting | Add an account | 16 px spinner + "Looking up your account…" | — / Cancel |
| 3a. Microsoft | Sign in with Microsoft | "We'll open your web browser so you can sign in securely." Link button "Use a code instead" | **Open browser** / Cancel |
| 3b. Waiting for browser | Finish signing in | "Complete sign-in in your browser, then come back here." Link "Copy sign-in link" | — / Cancel |
| 3c. Device code | Sign in on another device | "Go to {verification_uri} and enter this code:" then the `user_code` in 28 px monospace (estimate), "Code expires in mm:ss" countdown | **Copy code** / Cancel; link "Open {verification_uri}" |
| 3d. Google | Sign in with Google | Same pattern as 3a/3b ("Open browser") | **Open browser** / Cancel |
| 3e. Manual IMAP | Set up your account | Fields: Display name; Email address; Password ("Use an app password if your provider requires one"); Incoming mail (IMAP) server; Port; Security [SSL/TLS ▾ / STARTTLS]; Outgoing mail (SMTP) server; Port; Security; checkbox "Outgoing server uses the same sign-in"; expander "Advanced" → IMAP username, SMTP username | **Sign in** / Back |
| 4. Verifying | Add an account | Spinner + "Connecting to {host}…" | — / Cancel |
| 5. Done | Account added | "{email} is ready. We're syncing your mail now." | **Done** |

Defaults for manual form: IMAP port 993 + SSL/TLS; SMTP port 465 + SSL/TLS, or 587 + STARTTLS when the
user picks STARTTLS (ports auto-update when security changes, unless the user typed a custom port).
Microsoft 365 and Outlook.com never reach this form (presets, §5.1); note that Microsoft documents
`smtp.office365.com` on 587 (or 25) with STARTTLS only and says a client that defaults to 465 will not
work, so never "upgrade" a Microsoft host to 465.

### 2.3 States and keyboard

- Focus is trapped in the dialog. Initial focus: the email field (step 1), the primary button (3a, 3c, 5).
- **Enter** activates the primary button when enabled. **Esc** = Cancel; cancelling during 3b/3c stops
  the loopback listener / device-code polling immediately and discards the PKCE verifier.
- **Ctrl+C** while the code (3c) has focus copies the `user_code`.
- Tab order follows visual order; link buttons are tab stops.
- Field validation errors appear under the field in Fluent "Caption 1" (12/16 px) using
  `colorPaletteRedForeground1`, and the input border switches to `colorPaletteRedBorder2`. This is what
  Fluent UI v9 `Field`/`Input` do for `validationState="error"`. (Correction: an earlier draft used
  `colorStatusDangerBorder1`, which is a pale tint — #EEACB2 in light — and too faint for an input border.)

### 2.4 Error and edge cases in setup

| Situation | Message (exact) | Action |
|---|---|---|
| Account already present | "This account is already in Axentra Outlook." | Button "OK"; focus existing account in folder pane |
| Signed in as a different identity than typed | "You signed in as {actual}. Add {actual} instead?" | "Add {actual}" / "Try again" |
| Browser flow not completed in 10 min (estimate) | "Sign-in timed out. Please try again." | Back to step 3a |
| Admin consent required (`consent_required`; AADSTS65001 = no consent recorded, AADSTS90094 = admin consent required, AADSTS90095 = admin-consent-workflow "request access" interrupt) | "Your organization needs to approve Axentra Outlook before you can sign in." | "Copy approval link" (admin-consent URL) / Close |
| User declined consent (`access_denied`, AADSTS65004) | "Sign-in was cancelled." | Back to 3a |
| Blocked by Conditional Access (AADSTS53003) | "Your organization's sign-in policy blocked this app." | Close; log `trace_id` |
| Device code blocked by policy (Conditional Access authentication-flow policy; exact AADSTS code (unverified), expect 53003) | "Your organization doesn't allow sign-in with a code. Use your browser instead." | Back to 3a |
| Work account without an Exchange Online mailbox (Graph `MailboxNotEnabledForRESTAPI`, or AADSTS500011 "resource principal not found" when requesting the `outlook.office.com` resource in a tenant without Exchange Online (unverified mapping)) | "We couldn't find a mailbox for {email} in Microsoft 365." | Offer IMAP setup (3e) / Close |
| MSA without an Outlook.com mailbox | "This Microsoft account doesn't have an Outlook.com mailbox." | Close |
| IMAP auth failure | "We couldn't sign in. Check your password. Some providers need an app password." | Stay on 3e |
| TLS certificate invalid | "The server's security certificate isn't trusted." + details expander (issuer, host, fingerprint SHA-256) | "Cancel" only by default; no "connect anyway" in v1 (decision) |
| No network | "You're offline. Connect to the internet and try again." | "Retry" |

### 2.5 Colour tokens used by these surfaces (Fluent 2, from microsoft/fluentui tokens)

All values below were re-derived on 2026-10-19 from `packages/tokens/src` (web light/dark themes with
the `brandWeb` ramp). Status tokens map danger → `cranberry`, warning → `orange`, success → `green`
(`statusColorMapping.ts`), which is why the danger reds are #B10E1C/#C50F1F rather than the `red`
palette's #BC2F32/#D13438. If Axentra adopts Outlook's own brand ramp instead of `brandWeb`, the
brand rows change.

| Token | Light | Dark | Used for |
|---|---|---|---|
| colorNeutralBackground1 | #FFFFFF | #292929 | Dialog surface, inputs |
| colorNeutralBackground1Hover | #F5F5F5 | #3D3D3D | Secondary button hover |
| colorNeutralBackground1Pressed | #E0E0E0 | #1F1F1F | Secondary button pressed |
| colorNeutralForeground1 | #242424 | #FFFFFF | Titles, input text |
| colorNeutralForeground2 | #424242 | #D6D6D6 | Body text |
| colorNeutralForeground3 | #616161 | #ADADAD | Helper text, countdown |
| colorNeutralStroke1 | #D1D1D1 | #666666 | Input and secondary button borders |
| colorBrandBackground | #0F6CBD | #115EA3 | Primary button rest |
| colorBrandBackgroundHover | #115EA3 | #0F6CBD | Primary button hover |
| colorBrandBackgroundPressed | #0C3B5E | #0C3B5E | Primary button pressed |
| colorNeutralForegroundOnBrand | #FFFFFF | #FFFFFF | Primary button text |
| colorBrandForeground1 | #0F6CBD | #479EF5 | Link buttons, spinner |
| colorNeutralForegroundDisabled | #BDBDBD | #5C5C5C | Disabled button text |
| colorNeutralBackgroundDisabled | #F0F0F0 | #141414 | Disabled button fill |
| colorStrokeFocus2 | #000000 | #FFFFFF | 2 px focus ring (outer) |
| colorStatusDangerForeground1 | #B10E1C | #DC626D | Error text / icon |
| colorStatusDangerBackground1 | #FDF3F4 | #3B0509 | Error message bar |
| colorStatusDangerBorder1 | #EEACB2 | #C50F1F | Error message bar border |
| colorPaletteRedForeground1 | #BC2F32 | #E37D80 | Field validation text (Fluent `Field`) |
| colorPaletteRedBorder2 | #D13438 | #E37D80 | Invalid input border (Fluent `Input`) |
| colorStatusWarningForeground1 | #BC4B09 | #FAA06B | "Needs attention" icon |
| colorStatusWarningBackground1 | #FFF9F5 | #4A1E04 | Warning message bar |
| colorStatusWarningBorder1 | #FDCFB4 | #F7630C | Warning bar border |
| colorStatusSuccessForeground1 | #0E700E | #54B054 | "Account added" check |
| colorStatusSuccessBackground1 | #F1FAF1 | #052505 | Success bar |

### 2.6 Account connection state in the main window

Each account node in the folder pane shows a trailing 16 px status glyph (estimate); a message bar
appears at the top of the message list when the selected account is unhealthy.

| State | Glyph | Message bar text | Bar intent | Action button |
|---|---|---|---|---|
| `ok` | none | none | — | — |
| `syncing` (first sync only) | spinner, `colorBrandForeground1` | "Syncing your mailbox… {n} messages so far" | info (neutral) | — |
| `offline` | cloud-off, `colorNeutralForeground3` | "You're offline. Changes will sync when you reconnect." | info | — |
| `throttled` | none (silent unless > 5 min (estimate)) | "{provider} is limiting requests. Syncing will resume shortly." | warning | — |
| `needs_reauth` | warning triangle, `colorStatusWarningForeground1` | "Sign in again to keep {email} up to date." | warning | **Sign in** |
| `keyring_locked` | lock, `colorStatusWarningForeground1` | "Your keyring is locked. Unlock it to sync {email}." | warning | **Unlock** |
| `error` | error circle, `colorStatusDangerForeground1` | "We can't connect to {email}. {short reason}" | error | **Retry** / **Account settings** |

Message bar: height 36 px single line (estimate), 12 px horizontal padding (estimate), 1 px border in the
intent's Border1 token, background in Background1, icon in Foreground1. Dismiss "X" hides it until the
state changes.

---

## 3. Microsoft identity platform (OAuth 2.0)

### 3.1 App registration (one-time, by Axentra maintainers)

| Setting | Value | Why |
|---|---|---|
| Supported account types | Accounts in any organizational directory and personal Microsoft accounts (`signInAudience` = `AzureADandPersonalMicrosoftAccount`) | One client for M365 and Outlook.com |
| Platform | **Mobile and desktop applications** | Public client; no secret |
| Redirect URI | `http://localhost` (portal) and, via the manifest `replyUrlsWithType`, `http://127.0.0.1` | Port is ignored for loopback matching; Microsoft recommends the `127.0.0.1` literal, but the portal text box rejects http loopback IPs, so add it in the manifest |
| Do not register | Multiple loopback URIs that differ only by port; `[::1]` (IPv6 loopback not supported); any URI with a query string (not allowed when personal accounts are enabled) | Per redirect-URI restrictions |
| Allow public client flows | **Yes** | Required for the device code flow |
| Client secret / certificate | None | Public clients must not use secrets |
| API permissions (delegated, Microsoft Graph) | `User.Read`, `Mail.ReadWrite`, `Mail.Send`, `Calendars.ReadWrite`, `Contacts.ReadWrite`, `Tasks.ReadWrite`, `offline_access`, `openid`, `profile` | Core feature set |
| Optional delegated (Graph) | `MailboxSettings.ReadWrite` (time zone, automatic replies, master categories; personal accounts supported), `People.Read` (recipient suggestions; personal accounts supported), `Mail.ReadWrite.Shared` / `Mail.Send.Shared` / `Calendars.ReadWrite.Shared` (shared and delegated mailboxes/calendars) | Request incrementally when the feature is first used. The permissions reference states `Mail.ReadWrite.Shared` and `Mail.Send.Shared` are valid **only for work or school accounts**, and `Calendars.ReadWrite.Shared` carries no personal-account badge: never request these three for an MSA |
| Optional delegated (IMAP/SMTP) | `IMAP.AccessAsUser.All`, `SMTP.Send` (both available for personal accounts per the reference) | Only for the IMAP fallback path. Correction: in the portal these appear in the **Microsoft Graph** permission list (they are catalogued in the Graph permissions reference), not under "Office 365 Exchange Online"; the scope strings sent at runtime still use the `https://outlook.office.com/` prefix (§3.3) |
| Publisher verification | Complete it before public release (estimate of necessity: many tenants restrict user consent for unverified multitenant apps) | Avoid "unverified" consent screens |

### 3.2 Authority (tenant) selection

Authority base: `https://login.microsoftonline.com/{tenant}/oauth2/v2.0/`.

| `{tenant}` | When |
|---|---|
| `common` | Default for first sign-in when we don't know the account type |
| `organizations` | User chose "Work or school account" explicitly |
| `consumers` | Detected consumer domain (outlook.com, hotmail.*, live.*, msn.com) |
| `{tid}` (GUID) | All refreshes after first sign-in for work/school accounts — store `tid` from the ID token; required for guest scenarios |

### 3.3 Scope strings

Graph scopes may be written short (`Mail.ReadWrite`) or fully qualified
(`https://graph.microsoft.com/Mail.ReadWrite`). Outlook-resource scopes must always be fully qualified.

| Purpose | Scope string sent |
|---|---|
| Sign-in + Graph (default) | `openid profile offline_access User.Read Mail.ReadWrite Mail.Send Calendars.ReadWrite Contacts.ReadWrite Tasks.ReadWrite` |
| IMAP fallback | `offline_access https://outlook.office.com/IMAP.AccessAsUser.All` |
| SMTP fallback | `offline_access https://outlook.office.com/SMTP.Send` |

Rules:

- One token request may carry scopes for **one resource only** (plus OIDC scopes). The `/authorize`
  call may request consent across resources; tokens are then minted per resource.
- A refresh token is good for every scope the user consented to, so the same refresh token can mint a
  Graph token and an Outlook (IMAP/SMTP) token with two separate `/token` calls.
- Never send `Mail.ReadWrite` and `https://outlook.office.com/IMAP.AccessAsUser.All` in one `/token`
  request. On code redemption Microsoft requires scopes from a single resource; on a refresh request
  whose scopes span several resources it silently returns a token for the resource of the **first**
  scope, so a mixed list yields a wrong-audience token rather than an error.

### 3.4 Interactive flow: authorization code + PKCE with loopback redirect (default)

1. Generate `code_verifier`: 32 random bytes → base64url without padding (43 chars).
   `code_challenge` = base64url(SHA-256(verifier)); `code_challenge_method=S256`.
2. Generate `state`: 32 random bytes base64url; keep it in memory with the verifier, keyed to this
   attempt. Never put user data or URLs in `state`.
3. Bind an HTTP listener on `127.0.0.1:0` (OS-assigned port). Redirect URI sent =
   `http://127.0.0.1:{port}` — no path and no trailing slash, so it matches the registered
   `http://127.0.0.1` except for the port, which Microsoft ignores for loopback. Microsoft documents
   that a redirect URI without a path is returned with a trailing `/` when `response_mode` is `query`
   or `fragment`, so the browser arrives at `http://127.0.0.1:{port}/?code=…`; serve path `/`.
   Correction: an earlier draft sent `http://127.0.0.1:{port}/`; whether the trailing-slash form also
   matches the slash-less registration is (unverified) — confirm in the smoke test before changing.
4. Open the system browser (`org.freedesktop.portal.OpenURI` inside Flatpak, else `xdg-open`) to:

   `GET https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize?client_id={id}&response_type=code&redirect_uri={uri}&response_mode=query&scope={scopes}&state={state}&code_challenge={challenge}&code_challenge_method=S256&login_hint={typed email}`

   Correction: the earlier draft sent `prompt=select_account` together with `login_hint`; Microsoft's
   OIDC reference says the two can't be combined. Use `login_hint` (no `prompt`) for first sign-in and
   re-auth; use `prompt=select_account` **without** `login_hint` only for "Try again" after an identity
   mismatch (§2.4). Optional: `domain_hint` (from a stored `tid`) on re-auth; `prompt=consent` only
   when adding scopes.
5. Listener accepts exactly one request whose `state` matches. Accept `GET` (query) and also `POST`
   `application/x-www-form-urlencoded` (in case `form_post` is ever used). Ignore favicon and other
   paths. Respond with a minimal HTML page: "You're signed in. You can close this tab and return to
   Axentra Outlook." (or the error text). Then close the listener.
6. On `error` in the callback (`access_denied`, `consent_required`, `interaction_required`, …) show the
   mapped message (§2.4) and stop.
7. Redeem the code **immediately** (codes live about one minute):

   `POST https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token` (form-encoded)
   `client_id`, `grant_type=authorization_code`, `code`, `redirect_uri` (byte-identical to step 4),
   `code_verifier`, `scope` (Graph scopes only).
8. Response fields: `access_token`, `token_type` (`Bearer`), `expires_in` (seconds, typically ~3600),
   `scope`, `refresh_token` (present because `offline_access` was requested), `id_token`.
9. Read identity claims from the `id_token` payload only for display and keying: `oid`, `tid`,
   `preferred_username`, `name`. Do not validate or parse **access** tokens — Microsoft tokens for
   Graph may not be JWTs and can be encrypted for consumer accounts.
10. Call `GET /me?$select=id,displayName,mail,userPrincipalName` and compare with the typed address
    (§2.4 "different identity").

Listener timeout: 10 minutes (estimate). The user may press "Copy sign-in link" to paste the authorize
URL into another browser on the same machine.

### 3.5 Fallback flow: device authorization grant (device code)

Use when: no browser can reach the loopback listener (remote/SSH sessions, locked-down sandboxes), or
the user picks "Use a code instead".

1. `POST https://login.microsoftonline.com/{tenant}/oauth2/v2.0/devicecode` with `client_id`, `scope`.
   Optional `?mkt=xx-XX` query parameter localises `message`.
2. Response: `device_code`, `user_code`, `verification_uri`, `expires_in` (default 900 s), `interval`
   (seconds), `message`. `verification_uri_complete` is **not** returned by Microsoft — do not rely on it.
3. Show step 3c. Poll `POST …/token` with
   `grant_type=urn:ietf:params:oauth:grant-type:device_code`, `client_id`, `device_code`, waiting at
   least `interval` seconds between polls.
4. Poll results:

| `error` | Action |
|---|---|
| `authorization_pending` | Keep polling after `interval` |
| `slow_down` (RFC 8628) | Add 5 s to the interval, keep polling |
| `authorization_declined` | Stop; "Sign-in was cancelled." |
| `bad_verification_code` | Stop; internal error, log |
| `expired_token` | Stop; "The code expired. Get a new code." button restarts at step 1 |
| success | Same token handling as §3.4 step 8 |

Personal-account users are asked to sign in again on the second device — expected, not an error.
`slow_down` is not in Microsoft's documented error list (only `authorization_pending`,
`authorization_declined`, `bad_verification_code`, `expired_token`); handle it anyway per RFC 8628.
The device code flow needs no redirect URI. For personal accounts the authority must be `common` or
`consumers`.

### 3.6 Token lifecycle and refresh

- Keep the access token **in memory only** (optionally cache it in the Secret Service item with its
  expiry to avoid a refresh on every launch).
- Store the refresh token in the Secret Service (§7). Whenever a `/token` response includes a new
  `refresh_token`, replace the stored one atomically; discard the old one.
- Refresh proactively when less than 5 minutes (estimate) remain, and reactively once on HTTP 401.
  Single-flight per account and resource: one refresh in progress, other callers await it.
- Refresh request: `POST …/token` with `client_id`, `grant_type=refresh_token`, `refresh_token`,
  `scope` (subset of consented scopes for one resource).
- Refresh tokens for native apps have no fixed lifetime but can expire or be revoked (password change,
  admin revocation, inactivity). Always handle failure.

| Token endpoint `error` | Meaning | App state / UI |
|---|---|---|
| `invalid_grant` | Refresh token expired/revoked, or code/verifier invalid | `needs_reauth` → "Sign in" bar |
| `interaction_required` | MFA, Conditional Access, or terms prompt required | `needs_reauth`; interactive flow with `login_hint` |
| `consent_required` / `invalid_scope` | New scope not consented | Interactive flow with the missing scopes |
| `temporarily_unavailable`, `server_error` | Transient | Backoff and retry (§4.3) |
| `invalid_client`, `unauthorized_client` | Registration problem | `error`; log, don't loop |

Classify on the `error` string. Use `error_codes` (AADSTS numbers) only to pick friendlier text, and
log `trace_id` and `correlation_id` for support. AADSTS codes worth recognising (descriptions from
Microsoft's error-code reference):

| AADSTS | Meaning | Text / state |
|---|---|---|
| 70008, 700082 | Refresh token expired through inactivity | `needs_reauth`, no alarm wording |
| 50173 | Grant revoked (e.g. password changed or reset) | `needs_reauth` |
| 70043 | Refresh token rejected by Conditional Access sign-in frequency | `needs_reauth` |
| 50076, 50079 | MFA required / MFA registration required | interactive flow |
| 53003 | Blocked by Conditional Access | `error`; "Your organization's sign-in policy blocked this app." |
| 65001, 90094 | Consent missing / admin consent required | §2.4 admin-consent row |
| 50011 | Redirect URI mismatch (registration bug) | `error`; log, don't loop |
| 90002 | Tenant not found (used by detection, §1.3) | — |

Continuous access evaluation: if Graph returns 401 with a `WWW-Authenticate` header carrying
`error="insufficient_claims"` and a `claims` value, run the interactive flow passing that `claims`
parameter. v1 does not advertise the `cp1` client capability (decision), so this should be rare.

### 3.7 Google OAuth for desktop apps (gap filled)

The earlier draft only said "Google OAuth (PKCE loopback)". Facts from Google's native-app and OAuth 2.0
overview pages:

| Item | Value |
|---|---|
| Client type | "Desktop app" OAuth client in Google Cloud console |
| Authorization endpoint | `https://accounts.google.com/o/oauth2/v2/auth` |
| Token endpoint | `https://oauth2.googleapis.com/token` |
| Revocation endpoint | `https://oauth2.googleapis.com/revoke` (call on account removal) |
| Redirect | Loopback IP: `http://127.0.0.1:{port}` or `http://[::1]:{port}` (Google, unlike Microsoft, accepts IPv6 loopback). Custom URI schemes and the copy/paste (OOB) flow are no longer supported |
| PKCE | Always; `S256` |
| Client secret | Google issues one to desktop clients but documents desktop apps as public clients that cannot keep it secret; send it on the token request if issued and treat it as non-confidential. Google's page describes it as optional (verify against the live endpoint) |
| Scopes | `openid email https://mail.google.com/` (IMAP/SMTP); add `https://www.googleapis.com/auth/calendar` and the CardDAV scope only when §6 is enabled |
| Refresh tokens | Returned for installed apps. They stop working when: the user revokes access; unused for 6 months; the user changes password and the token carries Gmail scopes; the per-account cap of **100 refresh tokens per OAuth client** is exceeded (oldest silently invalidated); time-based access expires; an admin restricts a scope (`admin_policy_enforced`) |
| Testing mode | While the consent screen is "External" + "Testing", refresh tokens expire after **7 days** — dev builds will need re-auth weekly |
| Publishing | `https://mail.google.com/` is a restricted scope: public release needs Google's OAuth verification including the restricted-scope security assessment (business/legal task; requirements (verify) at release time) |

---

## 4. Microsoft Graph REST

### 4.1 Request conventions

| Header | Value | Notes |
|---|---|---|
| `Authorization` | `Bearer {access_token}` | Never log |
| `Prefer` | `IdType="ImmutableId"` | On **every** request (it applies per request) |
| `Prefer` | `outlook.timezone="UTC"` | Events/tasks returned in UTC; convert locally with IANA zones |
| `Prefer` | `outlook.body-content-type="html"` or `"text"` | Body format for message `body`/`uniqueBody` |
| `Prefer` | `odata.maxpagesize={n}` | Delta and list page size |
| `client-request-id` | new UUID per request | Log with the response `request-id` |
| `Accept-Encoding` | `gzip` | |

Multiple preferences may be sent as repeated `Prefer` headers or comma-separated in one (RFC 7240).
No Microsoft page shows either form with Outlook preferences combined, so which form Graph honours is
(unverified). Smoke test: send both forms and read the `Preference-Applied` response header, which
Graph echoes for `outlook.timezone` and `outlook.body-content-type`; keep the form for which every
preference is echoed. `IdType="ImmutableId"` is not echoed, so verify it by checking that a moved
message keeps its `id`.

Error envelope: `{"error":{"code":"…","message":"…","innerError":{"date":"…","request-id":"…","client-request-id":"…"}}}`.

| HTTP | Typical `error.code` | Handling |
|---|---|---|
| 400 | `ErrorInvalidIdMalformed`, `InefficientFilter`, `ErrorMimeContentInvalidBase64String` (sendMail MIME) | Bug or bad query; don't retry |
| 401 | `InvalidAuthenticationToken` | Refresh once, retry once; then `needs_reauth` |
| 403 | `ErrorAccessDenied`, `ErrorQuotaExceeded` (delta writes sync state; full mailbox) | Show error; quota → "Your mailbox is full" |
| 404 | `ErrorItemNotFound` | Treat as deleted remotely; drop pending ops on it |
| 409 / 412 | conflict / precondition failed | Re-fetch item, re-apply local change if still valid |
| 410 | (delta) | Documented "synchronization reset": the `Location` header holds the initial delta URL with an empty `$deltatoken`; run a full resync of that collection from it |
| 4xx | `syncStateNotFound` / sync state invalid | Delta token expired → full resync of that collection. For Outlook entities (message, mailFolder, event, contact, contactFolder, todoTask, todoTaskList) token lifetime is **not fixed**: tokens are evicted from a server cache, so even a deltaLink a few days old can fail |
| 424 | (batch sub-response) | A `dependsOn` predecessor failed; re-queue the chain |
| 429 | `TooManyRequests` | Wait `Retry-After` seconds, retry (§4.3) |
| 503 / 504 | `ServiceUnavailable` / gateway timeout | Honour `Retry-After` if present; on 504 halve page size |
| any | `MailboxNotEnabledForRESTAPI` | Mailbox not reachable via Graph → offer IMAP fallback (§1.2) |

### 4.2 Paging

- Follow `@odata.nextLink` until absent. Treat it as an opaque absolute URL; never edit it or extract
  `$skip`/`$skiptoken` (message lists use `$skip` internally and it can exceed the page size).
- Delta rounds: follow `@odata.nextLink` until a page carries `@odata.deltaLink`; persist the
  `deltaLink` URL as the folder/calendar/list cursor. A page never has both.
- Default message page size is 10; `$top` accepts 1–1000 for messages and calendarView. Use 50 for list
  pages and `odata.maxpagesize=100` for delta (estimate); halve on 504.

### 4.3 Throttling, concurrency and retries

Documented Outlook-service limits (per app ID **per mailbox**, v1.0 and beta):

| Limit | Value |
|---|---|
| API requests | 10,000 per 10-minute window |
| Concurrent requests | 4 |
| Upload volume (PATCH/POST/PUT) | 150 MB per 5-minute window |
| JSON batch | Graph forwards at most 4 unordered requests from a batch to Outlook at a time; `dependsOn` makes them sequential |

Implementation:

- Per-account semaphore of **3** concurrent Graph calls (estimate; one below the documented 4 to leave
  room for the user's other clients and batch fan-out).
- On 429: sleep exactly `Retry-After` seconds (header may be seconds or an HTTP-date), then retry; repeat
  while still 429. All requests count during throttling, so never retry immediately.
- No `Retry-After`: exponential backoff 2 s, 4 s, 8 s … cap 5 min, ±20% jitter (estimate).
- Batches (`POST /$batch`, at most 20 sub-requests — documented): the batch can return 200 while
  individual responses are 429 (each sub-request is evaluated against throttling separately); retry
  only the failed sub-requests after the largest `retry-after` among them. Sub-request `url`s are
  relative (`/me/messages/…`), and headers such as `Prefer` must be repeated inside each sub-request's
  `headers` object. Microsoft advises a batch be either fully sequential (`dependsOn` chain) or fully
  parallel; a failed predecessor makes dependants fail with 424.
- Track an upload byte budget per mailbox; queue attachment uploads when nearing 150 MB/5 min.

### 4.4 Immutable IDs

- Send `Prefer: IdType="ImmutableId"` on all requests, including delta (delta links work with either ID
  format). Messages, attachments, events, eventMessages, contacts keep one ID across folder moves in the
  same mailbox (outlookTask too).
- IDs still change when an item moves to an archive mailbox, or is exported and re-imported.
- Container IDs (mailFolder, calendar) are always stable; they don't use immutable format.
- IDs are **case-sensitive**: SQLite columns holding remote IDs must use `COLLATE BINARY` (the default) —
  never `NOCASE`.
- Migration of stored regular IDs: `POST /me/translateExchangeIds` with `inputIds` (≤ 1000),
  `sourceIdType: "restId"`, `targetIdType: "restImmutableEntryId"`.

### 4.5 Identity and settings

| Call | Use |
|---|---|
| `GET /me?$select=id,displayName,mail,userPrincipalName` | Account identity (`mail` may be null for some MSAs; fall back to `userPrincipalName`) |
| `GET /me/photo` | The signed-in user's own photo, metadata: `@odata.mediaEtag`, `width`, `height`. 404 (any error code) or the 1×1 placeholder (`"id":"1x1"`, empty etag) = no photo |
| `GET /me/photos/{n}x{n}/$value`, `GET /me/photo/$value` | Photo bytes for the account avatars (header, Account Manager, Settings › Accounts): the largest of 240 / 120 / 96 px that fits the photo, else (smaller photo, or 400 / 404 on the sized request: sizes are documented for Microsoft 365 only) the full photo. Kept only when the bytes are JPEG, PNG, GIF or WebP up to 1 MiB, as a `data:` URL in the store (`account_photos`, never backed up); a larger download stops at the limit (a declared larger `Content-Length` isn't read at all), and a photo that wasn't kept isn't downloaded again until its etag changes. Saved only for the account the check began with, never for a newer account that reused its id; removing an account stops its check. Delegated `User.Read`, already in the scopes; works for personal accounts |
| `GET /me/mailboxSettings` (optional scope) | `timeZone`, `language`, `workingHours`, `automaticRepliesSetting` |
| `GET /me/outlook/masterCategories` (optional scope) | Category names and preset colours |

### 4.6 Mail folders

| Operation | Request | Notes |
|---|---|---|
| Initial + incremental folder sync | `GET /me/mailFolders/delta?$select=displayName,parentFolderId,childFolderCount,unreadItemCount,totalItemCount,isHidden` | Persist `deltaLink`. Whether this returns nested folders is **not documented**: the reference page is silent and community reports disagree (one reports the whole tree flattened, another root-level only). Completeness check after every full round: for each folder with `childFolderCount > 0`, count delta rows whose `parentFolderId` equals its id; on a shortfall, track that parent with `GET /me/mailFolders/{id}/childFolders/delta` (exists in the v1.0 SDK/PowerShell surface as the childFolders delta; no REST reference page found (unverified)) or, failing that, list `childFolders` recursively. Do not `$select` `wellKnownName` — it is beta-only and v1.0 returns 400. May return 403 `ErrorQuotaExceeded` when the mailbox is full (documented) |
| Resolve roles | `GET /me/mailFolders/{wellKnownName}?$select=id` for `inbox`, `sentitems`, `drafts`, `deleteditems`, `junkemail`, `archive`, `outbox`, `msgfolderroot`, `conversationhistory`, `scheduled`, `recoverableitemsdeletions` | Well-known names work regardless of mailbox language. Other documented names: `clutter`, `conflicts`, `localfailures`, `serverfailures`, `syncissues`, `searchfolders`. `archive` is the one-click Archive folder, not the Exchange archive mailbox. `recoverableitemsdeletions` holds soft-deleted items and backs "Recover deleted items" |
| Hidden folders | add `includeHiddenFolders=true` to list calls | Show only if user enables "Show hidden folders" (decision) |
| Create | `POST /me/mailFolders/{parentId}/childFolders` `{"displayName":"…"}` | |
| Rename | `PATCH /me/mailFolders/{id}` `{"displayName":"…"}` | |
| Move | `POST /me/mailFolders/{id}/move` `{"destinationId":"…"}` | |
| Delete (not in Deleted Items) | `POST /me/mailFolders/{id}/move` `{"destinationId":"deleteditems"}` | Mirrors Outlook, which puts a deleted folder under Deleted Items. Correction: the earlier draft used `DELETE` and assumed it lands in Deleted Items; the API page does not say so, and by analogy with message DELETE it most likely soft-deletes past Deleted Items (unverified) |
| Delete from Deleted Items | `DELETE /me/mailFolders/{id}` (204) | Confirm in UI; recoverability (unverified) |
| Permanently delete | `POST /me/mailFolders/{id}/permanentDelete` | Documented action; purges the folder and its items. Behind an explicit confirm only |
| Empty folder | No single call; page `messages` and delete/move in `$batch` chunks | Show progress |

Folder role → UI name (Axentra list order in folder pane): Inbox, Drafts, Sent Items, Deleted Items,
Junk Email, Archive, Notes (hidden in v1), Conversation History, then user folders A→Z (estimate based
on new Outlook default order).

### 4.7 Messages

Standard list `$select` (keep in one constant):
`id,conversationId,internetMessageId,subject,bodyPreview,from,sender,toRecipients,ccRecipients,bccRecipients,replyTo,receivedDateTime,sentDateTime,isRead,isDraft,flag,importance,hasAttachments,categories,inferenceClassification,parentFolderId,changeKey,lastModifiedDateTime,webLink`

| Operation | Request | Response / notes |
|---|---|---|
| List page (backfill) | `GET /me/mailFolders/{id}/messages?$select=…&$orderby=receivedDateTime desc&$filter=receivedDateTime lt {cursor}&$top=50` | With `$filter`+`$orderby`, the ordered property must appear first in `$filter`, else `InefficientFilter` |
| Delta (recent window) | `GET /me/mailFolders/{id}/messages/delta?$select=…&$filter=receivedDateTime ge {windowStart}` + `Prefer: odata.maxpagesize=100` | Only `receivedDateTime ge/gt` filters allowed; filtered delta returns at most 5,000 messages; only `$orderby=receivedDateTime desc` allowed (without it order is not guaranteed); `$select`, `$top`, `$expand` supported; no `$search`. Documented caveat: a filtered delta still emits read/unread changes and `@removed` rows for messages **outside** the filter (tracking is per collection) — upsert/remove by id without assuming the row is inside the window. Optional custom query option `changeType=created|updated|deleted` exists (not used in v1) |
| Delta (whole folder) | same without `$filter` | Use when `totalItemCount` ≤ 5,000 (estimate) so every change is tracked |
| Removed in delta | item `{ "id": "…", "@removed": { "reason": "deleted" } }` | Means "gone from this folder" (moved or deleted). With immutable IDs a moved item reappears in the target folder's delta with the same `id` |
| Get body | `GET /me/messages/{id}?$select=body,uniqueBody,internetMessageHeaders` + `Prefer: outlook.body-content-type="html"` | `uniqueBody` is the non-quoted part — use for conversation view |
| Raw MIME | `GET /me/messages/{id}/$value` | For "View source", S/MIME, export .eml |
| Mark read/unread | `PATCH /me/messages/{id}` `{"isRead":true}` | Batch for multi-select |
| Flag | `PATCH` `{"flag":{"flagStatus":"flagged"}}`; with dates: `{"flag":{"flagStatus":"flagged","startDateTime":{…},"dueDateTime":{…}}}` | `dueDateTime` without `startDateTime` → 400 |
| Complete / clear flag | `{"flag":{"flagStatus":"complete"}}` / `{"flag":{"flagStatus":"notFlagged"}}` | Enum: `notFlagged`, `flagged`, `complete` |
| Categories | `PATCH` `{"categories":["Red category"]}` | Names must match master categories |
| Focused/Other | `PATCH` `{"inferenceClassification":"other"}` | Values `focused`, `other` |
| Move | `POST /me/messages/{id}/move` `{"destinationId":"{folderId or well-known name}"}` | 201 + moved message (same immutable id) |
| Copy | `POST /me/messages/{id}/copy` `{"destinationId":"…"}` | New id |
| Delete (Delete key) | `move` to `deleteditems` | Matches Outlook's Delete |
| Delete from Deleted Items / Shift+Del | `DELETE /me/messages/{id}` (204) | Soft delete: the item leaves every visible folder and lands in Recoverable Items › Deletions (well-known name `recoverableitemsdeletions`), which Microsoft describes as the home of items deleted from Deleted Items or with Shift+Delete and which users reach via "Recover deleted items". It does **not** go to Deleted Items. Microsoft's DELETE page is silent on the destination; the landing folder is stated by third-party Graph client docs (Rebex) (unverified first-party). Items under a compliance hold reportedly return 403 (third-party test, unverified) |
| Permanently delete | `POST /me/messages/{id}/permanentDelete` (204) | Documented: moves the item to the Purges folder, unreachable by any Outlook client, removed after the retention period unless a hold applies. Only behind an explicit "Permanently delete" confirm. Not available in US Gov or China clouds (per the API page) |
| New draft | `POST /me/messages` with message JSON | 201 draft in Drafts |
| Reply / Reply all / Forward drafts | `POST /me/messages/{id}/createReply` · `/createReplyAll` · `/createForward` (body optional: `{"comment":"…"}` or `{"message":{…}}`) | 201 draft including quoted original; composer then PATCHes |
| One-shot reply/forward | `POST /me/messages/{id}/reply` · `/replyAll` `{"comment":"…","message":{…}}`; `/forward` `{"comment":"…","toRecipients":[…]}` | 202; used for quick inline replies without attachments |
| Update draft | `PATCH /me/messages/{draftId}` (subject, body, toRecipients, ccRecipients, bccRecipients, importance, isReadReceiptRequested, isDeliveryReceiptRequested) | Autosave every 10 s while typing (estimate) |
| Send draft | `POST /me/messages/{draftId}/send` | 202; Sent Items copy appears asynchronously |
| Send without draft | `POST /me/sendMail` `{"message":{…},"saveToSentItems":true}` or base64 MIME body with `Content-Type: text/plain` | 202 only means accepted |
| Find sent copy | `GET /me/messages/{draftImmutableId}` after send (retry with backoff) | Works because the immutable ID survives the move to Sent Items |
| Search | `GET /me/messages?$search="from:ana subject:budget"` (or under a folder) | KQL; max 1,000 results; sorted by sent date; default fields from/subject/body |

Recipient object shape: `{"emailAddress":{"name":"…","address":"…"}}`. Body shape:
`{"contentType":"html"|"text","content":"…"}`. Recipient cap: 500 across To/Cc/Bcc (Exchange Online
default sending limit; the event and eventMessage pages cite 500 for attendees/recipients).

Meeting messages in the inbox are `eventMessage` items (`@odata.type` e.g.
`#microsoft.graph.eventMessageRequest`) with `meetingMessageType` — documented values: `none`,
`meetingRequest`, `meetingCancelled`, `meetingAccepted`, `meetingTenativelyAccepted` (this misspelling
is the published wire value — match it exactly; also accept `meetingTentativelyAccepted` defensively),
`meetingDeclined`. Get the linked event for RSVP buttons with
`GET /me/messages/{id}?$expand=microsoft.graph.eventMessage/event` (this exact cast path is the
documented example on the eventMessage get page). The `event` navigation assumes the calendar
attendant has already placed the event in the invitee's calendar; if it is null, retry later.
Event-message bodies come back as HTML only.

### 4.8 Attachments

| Operation | Request | Notes |
|---|---|---|
| List metadata | `GET /me/messages/{id}/attachments?$select=id,name,contentType,size,isInline,lastModifiedDateTime` | Never fetch `contentBytes` in lists |
| Download | `GET /me/messages/{id}/attachments/{attId}/$value` | Raw bytes; stream to disk |
| Types | `#microsoft.graph.fileAttachment` (`contentId`, `contentBytes`), `itemAttachment` (attached mail/event; `$expand=microsoft.graph.itemattachment/item`), `referenceAttachment` (cloud link) | |
| Inline images | body `<img src="cid:{contentId}">` ↔ fileAttachment with `isInline: true` | `hasAttachments` is false when only inline attachments exist |
| Add < 3 MB | `POST /me/messages/{id}/attachments` `{"@odata.type":"#microsoft.graph.fileAttachment","name":"…","contentType":"…","contentBytes":"<base64>","isInline":false}` | |
| Add 3–150 MB | `POST /me/messages/{id}/attachments/createUploadSession` `{"AttachmentItem":{"attachmentType":"file","name":"…","size":N}}` → `uploadUrl`, `expirationDateTime`, `nextExpectedRanges` | Then `PUT {uploadUrl}` chunks |
| Upload chunk | Headers `Content-Type: application/octet-stream`, `Content-Length`, `Content-Range: bytes {start}-{end}/{total}`; **no `Authorization` header** (URL is pre-authenticated and points at `outlook.office.com`; its embedded token expires at `expirationDateTime`) | Keep each chunk under 4 MB (Microsoft's performance guidance, not a hard limit); upload in order; intermediate 200s carry `nextExpectedRanges`; final 201 has a `Location` header ending in `Attachments('{id}')` |
| Resume | read `nextExpectedRanges` from the last 200 response | |
| Cancel | `DELETE {uploadUrl}` (no auth header) | 204 |
| Events | Same endpoints under `/me/events/{id}/attachments` | Not for group calendars |

`createUploadSession` for files under 3 MB fails with `ErrorAttachmentSizeShouldNotBeLessThanMinimumSize`.
Microsoft lists a known issue for large attachments on items in shared or delegated mailboxes; test
that path separately before enabling it.

### 4.9 Calendars and events

| Operation | Request | Notes |
|---|---|---|
| List calendars | `GET /me/calendars?$select=id,name,color,hexColor,isDefaultCalendar,canEdit,canShare,owner,changeKey` | Re-list every 30 min (estimate); no delta for calendars |
| Window read | `GET /me/calendars/{id}/calendarView?startDateTime={ISO}&endDateTime={ISO}&$top=200` | Expands occurrences and exceptions; offsets in the query are honoured (and are not affected by `Prefer: outlook.timezone`), no offset = UTC; `$top` 1–1000; `createdDateTime`/`lastModifiedDateTime` can't be `$select`ed (omit `$select` to get them) |
| Window delta | Default calendar: `GET /me/calendarView/delta?startDateTime=…&endDateTime=…`. Other calendars: `GET /me/calendars/{id}/calendarView/delta?…` | `+ Prefer: odata.maxpagesize` (and `outlook.timezone`, UTC if absent). `$select`, `$expand`, `$filter`, `$orderby`, `$search` **not** supported; the range is baked into the token. Correction/caveat: the v1.0 `event: delta` reference lists only the default-calendar path ("the user's primary calendar"), while the delta-for-events guide says each calendar can be tracked individually; the per-calendar path is therefore (unverified) for v1.0 — smoke-test it, and fall back to periodic `calendarView` re-reads with `changeKey` comparison for secondary calendars. Documented: `@removed` rows (reason `deleted`) can include events **outside** the window that changed; filter them by range instead of treating them as deletions inside the window |
| Create | `POST /me/calendars/{id}/events` | Include `transactionId` (UUID) so retries don't duplicate. Attendees get invitations automatically |
| Update | `PATCH /me/events/{id}` | Organizer edits send updates to attendees |
| Delete | `DELETE /me/events/{id}` | If organizer of a meeting, attendees get a cancellation |
| Cancel with note | `POST /me/events/{id}/cancel` `{"comment":"…"}` | Organizer only |
| RSVP | `POST /me/events/{id}/accept` · `/tentativelyAccept` · `/decline` with `{"comment":"…","sendResponse":true}` | 202. `proposedNewTime` (`{start,end}` dateTimeTimeZone) allowed on tentative/decline only if `allowNewTimeProposals` (default `true` on the event) and `sendResponse` are true, else 400. User calendars only (group calendars auto-accept) |
| Series vs occurrence | Respond/patch on `seriesMasterId` for "All events in series", on occurrence `id` for "This event" | |
| Instances | `GET /me/events/{masterId}/instances?startDateTime=…&endDateTime=…` | |
| Forward invite | `POST /me/events/{id}/forward` `{"toRecipients":[…],"comment":"…"}` | |
| Reminders | `GET /me/reminderView(startDateTime='…',endDateTime='…')`; `POST /me/events/{id}/snoozeReminder` `{"newReminderTime":{…}}`; `/dismissReminder` | Drive the reminders window |
| Teams meeting | `isOnlineMeeting: true`, `onlineMeetingProvider: "teamsForBusiness"` on create | Work/school only (provider values: `unknown`, `teamsForBusiness`, `skypeForBusiness`, `skypeForConsumer`); once set, `isOnlineMeeting`/`onlineMeetingProvider` can't be changed and the meeting stays online. Join link is `onlineMeeting.joinUrl` (`onlineMeetingUrl` is slated for deprecation) |
| Delete permanently | `POST /me/events/{id}/permanentDelete` | Documented; purges. Behind explicit confirm |

Event write payload (minimum): `subject`, `body{contentType,content}`, `start{dateTime,timeZone}`,
`end{dateTime,timeZone}`, `isAllDay`, `location{displayName}`, `attendees[{emailAddress{address,name},type}]`
(`type`: `required` | `optional` | `resource`), `showAs`, `isReminderOn`, `reminderMinutesBeforeStart`,
`sensitivity`, `importance`, `categories`, `recurrence`.

Time zones: store IANA names locally; when writing, send Windows zone names (map via CLDR
`windowsZones.xml`) because Graph's `timeZone` documentation centres on Windows names (estimate of
safest choice). All-day events: start and end at local midnight in the same zone; end is exclusive.

Recurrence shape (`patternedRecurrence`): `pattern{type: daily|weekly|absoluteMonthly|relativeMonthly|absoluteYearly|relativeYearly, interval, daysOfWeek[], dayOfMonth, month, firstDayOfWeek, index: first|second|third|fourth|last}`
and `range{type: endDate|noEnd|numbered, startDate, endDate, numberOfOccurrences, recurrenceTimeZone}`.
Keep it as JSON; do not expand Graph recurrences locally — calendarView already returns instances.

Rolling window: `[today − 90 days, today + 365 days]` (estimate). Re-anchor weekly: start a fresh delta
round for the new window and drop events outside it (except those the user opened recently).

### 4.10 Contacts

| Operation | Request |
|---|---|
| Folders | `GET /me/contactFolders` (plus `childFolders`); folder changes via `GET /me/contactFolders/delta` (documented `contactFolder: delta`). Known issue (Microsoft staff answer on Microsoft Q&A): this list does **not** include the default Contacts folder |
| Default folder id | No documented well-known alias for the root Contacts folder. Use `parentFolderId` of `GET /me/contacts?$top=1&$select=parentFolderId` (the documented workaround; needs at least one contact). `POST /me/contacts` always creates in the root folder |
| Delta (non-default folders) | `GET /me/contactFolders/{id}/contacts/delta?$select=…` (+ `Prefer: odata.maxpagesize`) |
| Delta (default folder) | `GET /me/contactFolders/{rootId}/contacts/delta` once `rootId` is known. While the mailbox has no contacts (no id obtainable), use `GET /me/contacts/delta` — undocumented, reported working by a Microsoft-staff Q&A answer (2024), which also warns it is unsupported (unverified); switch to the folder-scoped form as soon as a contact exists |
| CRUD | `POST /me/contacts` · `PATCH /me/contacts/{id}` · `DELETE /me/contacts/{id}` · `POST /me/contacts/{id}/permanentDelete` (purge) |
| Photo | `GET /me/contacts/{id}/photo/$value` (404 = none); `PUT …/photo/$value` with image bytes |

When updating, always resend `displayName`; changing name parts can otherwise regenerate it.

### 4.11 Microsoft To Do

| Operation | Request |
|---|---|
| Lists | `GET /me/todo/lists`; delta `GET /me/todo/lists/delta` |
| Tasks | `GET /me/todo/lists/{listId}/tasks`; delta `GET /me/todo/lists/{listId}/tasks/delta` |
| Create | `POST /me/todo/lists/{listId}/tasks` `{"title":"…","dueDateTime":{…},"importance":"high","isReminderOn":true,"reminderDateTime":{…},"body":{"contentType":"text","content":"…"},"categories":[…]}` |
| Complete | `PATCH …/tasks/{id}` `{"status":"completed"}` |
| Steps | `GET/POST …/tasks/{id}/checklistItems` `{"displayName":"…","isChecked":false}` |
| Links | `…/tasks/{id}/linkedResources` (`webUrl`, `applicationName`, `displayName`, `externalId`) |
| Lists CRUD | `POST /me/todo/lists` `{"displayName":"…"}` · `PATCH` · `DELETE` |

`wellknownListName`: `defaultList` (built-in **Tasks**), `flaggedEmails` (**Flagged email**), `none`
(user list); ignore `unknownFutureValue`. Built-in lists can't be renamed or deleted (documented on the
todoTaskList page) — disable those commands. Tasks for flagged emails appear in `flaggedEmails`
(documented); that the flag → task link is created server-side for every mailbox regardless of the
user's To Do "Flagged email" setting is (unverified) — never create a duplicate task client-side. Treat
`dueDateTime` as date-only in the UI (estimate, matching To Do's date picker).

### 4.12 Graph sync scheduler

| Collection | Initial | Incremental cadence (focused / background) |
|---|---|---|
| Folders | delta, full | 5 min / 15 min (estimate) |
| Inbox messages | filtered delta, last 30 days (estimate), then backfill pages until 6 months (estimate) | 60 s / 5 min (estimate) |
| Other folders | delta on first open; favourites eagerly | 5 min / 30 min (estimate) |
| Calendar (each visible calendar) | calendarView delta on window | 2 min / 10 min (estimate) |
| Contacts | delta | 30 min (estimate) |
| To Do lists + tasks | delta | 2 min / 10 min (estimate) |
| Own profile photo | right after sign-in / re-sign-in | at most every 6 h, from Quick and Full passes; the bytes only when the etag changed. A failed check (network, 5xx, 429, 401 / 403) keeps the stored photo, never fails the pass or changes the account's state, and is retried after 30 min. One check per account at a time; a sign-in during a check checks again after it |

Order on first sign-in: identity → folders → Inbox first page (render ASAP) → calendar today ± 7 days →
remaining Inbox window → To Do → contacts → other folders. Budget check: these cadences use well under
1% of the 10,000/10 min allowance.

---

## 5. IMAP4rev1 / IMAP4rev2 and SMTP

### 5.1 Connection and TLS

| Item | Rule |
|---|---|
| IMAP | Prefer implicit TLS on 993. STARTTLS on 143 only when the user selects it; abort if `STARTTLS` isn't advertised or fails. Never authenticate in clear text |
| SMTP submission | 465 implicit TLS (preferred per RFC 8314) or 587 + STARTTLS. Port 25 is not offered |
| Certificates | Validate chain and hostname with the system trust store (rustls + native roots). No silent bypass |
| Timeouts | Connect 15 s, command 60 s, IDLE read timeout 31 min (estimate) |
| Connection pool | ≤ 4 IMAP connections per account (estimate): 1 IDLE on INBOX, 1 foreground (user actions), 1–2 background sync. Gmail's per-account simultaneous-connection cap (commonly cited as 15) was not found on a current Google page (unverified). Google does document daily IMAP bandwidth caps for Workspace: 2,500 MB download and 500 MB upload per day — pace the initial backfill so it cannot exhaust them |
| SMTP limits (Microsoft 365 SMTP AUTH) | Documented: 30 messages per minute and 10,000 recipients per day per mailbox; TLS 1.2 or 1.3 required. A concurrent-connection cap of 3 per mailbox (error `4.3.2 STOREDRV.ClientSubmit`) is reported by third parties (unverified) — use one SMTP connection per account |
| Client ID | Send `ID ("name" "Axentra Outlook" "version" "{semver}")` after auth when `ID` is advertised |

Provider presets (skip autodiscovery when the domain matches):

| Provider | IMAP | SMTP | Auth |
|---|---|---|---|
| Outlook.com (IMAP fallback) | `outlook.office365.com:993` TLS | `smtp-mail.outlook.com:587` STARTTLS | OAuth2 only (XOAUTH2); Microsoft's settings page says Outlook.com requires Modern Auth/OAuth2. IMAP must be switched on by the user: Outlook.com **Settings › Mail › Forwarding and IMAP**, toggle **Let devices and apps use IMAP** (off by default per the support page), then **Save**. Microsoft warns that non-Microsoft domains on Outlook.com may not sync over IMAP |
| Microsoft 365 (IMAP fallback) | `outlook.office365.com:993` TLS | `smtp.office365.com:587` STARTTLS (documented; port 25 also allowed, 465 not supported) | OAuth2 (XOAUTH2). SMTP AUTH is off by default for tenants created after January 2020, is off whenever Entra "security defaults" are on, and can be disabled org-wide or per mailbox — and that switch blocks OAuth SMTP AUTH too. Microsoft recommends tenants disable SMTP AUTH, so treat SMTP failure on M365 as likely and prefer Graph `sendMail` whenever a Graph token can be had |
| Gmail / Workspace | `imap.gmail.com:993` TLS | `smtp.gmail.com:465` TLS or `:587` STARTTLS | OAuth2, scope `https://mail.google.com/`. App passwords remain usable (Google's Workspace transition article names them as the exception) |

Autodiscovery for other domains, in order: provider presets → DNS SRV: `_imaps._tcp`, then `_imap._tcp` (STARTTLS
only, shown pre-selected but still user-confirmed), `_submission._tcp` (RFC 6186) and `_submissions._tcp` (added by RFC 8314 for implicit-TLS submission —
correction: it is not in RFC 6186) → autoconfig XML in the format Thunderbird uses, tried at
`https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={email}` and
`https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml` (both (unverified) as conventions; no
standard defines them) → manual form (§2.2 step 3e). Never fetch autoconfig over plain HTTP, and show
the discovered hosts to the user before sending a password.

### 5.2 Authentication

Mechanism preference: `XOAUTH2` (Microsoft, Google) → `OAUTHBEARER` (RFC 7628) → `PLAIN` (TLS only) →
`LOGIN` (TLS only, legacy). Always re-read `CAPABILITY` after TLS and after auth; capabilities change.

SASL XOAUTH2 initial response (both Microsoft and Google):

```
base64( "user=" + {email} + "\x01" + "auth=Bearer " + {access_token} + "\x01" + "\x01" )
```

| Protocol | Exchange |
|---|---|
| IMAP | `A01 AUTHENTICATE XOAUTH2 {base64}` (single line when `SASL-IR` is advertised; otherwise send `AUTHENTICATE XOAUTH2`, wait for `+`, then the base64) |
| SMTP | `AUTH XOAUTH2 {base64}` → `235 2.7.0 …`; or `AUTH XOAUTH2`, `334`, then base64 (Microsoft's own sample uses this two-step form and answers `235 2.7.0 Authentication successful`; Google answers `235 2.7.0 Accepted`) |
| Failure (Google) | Server sends a `+` / `334` continuation whose base64 decodes to a JSON object with `status` (e.g. `"401"`), `schemes` and `scope` (`"https://mail.google.com/"`); client **must reply with an empty line**, then gets `NO SASL authentication failed` / `535 5.7.x`. Correction: Google's published sample decodes to `"schemes":"bearer mac"`, not `"bearer"`, and its SMTP sample ends in `535-5.7.1 … Username and Password not accepted`, not `5.7.8` (5.7.8 is commonly seen for password failures (unverified)). Key on the `status` field and on any `535`, never on `schemes` or the enhanced code |
| Failure (Microsoft) | `NO AUTHENTICATE failed.` / `535 5.7.3 Authentication unsuccessful` (both in Microsoft's samples) |

On auth failure with OAuth: refresh the token once and retry; if it still fails → `needs_reauth`.
Shared mailbox (Microsoft): token for the signed-in user, `user=` set to the shared mailbox address.

Basic authentication facts that drive this design:

| Service | Status |
|---|---|
| Outlook.com / personal Microsoft accounts | Basic auth for third-party mail apps stopped working on **16 Sep 2024**; OAuth2 is required |
| Exchange Online IMAP, POP, EAS, EWS | Basic auth disabled in all tenants (from 1 Oct 2022, completed early 2023); cannot be re-enabled |
| Exchange Online SMTP AUTH | Basic auth still available but scheduled (Message Center MC786329, last updated 27 Jan 2026, which put an earlier, faster schedule on hold): unchanged through Dec 2026; disabled by default for existing tenants at the end of **Dec 2026**, with admins able to re-enable it; unavailable by default for tenants created after Dec 2026; final removal date to be announced in H2 2027 (verify — timeline has changed several times). Irrelevant to Axentra's own sign-in (OAuth only) but explains user reports of other clients breaking |
| Google | Password-only ("less secure apps") access for IMAP/POP/SMTP/CalDAV/CardDAV turned off on **14 Mar 2025** (Google's article says "for all Google Accounts"; new first-time LSA connections were blocked from summer 2024); OAuth required, app passwords excepted |

Consequence: Axentra never offers password sign-in for Microsoft or Google domains.

### 5.3 Capabilities used

| Capability | RFC | Use | If absent |
|---|---|---|---|
| `IMAP4rev1` | 3501 | Baseline | Refuse server |
| `IMAP4rev2` | 9051 | `ENABLE IMAP4rev2`; UTF-8 mailbox names, built-in ESEARCH/MOVE/UIDPLUS/SPECIAL-USE semantics | Stay on rev1 |
| `IDLE` | 2177 | Push for INBOX | Poll (`NOOP`/`STATUS`) every 2 min (estimate) |
| `CONDSTORE` | 7162 | `MODSEQ`, `CHANGEDSINCE` for cheap flag sync | Full flag fetch over the window |
| `QRESYNC` | 7162 | `SELECT … (QRESYNC (…))`, `VANISHED` for deletions | UID set diff via `UID SEARCH`/`ESEARCH` |
| `UIDPLUS` | 4315 | `APPENDUID`, `COPYUID`, `UID EXPUNGE` | Re-scan target folder; avoid `EXPUNGE` of other users' `\Deleted` |
| `MOVE` | 6851 | `UID MOVE` | `UID COPY` + `UID STORE +FLAGS.SILENT (\Deleted)` + `UID EXPUNGE` |
| `SPECIAL-USE` | 6154 | Folder roles from `LIST` attributes | Name heuristics (§5.4) |
| `LIST-EXTENDED`, `LIST-STATUS` | 5258, 5819 | One `LIST` with `RETURN (SPECIAL-USE CHILDREN STATUS (…))` | `LIST` then `STATUS` per folder |
| `ESEARCH` | 4731 | `UID SEARCH RETURN (ALL COUNT)` compact results | Plain `SEARCH` |
| `SASL-IR` | 4959 | One-round-trip auth | Two-step auth |
| `ENABLE` | 5161 | Turn on QRESYNC, IMAP4rev2, UTF8=ACCEPT | — |
| `ID` | 2971 | Identify client | — |
| `NAMESPACE` | 2342 | Personal prefix (e.g. `INBOX.`) and delimiter | Use `LIST "" ""` delimiter |
| `LITERAL+` / `LITERAL-` | 7888 | Non-synchronising literals for APPEND | Synchronising literals |
| `BINARY` | 3516 | Fetch decoded parts | Decode CTE locally |
| `UTF8=ACCEPT` | 6855 | UTF-8 in names/headers | Modified UTF-7 mailbox names |
| `COMPRESS=DEFLATE` | 4978 | Bandwidth | — |
| `OBJECTID` | 8474 | `EMAILID`, `THREADID` (stable IDs across moves) | Message-ID + heuristics |
| `PREVIEW` | 8970 | Server-generated preview text | Fetch first 2 KB of the text part |
| `UNSELECT` | 3691 | Leave mailbox without expunge | `EXAMINE` a dummy / `CLOSE` carefully |
| `X-GM-EXT-1` | Gmail | `X-GM-MSGID`, `X-GM-THRID`, `X-GM-LABELS`, `X-GM-RAW` search | — |

Do not assume any capability for a given provider; probe at runtime. Record the post-auth capability
set on the account row for diagnostics.

Exchange Online in practice: no Microsoft page lists the IMAP capabilities. A published 2020 protocol log
of `outlook.office365.com` shows, before login:
`IMAP4 IMAP4rev1 AUTH=PLAIN AUTH=XOAUTH2 SASL-IR UIDPLUS MOVE ID UNSELECT CHILDREN IDLE NAMESPACE LITERAL+`,
and a 2022 JavaMail trace reportedly lacked `MOVE` (both third-party, unverified). Neither shows
`CONDSTORE`, `QRESYNC`, `ENABLE`, `SPECIAL-USE`, `LIST-EXTENDED`, `LIST-STATUS`, `ESEARCH`, `BINARY` or
`COMPRESS`. Plan for the Exchange IMAP fallback to run entirely on the "If absent" column: full flag
fetch over the window, UID-set diff for deletions, `LIST` + per-folder `STATUS`, and name heuristics for
roles ("Sent Items", "Deleted Items", "Junk Email", "Archive", "Drafts").

Order rules the table implies but did not state: send `ENABLE QRESYNC` (which also enables CONDSTORE)
before any `SELECT … (QRESYNC …)`; and `HIGHESTMODSEQ` may appear in `STATUS`/`LIST … RETURN (STATUS …)`
only when `CONDSTORE` is advertised.

### 5.4 Folder discovery and role mapping

`LIST "" "*" RETURN (SPECIAL-USE CHILDREN STATUS (MESSAGES UNSEEN UIDNEXT UIDVALIDITY HIGHESTMODSEQ))`
— only when `LIST-EXTENDED`, `LIST-STATUS` and `SPECIAL-USE` are advertised (drop `HIGHESTMODSEQ`
without `CONDSTORE`); otherwise plain `LIST "" "*"` followed by `STATUS` per selectable folder.

| Axentra role | IMAP signal (in priority order) | Name fallbacks (case-insensitive, any locale list we ship) |
|---|---|---|
| Inbox | Mailbox named `INBOX` (always case-insensitive per RFC) | — |
| Drafts | `\Drafts` | Drafts, Draft |
| Sent | `\Sent` | Sent Items, Sent, Sent Mail, Sent Messages, `[Gmail]/Sent Mail` |
| Deleted Items | `\Trash` | Deleted Items, Trash, Bin, Deleted Messages |
| Junk Email | `\Junk` | Junk Email, Junk, Spam, Bulk Mail |
| Archive | `\Archive` | Archive, Archives |
| All Mail (Gmail) | `\All` | — (shown as a normal folder; **not** the Archive role) |
| Starred (Gmail) | `\Flagged` | — (virtual; hide by default) |
| Important (Gmail) | `\Important` (RFC 8457) | — (hide by default) |

Rules: one folder per role (first match wins; user can override in Account settings). Folders with
`\Noselect` or `\NonExistent` are tree nodes only. Store the server name exactly (raw bytes or decoded
modified UTF-7) and the delimiter; display only the last segment. If a role is missing, create it on
first need with `CREATE "Archive" (USE (\Archive))` when `CREATE-SPECIAL-USE` is advertised, else plain
`CREATE`.

Gmail specifics: "Archive" = move from `INBOX` to the `\All` mailbox (removes the Inbox label);
"Delete" = move to `\Trash`; `\Flagged` = starred.

### 5.5 Sync algorithm (per folder)

State per folder: `uidvalidity`, `uidnext`, `highest_modseq`, `synced_min_uid` (window floor).

1. `SELECT` (or `EXAMINE` for read-only background passes). If `UIDVALIDITY` ≠ stored → delete all local
   messages for the folder and resync from scratch.
2. **New mail**: `UID FETCH {stored_uidnext}:* (UID FLAGS INTERNALDATE RFC822.SIZE ENVELOPE BODYSTRUCTURE
   [MODSEQ] [EMAILID THREADID | X-GM-MSGID X-GM-THRID X-GM-LABELS] BODY.PEEK[HEADER.FIELDS (REFERENCES
   IN-REPLY-TO LIST-UNSUBSCRIBE IMPORTANCE X-PRIORITY)])`. Pitfall: `N:*` returns the highest message
   even when its UID < N; discard UIDs below `stored_uidnext`.
3. **Flag changes**:
   - QRESYNC: `SELECT {mbox} (QRESYNC ({uidvalidity} {modseq}))` returns `VANISHED (EARLIER)` and changed
     `FETCH` lines in one go.
   - CONDSTORE only: `UID FETCH {synced_min_uid}:* (FLAGS) (CHANGEDSINCE {modseq})`.
   - Neither: `UID FETCH {synced_min_uid}:* (FLAGS)` and diff locally (cap window to 5,000 newest (estimate)).
4. **Deletions** (no QRESYNC): `UID SEARCH RETURN (ALL) UID {synced_min_uid}:*` (ESEARCH) or plain
   `UID SEARCH UID …`; local UIDs not returned are removed.
5. **Previews**: `PREVIEW` if available, else `BODY.PEEK[{textPart}]<0.2048>` decoded and trimmed to 255
   chars to match Graph's `bodyPreview`.
6. **Backfill** older mail in pages of 200 UIDs (estimate) going down from `synced_min_uid`, initial
   window 30 days (estimate) using `UID SEARCH SINCE {date}`.
7. Persist `uidnext`, `highest_modseq` only after the local transaction commits.

Always use `BODY.PEEK[...]` (never `BODY[...]`) for background fetches so `\Seen` is not set.

IDLE loop (INBOX connection): `IDLE` → react to untagged `EXISTS`, `EXPUNGE`, `FETCH`, `VANISHED` by
scheduling step 2/3 on the foreground connection → `DONE` and re-`IDLE` every 25 min (estimate; RFC 2177
lets servers drop idle clients after 30 min). On connection loss: reconnect with backoff 1 s → 60 s
(estimate).

Moves: with `MOVE`/`COPYUID` we learn new UIDs and update the row in place (keep local id, change
`folder_id`, `imap_uid`); otherwise delete locally and let the target folder's sync add it.

### 5.6 Flags and keywords mapping

| Axentra field | IMAP | Graph equivalent |
|---|---|---|
| `is_read` | `\Seen` | `isRead` |
| `flag_status = flagged` | `\Flagged` set | `flag.flagStatus = flagged` |
| `flag_status = complete` | no standard flag; keyword `$Completed` (Axentra-private, estimate) | `flag.flagStatus = complete` |
| `is_answered` | `\Answered` | (no direct field; derive from conversation) |
| `is_forwarded` | `$Forwarded` | — |
| `is_draft` | `\Draft` | `isDraft` |
| pending expunge | `\Deleted` (hidden in UI; never shown as "deleted but present") | — |
| `is_junk` hint | `$Junk` / `$NotJunk` (set when user marks junk/not junk, plus move) | move to `junkemail` |
| read receipt sent | `$MDNSent` | — |
| `categories` | other keywords (e.g. `$label1`…, or user keywords) mapped by name; Gmail: `X-GM-LABELS` | `categories` |

Commands: `UID STORE {uids} +FLAGS.SILENT (\Seen)` / `-FLAGS.SILENT`; with CONDSTORE insert the store
modifier between the UID set and the operation — `UID STORE {uids} (UNCHANGEDSINCE {modseq}) +FLAGS.SILENT (\Seen)`
(RFC 7162 syntax) — and treat UIDs returned in the `[MODIFIED …]` response code as conflicts. Check `PERMANENTFLAGS` from `SELECT`: if `\*` is
absent, custom keywords can't be stored — disable categories for that folder.

### 5.7 Message operations on IMAP

| User action | IMAP sequence |
|---|---|
| Delete (not in Trash) | `UID MOVE {uid} {Trash}` |
| Delete in Trash / Shift+Del | `UID STORE {uid} +FLAGS.SILENT (\Deleted)` then `UID EXPUNGE {uid}` (requires UIDPLUS; otherwise `EXPUNGE` only if no other `\Deleted` messages exist, else ask) |
| Archive | `UID MOVE {uid} {Archive}` (Gmail: to `\All`) |
| Junk / Not junk | `UID MOVE` to Junk / Inbox + `$Junk`/`$NotJunk` keywords |
| Save draft | `APPEND {Drafts} (\Draft \Seen) {date} {literal}`; read `APPENDUID`; then delete the previous draft UID |
| Save sent copy | `APPEND {Sent} (\Seen) {literal}` per §5.9 policy |
| Mark all read | `UID STORE 1:* +FLAGS.SILENT (\Seen)` on the selected folder |

### 5.8 Server-side search

Map the Axentra search box syntax to `UID SEARCH CHARSET UTF-8 …` (IMAP4rev2 is UTF-8 by default):

| Axentra token | IMAP criterion | Gmail (`X-GM-RAW`) |
|---|---|---|
| free text | `TEXT "…"` (servers may search headers+body) | `"…"` |
| `from:` | `FROM "…"` | `from:` |
| `to:` / `cc:` | `TO "…"` / `CC "…"` | same |
| `subject:` | `SUBJECT "…"` | same |
| `is:unread` / `is:read` | `UNSEEN` / `SEEN` | same |
| `is:flagged` | `FLAGGED` | `is:starred` |
| `after:` / `before:` | `SINCE dd-Mon-yyyy` / `BEFORE dd-Mon-yyyy` (date only, server zone) | same |
| `has:attachment` | no standard criterion; `HEADER Content-Type "multipart/mixed"` heuristic, then refine locally | `has:attachment` |
| `larger:` | `LARGER n` | `larger:` |

Search the current folder on the server; for "All folders", search locally (SQLite FTS5) first and
run server searches folder by folder in the background. Use `ESEARCH RETURN (ALL COUNT)` when present.
Results merge by `(folder_id, imap_uid)`.

### 5.9 SMTP submission and Sent copies

Sequence: `EHLO {fqdn-or-[ip]}` → (STARTTLS → `EHLO`) → `AUTH …` → `MAIL FROM:<{from}> [SIZE=n] [BODY=8BITMIME] [SMTPUTF8]` →
`RCPT TO:<…>` per recipient (To, Cc, **and Bcc**) → `DATA` → message → `.` → `QUIT`.

Extensions to honour: `SIZE` (RFC 1870, pre-check message size), `8BITMIME` (RFC 6152), `SMTPUTF8`
(RFC 6531, needed for non-ASCII addresses), `PIPELINING` (RFC 2920), `ENHANCEDSTATUSCODES` (RFC 2034),
`DSN` (RFC 3461, optional).

Message construction (RFC 5322 / MIME): `Message-ID: <{uuid}@{from-domain}>` generated client-side and
stored before sending; `Date`; `From`; `To`; `Cc`; **no `Bcc` header in the transmitted copy**;
`In-Reply-To` and `References` for replies; `multipart/alternative` (text/plain + text/html), wrapped
in `multipart/related` for inline `cid:` images, wrapped in `multipart/mixed` for attachments.
Headers with non-ASCII use RFC 2047 encoded-words unless SMTPUTF8 is in use.

Reply codes:

| Code | Handling |
|---|---|
| `4xx` | Keep in Outbox, retry with backoff (1, 5, 15, 60 min (estimate)) |
| `5xx` on RCPT | Show per-recipient failure, keep draft |
| `535 5.7.3` (Microsoft) / `535 5.7.1` in Google's XOAUTH2 sample (`5.7.8` also seen) — i.e. any `535` | Auth failed → refresh token once → `needs_reauth` |
| `535 5.7.139` (Microsoft, SMTP AUTH disabled for tenant/mailbox) (unverified: the code and its "SmtpClientAuthentication is disabled" text come from community reports; Microsoft's SMTP AUTH page documents the switch but not the reply) | Error: "Sending is turned off for this account by your organization." Suggest Graph |
| `5.7.60` (Microsoft, documented NDR) | Sending from an address the signed-in mailbox lacks Send As rights for: "You don't have permission to send as {from}." |
| `552` / SIZE exceeded | "This message is too large to send." |

Sent-copy policy (per account, column `sent_copy_policy`):

| Provider | Policy | Reason |
|---|---|---|
| Gmail | `server`, confirmed by one `detect` probe on the first send | Gmail files SMTP-submitted mail into Sent Mail itself and offers no switch to stop it; appending would duplicate. Consistent across client bug trackers and vendor KB articles, but no current Google page states it (unverified first-party) |
| Microsoft 365 via SMTP AUTH | `server` | Documented: Microsoft's comparison of sending methods marks client SMTP submission as "Saves to Sent Items folder: Yes" |
| Outlook.com via SMTP | `detect` | Not documented for consumer accounts (no Microsoft page found) |
| Generic | `detect` | Varies |

`detect`: 30 s after send (estimate), `UID SEARCH HEADER Message-ID "{id}"` in the Sent folder; if not
found, `APPEND` the exact transmitted bytes (with the Bcc header re-added so the Sent copy shows Bcc)
and switch the account to `client_append`; if found, switch to `server`. Graph accounts never append —
`send`/`sendMail` save to Sent Items (`saveToSentItems` defaults to true).

---

## 6. CalDAV / CardDAV (non-Microsoft calendars and contacts)

Brief by design; Microsoft accounts never use DAV.

| Step | CalDAV (RFC 4791) | CardDAV (RFC 6352) |
|---|---|---|
| Discovery | `/.well-known/caldav`, DNS SRV `_caldavs._tcp` (RFC 6764) | `/.well-known/carddav`, `_carddavs._tcp` |
| Principal | `PROPFIND` `DAV:current-user-principal` (RFC 5397) | same |
| Home | `CALDAV:calendar-home-set` | `CARDDAV:addressbook-home-set` |
| Collections | `PROPFIND Depth: 1` for `resourcetype`, `displayname`, `CS:getctag`, `DAV:sync-token`, `CALDAV:supported-calendar-component-set` (VEVENT/VTODO), `apple:calendar-color`, `CALDAV:calendar-timezone` | `resourcetype`, `displayname`, `sync-token` |
| Incremental sync | `REPORT DAV:sync-collection` with stored `sync-token` (RFC 6578); fallback: compare `getctag`, then ETags | same |
| Fetch | `REPORT calendar-multiget` for changed hrefs; window queries via `calendar-query` + `time-range` | `addressbook-multiget` |
| Create | `PUT {href}.ics` with `If-None-Match: *` | `PUT {href}.vcf` with `If-None-Match: *` |
| Update / delete | `PUT` / `DELETE` with `If-Match: {etag}`; 412 → refetch and merge | same |
| Auth | OAuth bearer (Google) or Basic over TLS with app password | same |

Google endpoints (checked against Google's CalDAV and CardDAV guides, 2026-10-19):

- **CalDAV**: events collection `https://apidata.googleusercontent.com/caldav/v2/{calendarId}/events`;
  principal URL for clients that start from a principal `https://apidata.googleusercontent.com/caldav/v2/{calendarId}/user`.
  `{calendarId}` is the user's email address for the primary calendar. The old
  `https://www.google.com/calendar/dav` base is deprecated and unsupported. The CalDAV API must be
  enabled in the Google Cloud project. OAuth 2.0 over HTTPS only (Basic auth → 401). Supported:
  `sync-collection` (switch to it after the initial sync), all REPORTs except `free-busy-query`, ctag.
  Not supported: VTODO/VJOURNAL (so no Google tasks over DAV), MKCALENDAR, LOCK/COPY/MOVE, custom
  properties, ACL; the scheduling inbox is always empty and invitations land in the events
  collection. Scope: the guide names none; `https://www.googleapis.com/auth/calendar` is the full
  Calendar API scope and what other DAV clients request (unverified for CalDAV specifically). Quotas
  follow the Calendar API.
- **CardDAV**: `PROPFIND https://www.googleapis.com/.well-known/carddav` → 301 to the address book;
  do not hard-code the target or cache the redirect permanently (Google suggests re-checking every 2–4
  weeks). Address book paths look like `…/carddav/v1/principals/{email}/lists/default` but must be
  discovered. vCard **3.0 only**; `sync-collection` supported (tokens valid 29 days), ctag/ETag fallback
  (frequent ctag polling is throttled); create contacts with `POST` to the collection (RFC 5995
  add-member), update/delete with `PUT`/`DELETE` + `If-Match` (412 on conflict). No LOCK/COPY/MOVE/MKCOL.
  OAuth only. Scope: Google's CardDAV page and scope list name none; `https://www.googleapis.com/auth/carddav`
  is what other DAV clients request (unverified).

Correction to the table above for Google: "Create" uses `POST` (not `PUT … If-None-Match: *`) on
CardDAV, and "Auth" is OAuth bearer only — app passwords do not work for Google DAV.

Invitations for non-Graph accounts: incoming `text/calendar; method=REQUEST` parts (iTIP RFC 5546 over
iMIP RFC 6047) render RSVP buttons; replying sends an email with `METHOD:REPLY` and the user's
`ATTENDEE;PARTSTAT=…` to the organizer, and writes the event into the account's DAV calendar (or a
local-only calendar if none). If the server supports CalDAV scheduling (RFC 6638), updating `PARTSTAT`
on the server copy is enough; don't also send iMIP (avoid double replies).

Local recurrence expansion is required for DAV (Graph does it server-side): expand `RRULE`, `RDATE`,
`EXDATE`, and override instances (`RECURRENCE-ID`) over the visible window (e.g. the `rrule` crate).

---

## 7. Secure credential storage on Linux

### 7.1 What is a secret

| Item | Where |
|---|---|
| OAuth refresh tokens (Microsoft, Google) | Secret Service |
| IMAP/SMTP/DAV passwords, app passwords | Secret Service |
| Access tokens | Memory (optional Secret Service cache with expiry) |
| PKCE verifier, `state`, device code | Memory only, wiped after the flow |
| Account metadata (email, hosts, ports, tenant id) | SQLite (not secret) |

Never write secrets to SQLite, config files, logs, crash dumps or protocol traces. Redact
`Authorization` headers, `AUTHENTICATE`/`AUTH` lines (the XOAUTH2 base64 contains the token), and
`refresh_token`/`access_token`/`code` values in any debug output. Hold secrets in `secrecy::SecretString`
/ `zeroize` buffers.

### 7.2 Secret Service API (freedesktop.org) — names Axentra uses

| Element | Value |
|---|---|
| Bus / name | Session bus, `org.freedesktop.secrets` |
| Service object | `/org/freedesktop/secrets` (interface `org.freedesktop.Secret.Service`) |
| Default collection | Alias `default` → `ReadAlias("default")`, path `/org/freedesktop/secrets/aliases/default` |
| Session | `OpenSession(algorithm, input) → (output, session)`; algorithm `dh-ietf1024-sha256-aes128-cbc-pkcs7` preferred, `plain` allowed (session bus is per-user) |
| Lookup | `SearchItems(attributes a{ss}) → (unlocked ao, locked ao)` |
| Unlock | `Unlock(objects ao) → (unlocked ao, prompt o)`; if `prompt` ≠ `/`, call `org.freedesktop.Secret.Prompt.Prompt(window_id)` and wait for `Completed(dismissed b, result v)` |
| Read | `GetSecrets(items ao, session o)` or `Item.GetSecret(session)` |
| Write | `Collection.CreateItem(properties a{sv}, secret, replace=true) → (item, prompt)` with `org.freedesktop.Secret.Item.Label` and `org.freedesktop.Secret.Item.Attributes` |
| Delete | `Item.Delete() → prompt` (on account removal) |
| Secret struct | `(session o, parameters ay, value ay, content_type s)`; content type `text/plain; charset=utf8` |

Attribute schema (attributes are **not** encrypted — no secrets, no tokens in attributes or labels):

| Attribute | Example |
|---|---|
| `xdg:schema` | `app.axentra.Outlook.Credential` |
| `account_id` | `6f1c…` (UUID, matches `account.id_uuid`) |
| `kind` | `oauth_refresh_token` · `password` · `smtp_password` · `dav_password` · `access_token_cache` |
| `provider` | `microsoft` · `google` · `imap` |
| `username` | `ana@example.com` |

Label: `Axentra Outlook – ana@example.com ({provider})`. The secret value for OAuth is a small JSON
document: `{"refresh_token":"…","resource":"graph","tenant":"{tid}","scopes":"…"}` (one item per
resource if IMAP fallback tokens exist).

### 7.3 Backends and fallbacks

| Environment | Backend |
|---|---|
| GNOME / KDE (gnome-keyring, KWallet ≥ 5.97 Secret Service, KeePassXC with Secret Service enabled) | D-Bus Secret Service (above) |
| Flatpak | `org.freedesktop.portal.Secret.RetrieveSecret(IN fd h, IN options a{sv}, OUT handle o)`: the app passes a writable fd; the portal writes a per-app master secret to it and completes via the returned `Request` object. The secret is opaque, stable while the app stays installed, and may be short, so expand it with a KDF (e.g. HKDF-SHA256) before using it to encrypt a local keyring file inside the sandbox. If the `Response` results carry a `token`, pass it back in `options.token` on later calls. The `oo7` crate implements this pattern |
| No Secret Service and no portal (minimal WM, SSH) | Blocking dialog "Axentra Outlook can't find a keyring" with options **Use a passphrase** (encrypted file: Argon2id m=64 MiB, t=3, p=1 (estimate) + XChaCha20-Poly1305, file mode 0600 in `$XDG_DATA_HOME/axentra-outlook/secrets.bin`) or **Don't remember** (sign in every launch) |
| Keyring locked at startup | Account state `keyring_locked`; sync paused; **Unlock** calls `Unlock` with a prompt |

Recommended crate: `oo7` (pure Rust, async, D-Bus Secret Service + portal file backend). Alternatives:
`secret-service`, `keyring`. Pass the window identifier to prompts when available (X11 window id as
decimal string; on Wayland use an xdg-foreign handle if the toolkit exposes one, else empty) (estimate).

---

## 8. Local SQLite model and field mapping

### 8.1 Schema (core tables)

```sql
CREATE TABLE account (
  id INTEGER PRIMARY KEY, id_uuid TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL CHECK (kind IN ('graph','imap')),
  provider TEXT NOT NULL,              -- microsoft_work | microsoft_personal | google | imap
  email TEXT NOT NULL, display_name TEXT, color TEXT,
  ms_oid TEXT, ms_tid TEXT, authority TEXT,           -- Microsoft only
  imap_host TEXT, imap_port INTEGER, imap_security TEXT, imap_username TEXT,
  smtp_host TEXT, smtp_port INTEGER, smtp_security TEXT, smtp_username TEXT,
  auth_method TEXT,                    -- oauth_graph | xoauth2 | oauthbearer | plain
  caldav_url TEXT, carddav_url TEXT,
  sent_copy_policy TEXT DEFAULT 'detect',
  capabilities TEXT,                   -- JSON array, post-auth IMAP caps
  status TEXT NOT NULL DEFAULT 'ok', last_error TEXT, last_sync_at INTEGER,
  sort_order INTEGER, created_at INTEGER NOT NULL
);
CREATE TABLE folder (
  id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  remote_id TEXT NOT NULL,             -- Graph folder id | IMAP mailbox name (UTF-8)
  parent_id INTEGER REFERENCES folder(id), display_name TEXT NOT NULL,
  role TEXT,                           -- inbox|drafts|sent|trash|junk|archive|outbox|all|conversation_history|NULL
  delimiter TEXT, is_hidden INTEGER DEFAULT 0, is_selectable INTEGER DEFAULT 1,
  total_count INTEGER, unread_count INTEGER,
  uidvalidity INTEGER, uidnext INTEGER, highest_modseq INTEGER, synced_min_uid INTEGER,
  delta_link TEXT, is_favorite INTEGER DEFAULT 0, sort_order INTEGER,
  UNIQUE (account_id, remote_id)
);
CREATE TABLE message (
  id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL, folder_id INTEGER NOT NULL REFERENCES folder(id),
  remote_id TEXT,                      -- Graph immutable id | NULL for IMAP
  imap_uid INTEGER, email_id TEXT,     -- OBJECTID EMAILID or X-GM-MSGID
  internet_message_id TEXT, conversation_id TEXT,
  subject TEXT, preview TEXT,
  from_name TEXT, from_address TEXT, sender_name TEXT, sender_address TEXT,
  to_json TEXT, cc_json TEXT, bcc_json TEXT, reply_to_json TEXT,
  received_at INTEGER, sent_at INTEGER,
  is_read INTEGER, is_draft INTEGER, is_answered INTEGER, is_forwarded INTEGER,
  flag_status TEXT, flag_start_at INTEGER, flag_due_at INTEGER, flag_completed_at INTEGER,
  importance TEXT, inference TEXT, has_attachments INTEGER, categories_json TEXT,
  size INTEGER, change_key TEXT, modseq INTEGER, last_modified_at INTEGER,
  body_state TEXT DEFAULT 'none',      -- none | preview | full
  local_state TEXT DEFAULT 'synced',   -- synced | pending
  UNIQUE (account_id, remote_id), UNIQUE (folder_id, imap_uid)
);
CREATE TABLE attachment (id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL REFERENCES message(id) ON DELETE CASCADE,
  remote_id TEXT, imap_part TEXT, name TEXT, content_type TEXT, size INTEGER,
  is_inline INTEGER, content_id TEXT, local_path TEXT);
CREATE TABLE calendar (id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL, remote_id TEXT NOT NULL,
  name TEXT, hex_color TEXT, color_enum TEXT, is_default INTEGER, can_edit INTEGER, owner_address TEXT,
  delta_link TEXT, window_start INTEGER, window_end INTEGER, dav_sync_token TEXT, dav_ctag TEXT,
  UNIQUE (account_id, remote_id));
CREATE TABLE event (id INTEGER PRIMARY KEY, calendar_id INTEGER NOT NULL REFERENCES calendar(id) ON DELETE CASCADE,
  remote_id TEXT, ical_uid TEXT, series_master_remote_id TEXT, event_type TEXT, recurrence_id TEXT,
  subject TEXT, body_html TEXT, preview TEXT,
  start_utc INTEGER, end_utc INTEGER, start_tz TEXT, end_tz TEXT, is_all_day INTEGER,
  location TEXT, locations_json TEXT, show_as TEXT, sensitivity TEXT, importance TEXT,
  is_cancelled INTEGER, is_organizer INTEGER, organizer_name TEXT, organizer_address TEXT,
  response TEXT, response_at INTEGER, response_requested INTEGER, allow_new_time_proposals INTEGER,
  is_reminder_on INTEGER, reminder_minutes INTEGER, recurrence_json TEXT, online_meeting_url TEXT,
  categories_json TEXT, change_key TEXT, etag TEXT, transaction_id TEXT, last_modified_at INTEGER);
CREATE TABLE attendee (event_id INTEGER NOT NULL REFERENCES event(id) ON DELETE CASCADE,
  name TEXT, address TEXT, kind TEXT, response TEXT, response_at INTEGER);
CREATE TABLE contact (id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL, folder_remote_id TEXT,
  remote_id TEXT, vcard_uid TEXT, display_name TEXT, given_name TEXT, middle_name TEXT, surname TEXT,
  nickname TEXT, title TEXT, suffix TEXT, company TEXT, job_title TEXT, department TEXT,
  emails_json TEXT, business_phones_json TEXT, home_phones_json TEXT, mobile_phone TEXT,
  addresses_json TEXT, im_json TEXT, birthday TEXT, notes TEXT, categories_json TEXT,
  photo_path TEXT, change_key TEXT, etag TEXT, last_modified_at INTEGER);
CREATE TABLE task_list (id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL, remote_id TEXT NOT NULL,
  display_name TEXT, wellknown TEXT, is_owner INTEGER, is_shared INTEGER, delta_link TEXT,
  UNIQUE (account_id, remote_id));
CREATE TABLE task (id INTEGER PRIMARY KEY, list_id INTEGER NOT NULL REFERENCES task_list(id) ON DELETE CASCADE,
  remote_id TEXT, title TEXT, body TEXT, status TEXT, importance TEXT,
  due_date TEXT, start_date TEXT, reminder_at INTEGER, is_reminder_on INTEGER, completed_at INTEGER,
  recurrence_json TEXT, categories_json TEXT, checklist_json TEXT, linked_json TEXT,
  created_at INTEGER, last_modified_at INTEGER, etag TEXT);
CREATE TABLE pending_op (id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL, op TEXT NOT NULL,
  target_kind TEXT, target_local_id INTEGER, payload_json TEXT, created_at INTEGER,
  attempts INTEGER DEFAULT 0, next_attempt_at INTEGER, last_error TEXT);
```

### 8.2 Message mapping

| Column | Graph (`message`) | IMAP | Notes |
|---|---|---|---|
| `remote_id` | `id` (immutable) | — | |
| `imap_uid` / `email_id` | — | `UID`; `EMAILID` or `X-GM-MSGID` | |
| `internet_message_id` | `internetMessageId` | ENVELOPE message-id | Used for dedupe and Sent detection |
| `conversation_id` | `conversationId` | `THREADID` / `X-GM-THRID` / computed from References | |
| `subject` | `subject` | ENVELOPE subject (RFC 2047 decoded) | |
| `preview` | `bodyPreview` (≤ 255 chars) | PREVIEW or first text bytes | |
| `from_*` / `sender_*` | `from.emailAddress.{name,address}` / `sender…` | ENVELOPE from / sender | |
| `to_json`… | `toRecipients[].emailAddress` etc. | ENVELOPE to/cc/bcc/reply-to | JSON `[{"name":…,"address":…}]` |
| `received_at` | `receivedDateTime` | `INTERNALDATE` | List sort key |
| `sent_at` | `sentDateTime` | ENVELOPE date | |
| `is_read` | `isRead` | `\Seen` | |
| `is_draft` | `isDraft` | `\Draft` | |
| `flag_*` | `flag.flagStatus`, `flag.startDateTime`, `flag.dueDateTime`, `flag.completedDateTime` | `\Flagged` (+ `$Completed`) | |
| `importance` | `importance` (`low`/`normal`/`high`) | `Importance:` / `X-Priority:` header | |
| `inference` | `inferenceClassification` | NULL (Focused disabled on IMAP) | |
| `has_attachments` | `hasAttachments` | BODYSTRUCTURE has non-inline parts | |
| `categories_json` | `categories` | keywords / `X-GM-LABELS` | |
| `size` | — | `RFC822.SIZE` | |
| `change_key` / `modseq` | `changeKey` | `MODSEQ` | Conflict detection |

### 8.3 Event, contact, task mapping

| Event column | Graph (`event`) | iCalendar (CalDAV) |
|---|---|---|
| `remote_id` / `ical_uid` | `id` / `iCalUId` | href / `UID` |
| `event_type` | `type` (`singleInstance`, `occurrence`, `exception`, `seriesMaster`) | derived (RRULE, RECURRENCE-ID) |
| `subject`, `body_html`, `preview` | `subject`, `body.content`, `bodyPreview` | `SUMMARY`, `X-ALT-DESC;FMTTYPE=text/html` or `DESCRIPTION` |
| `start_utc`/`end_utc`, `*_tz` | `start`/`end` (`dateTime`,`timeZone`), `originalStartTimeZone` | `DTSTART`/`DTEND`/`DURATION` with `TZID` |
| `is_all_day` | `isAllDay` | `VALUE=DATE` |
| `location`, `locations_json` | `location.displayName`, `locations` | `LOCATION` |
| `show_as` | `showAs` (`free`, `tentative`, `busy`, `oof`, `workingElsewhere`, `unknown`) | `X-MICROSOFT-CDO-BUSYSTATUS` else `TRANSP` (OPAQUE→busy, TRANSPARENT→free) |
| `sensitivity` | `sensitivity` (`normal`, `personal`, `private`, `confidential`) | `CLASS` (PUBLIC/PRIVATE/CONFIDENTIAL) |
| `importance` | `importance` | `PRIORITY` (1–4 high, 5 normal, 6–9 low) |
| `is_cancelled` | `isCancelled` | `STATUS:CANCELLED` |
| `organizer_*`, `is_organizer` | `organizer.emailAddress`, `isOrganizer` | `ORGANIZER` vs account address |
| `response` | `responseStatus.response` (`none`, `organizer`, `tentativelyAccepted`, `accepted`, `declined`, `notResponded`) | own `ATTENDEE;PARTSTAT` (NEEDS-ACTION, ACCEPTED, TENTATIVE, DECLINED) |
| attendee rows | `attendees[]{emailAddress, type, status{response,time}}` | `ATTENDEE;ROLE;PARTSTAT;CUTYPE` |
| `reminder_minutes`, `is_reminder_on` | `reminderMinutesBeforeStart`, `isReminderOn` | first `VALARM` `TRIGGER:-PT{n}M` |
| `recurrence_json` | `recurrence` | `RRULE`/`EXDATE`/`RDATE` text |
| `online_meeting_url` | `onlineMeeting.joinUrl` | `X-…` / URL in `LOCATION`/`DESCRIPTION` (heuristic) |
| `change_key` / `etag` | `changeKey` / `@odata.etag` | `ETag` |

| Contact column | Graph (`contact`) | vCard (CardDAV) |
|---|---|---|
| `display_name`, `given_name`, `middle_name`, `surname`, `nickname`, `title`, `suffix` | `displayName`, `givenName`, `middleName`, `surname`, `nickName`, `title`, `generation` | `FN`, `N` components, `NICKNAME` |
| `company`, `job_title`, `department` | `companyName`, `jobTitle`, `department` | `ORG`, `TITLE` |
| `emails_json` | `emailAddresses[]{name,address}`; v1.0 now also documents `primaryEmailAddress`, `secondaryEmailAddress`, `tertiaryEmailAddress` (contact resource page dated 2026-02-27) — read them when present, write `emailAddresses` | `EMAIL;TYPE=` |
| phones | `businessPhones[]`, `homePhones[]`, `mobilePhone` | `TEL;TYPE=work/home/cell` |
| `addresses_json` | `businessAddress`, `homeAddress`, `otherAddress` (`street`, `city`, `state`, `postalCode`, `countryOrRegion`) | `ADR;TYPE=` |
| `im_json`, `birthday`, `notes`, `categories_json` | `imAddresses`, `birthday` (DateTimeOffset in UTC — store the date part only), `personalNotes`, `categories` | `IMPP`, `BDAY`, `NOTE`, `CATEGORIES` |
| `photo_path` | `/contacts/{id}/photo/$value` | `PHOTO` |

| Task column | Graph (`todoTask`) | VTODO |
|---|---|---|
| `title`, `body` | `title`, `body.content` | `SUMMARY`, `DESCRIPTION` |
| `status` | `status` (`notStarted`, `inProgress`, `completed`, `waitingOnOthers`, `deferred`) | `STATUS` (NEEDS-ACTION, IN-PROCESS, COMPLETED, CANCELLED) |
| `importance` | `importance` | `PRIORITY` |
| `due_date`, `start_date` | `dueDateTime`, `startDateTime` (dateTimeTimeZone) | `DUE`, `DTSTART` |
| `reminder_at`, `is_reminder_on` | `reminderDateTime`, `isReminderOn` | `VALARM` |
| `completed_at` | `completedDateTime` | `COMPLETED` |
| `checklist_json` | `checklistItems[]{id,displayName,isChecked,checkedDateTime}` | child VTODOs via `RELATED-TO` |
| `linked_json` | `linkedResources[]` | — |
| list `wellknown`, `is_owner`, `is_shared` | `todoTaskList.wellknownListName`, `isOwner`, `isShared` | — |

### 8.4 Local-first write path

1. User action updates SQLite immediately (`local_state='pending'`) and inserts a `pending_op`.
2. The op runner executes ops per account in FIFO order (coalescing e.g. read→unread→read into the last
   value), respecting the account semaphore.
3. On success: write server values (`changeKey`, new UID, new id) and clear `pending`.
4. On 404/`ErrorItemNotFound`: drop the op and remove the local row if remote deletion is confirmed.
5. On conflict: refetch; re-apply only if the user's change still differs from the server.
6. Incoming sync never overwrites a field that has a newer pending op (avoid flicker on echo).
7. After 10 failed attempts (estimate) mark the op failed and show "Some changes couldn't be saved"
   with **Retry** / **Discard**.

---

## 9. Classic Outlook differences (reference)

| Topic | New Outlook / OWA (mirrored) | Classic Outlook | Axentra decision |
|---|---|---|---|
| Exchange protocol | Cloud service (REST-based) | MAPI/HTTP + EWS + Autodiscover, OST cache | Graph |
| IMAP/POP accounts | Synced through Microsoft cloud | Direct, stored in PST | Direct IMAP, SQLite cache, no POP |
| OAuth for IMAP to Exchange Online | n/a | Not planned by Microsoft for Outlook clients (unverified; no current Microsoft statement found in this pass) | Supported (XOAUTH2 fallback) |
| Offline store | Limited offline cache | Full OST/PST | Full local SQLite cache |
| Add account entry | Folder pane "Add account" (bottom of the folder list); Settings › Accounts › Add account | File › Add Account | Same as new Outlook |

---

## 10. Implementation notes (must not miss)

1. Send `Prefer: IdType="ImmutableId"` on **every** Graph request, including delta and batch
   sub-requests. Remote IDs are case-sensitive; never use `NOCASE` collation on them.
2. One resource per `/token` request. Graph and `outlook.office.com` (IMAP/SMTP) need separate access
   tokens minted from the same refresh token.
3. Always replace the stored refresh token when a new one arrives; make the write atomic and
   single-flight per account.
4. Redeem authorization codes immediately (≈1 min lifetime). Use `127.0.0.1` with an OS-assigned port;
   do not register port-specific or `[::1]` redirect URIs; no query strings in redirect URIs. Never
   combine `login_hint` with `prompt=select_account`.
5. Device code: Microsoft does not return `verification_uri_complete`; honour `interval`; handle
   `slow_down`; requires "Allow public client flows".
6. Never parse or validate Microsoft access tokens; read identity from the ID token and `/me`.
7. Throttling is per app per mailbox: ≤ 4 concurrent requests (we use 3), 10,000 requests/10 min,
   150 MB uploads/5 min. Obey `Retry-After` exactly; batches can return 200 with 429 inside.
8. Treat `@odata.nextLink`/`@odata.deltaLink` as opaque URLs. Persist the deltaLink only after the
   page's changes are committed. On 410 or `syncStateNotFound`, resync that collection.
9. Delta is per folder and per calendar. Filtered message delta caps at 5,000 messages; edits to
   older backfilled mail outside the filter are not reported — refresh them lazily on open — yet
   read/unread flips and `@removed` rows for such mail *can* still arrive, so never assume a delta row
   lies inside the window.
10. Delta can replay changes and repeat an item within a round; upserts must be idempotent. A moved
    message shows as `@removed` in the source folder and as a create in the target with the same id.
11. calendarView delta does not support `$select` and bakes the date range into the token; re-anchor
    the window by starting a new round. The v1.0 reference documents it for the primary calendar only;
    the per-calendar path is a smoke-test item (§4.9). Out-of-range `@removed` rows are normal.
12. `dueDateTime` on a message flag requires `startDateTime`. `proposedNewTime` requires
    `sendResponse: true` and `allowNewTimeProposals`.
13. Attachments ≥ 3 MB need an upload session; never send `Authorization` to the `uploadUrl`; keep
    chunks under 4 MB and in order.
14. Graph `send`/`sendMail` return 202 — that is acceptance, not delivery. Find the Sent copy by the
    draft's immutable ID with retries.
15. Graph change notifications need a public HTTPS webhook; desktop clients poll delta instead.
16. IMAP: always `BODY.PEEK`; discard `N:*` results below `N`; drop the folder cache on UIDVALIDITY
    change; re-read CAPABILITY after STARTTLS and after AUTH.
17. Re-issue IDLE before 29 minutes (we use 25). One IDLE connection per watched folder.
18. XOAUTH2 failure on Google requires an empty client response before the server sends NO/535.
19. Never offer password sign-in for Microsoft or Google domains — basic auth is gone for Outlook.com
    (Sep 2024) and Exchange Online IMAP/POP; SMTP AUTH basic auth is being phased out (Dec 2026 default-off).
20. Strip `Bcc` from the transmitted MIME but keep it in the Sent copy; apply the per-account
    sent-copy policy to avoid duplicates (Exchange Online documents that it saves SMTP AUTH
    submissions to Sent Items; Gmail does the same per consistent community reports).
21. Gmail `\All` is not Archive; Archive on Gmail = move out of INBOX to `\All`.
22. Secret Service attributes and labels are plaintext — put only identifiers there. Redact tokens,
    passwords and XOAUTH2 strings from every log and protocol trace.
23. Handle a locked or missing keyring as a first-class account state, not a crash; Flatpak builds use
    the Secret portal.
24. Never use `/beta` endpoints in shipped code; never use EWS.
25. Graph `DELETE` on a message skips Deleted Items (soft delete into Recoverable Items). Normal Delete
    is a `move` to `deleteditems`, for folders too; `permanentDelete` purges.
26. `GET /me/contactFolders` omits the default Contacts folder; get its id from a contact's
    `parentFolderId`.
27. Outlook delta tokens have no fixed lifetime (server cache eviction); any deltaLink may come back
    `syncStateNotFound` or 410 — full resync of that one collection, never of the account.
28. Microsoft 365 SMTP AUTH is frequently disabled by tenant policy (and by security defaults), which
    blocks OAuth submission too; the IMAP fallback must surface that clearly.
29. Google refresh tokens issued while the consent screen is in "Testing" expire after 7 days; Google
    also caps live refresh tokens at 100 per account per client and silently drops the oldest.
30. `Mail.ReadWrite.Shared`, `Mail.Send.Shared` and `Calendars.ReadWrite.Shared` are not for personal
    Microsoft accounts; request them only for work/school sign-ins.

---

## 11. Open items tracked by this spec

Status after the 2026-10-19 fact-check:

- `mailFolders/delta` hierarchy (§4.6): **still open.** Microsoft's page is silent and community reports
  conflict. Mitigated by the completeness check + per-parent `childFolders/delta` fallback; settle it
  with a tenant test (nested folders three levels deep, then rename/move one).
- Default Contacts folder alias (§4.10): **answered (negative).** No documented alias; `contactFolders`
  omits the root folder (known issue). Use a contact's `parentFolderId`; `/me/contacts/delta` is an
  undocumented stopgap.
- Graph `DELETE` landing folder (§4.7): **mostly answered.** Recoverable Items › Deletions, not Deleted
  Items (third-party docs plus Microsoft's description of `recoverableitemsdeletions`); smoke-test once.
- Sent copies (§5.9): Microsoft 365 SMTP AUTH **answered** (saves to Sent Items, documented).
  Gmail **unverified first-party** (consistent community evidence). Outlook.com **open** (keep `detect`).
- Prefer header combination (§4.1): **open**; smoke test via `Preference-Applied`.
- `$expand` cast path and `meetingTenativelyAccepted` spelling (§4.7): **answered**, both as published.
- Shared-mailbox scopes for personal accounts (§3.1): **answered** — not supported.
- Exchange Online IMAP capabilities (§5.3): **partly answered** from third-party logs (no CONDSTORE /
  QRESYNC / SPECIAL-USE); still confirm against a 2026 tenant.
- SMTP AUTH basic-auth timeline (§5.2): **current as of MC786329 (27 Jan 2026)**; re-check near release.
- Google OAuth verification: `https://mail.google.com/` is a restricted scope for public apps
  (business/legal task); CalDAV/CardDAV endpoints now confirmed, scope strings still (unverified).
- Calendar delta on secondary calendars (§4.9): **new open item**; v1.0 reference shows the primary
  calendar only.
- On-premises Exchange Server (no Graph) support: out of v1 unless IMAP is enabled on the server.
  Product decision, not a research question. Note that EWS's announced retirement covers Exchange
  Online only, so on-prem support would mean EWS, which this spec forbids.

---

## Sources

- https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow
- https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-device-code
- https://learn.microsoft.com/en-us/entra/identity-platform/reply-url
- https://learn.microsoft.com/en-us/entra/identity-platform/scenario-desktop-app-configuration
- https://learn.microsoft.com/en-us/entra/identity-platform/reference-error-codes
- https://learn.microsoft.com/en-us/graph/permissions-reference
- https://learn.microsoft.com/en-us/graph/throttling
- https://learn.microsoft.com/en-us/graph/throttling-limits
- https://learn.microsoft.com/en-us/graph/json-batching
- https://learn.microsoft.com/en-us/graph/paging
- https://learn.microsoft.com/en-us/graph/outlook-immutable-id
- https://learn.microsoft.com/en-us/graph/delta-query-overview
- https://learn.microsoft.com/en-us/graph/delta-query-messages
- https://learn.microsoft.com/en-us/graph/delta-query-events
- https://learn.microsoft.com/en-us/graph/api/resources/mailfolder?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/mailfolder-delta?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/resources/message?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/user-list-messages?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/resources/followupflag?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/user-sendmail?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/outlook-large-attachments
- https://learn.microsoft.com/en-us/graph/search-query-parameter
- https://learn.microsoft.com/en-us/graph/api/resources/event?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/calendar-list-calendarview?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/event-accept?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/event-decline?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/resources/contact?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/resources/todotask?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/graph/api/resources/todotasklist?view=graph-rest-1.0
- https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth
- https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/deprecation-of-basic-authentication-exchange-online
- https://learn.microsoft.com/en-us/exchange/mail-flow-best-practices/how-to-set-up-a-multifunction-device-or-application-to-send-email-using-microsoft-365-or-office-365
- https://techcommunity.microsoft.com/blog/exchange/updated-exchange-online-smtp-auth-basic-authentication-deprecation-timeline/4489835
- https://support.microsoft.com/en-us/office/pop-imap-and-smtp-settings-for-outlook-com-d088b986-291d-42b8-9564-9c414e2aa040
- https://support.microsoft.com/en-us/topic/c5d65390-9676-4763-b41f-d7986499a90d
- https://insider.office.com/blog/starting-to-support-third-party-accounts-in-new-outlook-for-windows-preview (as of 2026-10-19 this redirects to the generic Microsoft 365 Insider blog index; the post itself is no longer reachable at this URL — replaced for account-type facts by the "Getting started with the new Outlook" page below)
- https://developers.google.com/workspace/gmail/imap/xoauth2-protocol
- https://developers.google.com/identity/protocols/oauth2/native-app
- https://knowledge.workspace.google.com/admin/sync/transition-from-less-secure-apps-to-oauth
- https://specifications.freedesktop.org/secret-service/latest/
- https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Secret.html
- https://github.com/microsoft/fluentui (packages/tokens/src/alias/lightColor.ts, darkColor.ts, lightColorPalette.ts, darkColorPalette.ts; global/brandColors.ts, global/colors.ts)
- https://fluent2.microsoft.design/
- OAuth/SASL RFCs: https://www.rfc-editor.org/rfc/rfc6749 · https://www.rfc-editor.org/rfc/rfc7636 · https://www.rfc-editor.org/rfc/rfc8252 · https://www.rfc-editor.org/rfc/rfc8628 · https://www.rfc-editor.org/rfc/rfc7628 · https://www.rfc-editor.org/rfc/rfc4959 · https://www.rfc-editor.org/rfc/rfc7240
- IMAP RFCs: https://www.rfc-editor.org/rfc/rfc3501 · https://www.rfc-editor.org/rfc/rfc9051 · https://www.rfc-editor.org/rfc/rfc2177 · https://www.rfc-editor.org/rfc/rfc7162 · https://www.rfc-editor.org/rfc/rfc4315 · https://www.rfc-editor.org/rfc/rfc6851 · https://www.rfc-editor.org/rfc/rfc6154 · https://www.rfc-editor.org/rfc/rfc8457 · https://www.rfc-editor.org/rfc/rfc5258 · https://www.rfc-editor.org/rfc/rfc5819 · https://www.rfc-editor.org/rfc/rfc4731 · https://www.rfc-editor.org/rfc/rfc5161 · https://www.rfc-editor.org/rfc/rfc2971 · https://www.rfc-editor.org/rfc/rfc2342 · https://www.rfc-editor.org/rfc/rfc7888 · https://www.rfc-editor.org/rfc/rfc3516 · https://www.rfc-editor.org/rfc/rfc6855 · https://www.rfc-editor.org/rfc/rfc4978 · https://www.rfc-editor.org/rfc/rfc8474 · https://www.rfc-editor.org/rfc/rfc8970 · https://www.rfc-editor.org/rfc/rfc3691 · https://www.rfc-editor.org/rfc/rfc6186
- SMTP/message RFCs: https://www.rfc-editor.org/rfc/rfc6409 · https://www.rfc-editor.org/rfc/rfc8314 · https://www.rfc-editor.org/rfc/rfc1870 · https://www.rfc-editor.org/rfc/rfc6152 · https://www.rfc-editor.org/rfc/rfc6531 · https://www.rfc-editor.org/rfc/rfc2920 · https://www.rfc-editor.org/rfc/rfc2034 · https://www.rfc-editor.org/rfc/rfc3461 · https://www.rfc-editor.org/rfc/rfc5322 · https://www.rfc-editor.org/rfc/rfc2045 · https://www.rfc-editor.org/rfc/rfc2047
- Calendar/contacts RFCs: https://www.rfc-editor.org/rfc/rfc4791 · https://www.rfc-editor.org/rfc/rfc6352 · https://www.rfc-editor.org/rfc/rfc6578 · https://www.rfc-editor.org/rfc/rfc6764 · https://www.rfc-editor.org/rfc/rfc5397 · https://www.rfc-editor.org/rfc/rfc5545 · https://www.rfc-editor.org/rfc/rfc5546 · https://www.rfc-editor.org/rfc/rfc6047 · https://www.rfc-editor.org/rfc/rfc6638 · https://www.rfc-editor.org/rfc/rfc6350

Added in the 2026-10-19 fact-check pass:

- Microsoft Graph, mail: https://learn.microsoft.com/en-us/graph/api/message-delete?view=graph-rest-1.0 · https://learn.microsoft.com/en-us/graph/api/message-permanentdelete?view=graph-rest-1.0 · https://learn.microsoft.com/en-us/graph/api/message-delta?view=graph-rest-1.0 · https://learn.microsoft.com/en-us/graph/api/mailfolder-delete?view=graph-rest-1.0 · https://learn.microsoft.com/en-us/graph/api/eventmessage-get?view=graph-rest-1.0 · https://learn.microsoft.com/en-us/graph/api/resources/eventmessage?view=graph-rest-1.0
- Microsoft Graph, calendar/contacts/tasks: https://learn.microsoft.com/en-us/graph/api/event-delta?view=graph-rest-1.0 · https://learn.microsoft.com/en-us/graph/api/contact-delta?view=graph-rest-1.0 · https://learn.microsoft.com/en-us/graph/api/resources/contactfolder?view=graph-rest-1.0
- Microsoft Q&A threads used for undocumented behaviour (community/Microsoft-staff answers, not reference docs): https://learn.microsoft.com/en-us/answers/questions/1634378/microsoft-graph-api-contacts-delta · https://learn.microsoft.com/en-us/answers/questions/854621/how-to-get-folder-id-of-default-contacts-folder-wh · https://learn.microsoft.com/en-us/answers/a/889592
- Graph permissions reference source (MSA badges; shared-permission notes): https://github.com/microsoftgraph/microsoft-graph-docs-contrib/blob/main/concepts/permissions-reference.md · https://github.com/microsoftgraph/microsoft-graph-docs-contrib/blob/main/includes/permissions-notes/mail.readwrite.shared.md · https://github.com/microsoftgraph/microsoft-graph-docs-contrib/blob/main/includes/permissions-notes/mail.send.shared.md
- Microsoft identity platform OIDC discovery (tenant/domain probe): https://learn.microsoft.com/en-us/entra/identity-platform/v2-protocols-oidc
- Exchange Online SMTP AUTH: https://learn.microsoft.com/en-us/exchange/clients-and-mobile-in-exchange-online/authenticated-client-smtp-submission · Message Center MC786329 (mirror: https://mc.merill.net/message/MC786329)
- New Outlook account entry points and supported account types: https://support.microsoft.com/en-us/office/getting-started-with-the-new-outlook-for-windows-656bb8d9-5a60-49b2-a98b-ba7822bc7627
- Google: https://developers.google.com/identity/protocols/oauth2 (refresh-token expiry) · https://developers.google.com/identity/protocols/oauth2/scopes · https://developers.google.com/workspace/calendar/caldav/v2/guide · https://developers.google.com/people/carddav · https://knowledge.workspace.google.com/admin/gmail/gmail-bandwidth-limits
- Third-party evidence (flagged (unverified) where used): Rebex GraphClient.DeleteMessage docs (Graph DELETE → Recoverable Items › Deletions) https://rebex.net/doc/api/Rebex.Net.GraphClient.DeleteMessage.html · Office 365 for IT Pros on permanentDelete https://office365itpros.com/2025/05/07/permanent-deletion-mailbox-item/ · Exchange Online IMAP capability log https://eightwone.com/2020/07/01/configuring-exchange-account-with-imap-oauth2/ · Gmail SMTP auto-saving to Sent Mail (client trackers/KB): https://msoutlook.info/?p=526 · https://bugzilla.gnome.org/show_bug.cgi?id=599230
- Fluent UI v9 component defaults: https://github.com/microsoft/fluentui (packages/tokens/src/statusColorMapping.ts, global/typographyStyles.ts, global/fonts.ts, global/borderRadius.ts; packages/react-components/react-dialog/library/src/contexts/constants.ts and DialogSurface/DialogActions styles; react-button Button styles; react-input Input styles; react-field Field styles)
- SMTP submission SRV for implicit TLS (`_submissions._tcp`): https://www.rfc-editor.org/rfc/rfc8314#section-5.1
