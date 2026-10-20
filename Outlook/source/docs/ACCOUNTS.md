# Connecting accounts

Axentra Outlook talks directly to your mail provider; nothing is relayed
through a cloud service. Behaviour is specified in `docs/spec/protocols.md`;
this page explains the one-time setup a person running Axentra needs.

| Account | How Axentra connects | What you need |
|---|---|---|
| Microsoft 365 (work or school), Outlook.com, Hotmail, Live, MSN | Microsoft Graph with OAuth 2.0 (browser sign-in, or a code on another device) | An application (client) ID from a free Microsoft Entra app registration (below) |
| Gmail / Google Workspace | IMAP + SMTP with an app password, or with Google OAuth when a Google client is configured | 2-Step Verification and an app password (<https://myaccount.google.com/apppasswords>), or `AXENTRA_GOOGLE_CLIENT_ID` |
| iCloud Mail | IMAP + SMTP | An app-specific password (<https://account.apple.com/account/manage>) |
| Yahoo Mail | IMAP + SMTP | An app password (Yahoo account security page) |
| Any other IMAP provider | IMAP (993 SSL/TLS or 143 STARTTLS) + SMTP submission (465 SSL/TLS or 587 STARTTLS) | Server names, ports and your password |

Add accounts in **Settings › Accounts › Email accounts › Add account** (the
avatar at the top right opens that page).

## Microsoft accounts: register an app in Microsoft Entra

Axentra ships without a built-in client ID. Registering an app is free and
takes a few minutes; you can use any Microsoft account (a personal Outlook.com
account works) to sign in to the Entra admin center.

1. Go to <https://entra.microsoft.com> › **Identity** › **Applications** ›
   **App registrations** › **New registration**.
2. **Name**: anything, for example "Axentra Outlook (my build)".
3. **Supported account types**: *Accounts in any organizational directory and
   personal Microsoft accounts* (lets the same registration sign in to
   Microsoft 365 and Outlook.com).
4. **Redirect URI**: platform **Public client/native (mobile & desktop)**,
   value `http://localhost`. Click **Register**.
5. That one redirect URI is all Axentra needs: it listens on a random local
   port and sends Microsoft `http://localhost:<port>`, and Microsoft ignores
   the port for `localhost` redirects. Don't register port-specific URIs,
   `http://127.0.0.1` alone (personal-account sign-in rejects it) or URIs with
   a query string.
6. **Authentication** › **Advanced settings** › **Allow public client flows**:
   **Yes** (needed for "Use a code instead"). Don't create a client secret.
7. **API permissions** › **Add a permission** › **Microsoft Graph** ›
   **Delegated permissions**, add:
   `User.Read`, `Mail.ReadWrite`, `Mail.Send`, `Calendars.ReadWrite`,
   `Contacts.ReadWrite`, `Tasks.ReadWrite`, `offline_access`, `openid`,
   `profile`. No admin consent is needed for these in most tenants; if your
   organization restricts user consent, Axentra shows "Your organization needs
   to approve Axentra Outlook before you can sign in." and an administrator has
   to grant consent once.
   Optional, only for the IMAP fallback of mailboxes that Graph can't reach:
   `IMAP.AccessAsUser.All` and `SMTP.Send` (also listed under Microsoft Graph).
8. Copy the **Application (client) ID** from the **Overview** page.

Give the ID to Axentra in either way:

* set it for every account: `export AXENTRA_MS_CLIENT_ID=<application id>`
  before starting `axentra-outlook`; or
* paste it in the Add account dialog under **Sign in with Microsoft ›
  Advanced › Application (client) ID**. It is saved with the account (it is
  not a secret).

After a successful Microsoft sign-in Axentra remembers the ID in
`~/.config/axentra-outlook/microsoft-client-id` (shared by every profile of
your user, including other `AXENTRA_DB` databases) and prefills it next time.
The order is: the ID typed in the dialog, then `AXENTRA_MS_CLIENT_ID`, then
the remembered one. Delete the file to forget it.

### Signing in

* **Open browser** opens your default browser (`xdg-open`) on Microsoft's
  sign-in page. When you finish, the page says "You're signed in. You can close
  this tab and return to Axentra Outlook." **Copy sign-in link** puts the same
  link on the clipboard if the wrong browser opened.
* **Use a code instead** shows a short code and a web address to visit on any
  device (useful over SSH or in a locked-down sandbox). The code expires after
  about 15 minutes.
* If you sign in as a different account than the address you typed, Axentra
  asks whether to add that account instead.

### What Axentra syncs

| | Microsoft 365 / Outlook.com (Graph) | IMAP accounts |
|---|---|---|
| Mail | Folders and messages by delta query (immutable ids), within the "days to sync" window (30 days by default) | Every selectable folder, UID-based, within the same window; `BODY.PEEK` so syncing never marks mail read; CONDSTORE when offered; IDLE on Inbox, otherwise every 2 minutes |
| Changes made here | Read/unread, flag (with due date), complete, categories, Focused/Other, move, delete | Read/unread, flag, complete (`$Completed` keyword), move (`MOVE`, or COPY + delete), delete |
| Folders made here | Create, rename, move, delete | Create, rename (with subfolders), move, delete |
| Sending | `sendMail` with the MIME message; Microsoft files the Sent Items copy | SMTP submission; the Sent copy is appended unless the server files one itself (Gmail, Microsoft 365; detected for others) |
| Calendar | Calendars and events 90 days back to a year ahead: create, edit, delete, accept / tentative / decline, deleted occurrences | — |
| People, To Do | Contacts (default folder); To Do lists, tasks and steps | — |

Drafts sent from here that came from the server are deleted there (the real
Sent copy is matched by Message-ID). Changes made while offline are pushed on
the next sync.

## Google accounts with OAuth (optional)

Without configuration, Gmail is added with an app password over IMAP/SMTP.
To sign in with Google instead, create an OAuth client of type **Desktop app**
in the Google Cloud console (APIs & Services › Credentials), add the
`https://mail.google.com/` scope to the consent screen, and start Axentra
with:

```sh
export AXENTRA_GOOGLE_CLIENT_ID=<client id>
export AXENTRA_GOOGLE_CLIENT_SECRET=<client secret>   # Google issues one; it is not confidential for desktop apps
```

While the consent screen is in "Testing", Google expires refresh tokens after
7 days, so you'll be asked to sign in again weekly.

## Where credentials are stored

Passwords and OAuth refresh tokens are stored only in the desktop keyring via
the freedesktop Secret Service (GNOME Keyring, KWallet 5.97+, KeePassXC with
Secret Service enabled), under the schema `app.axentra.Outlook.Credential`.
Account settings (servers, ports, user names, tenant and client IDs) are kept
in the local database; they contain no secrets. Access tokens live in memory
only.

If no keyring is running, adding an account fails with "Axentra Outlook can't
find a keyring to store your sign-in securely…" — Axentra never falls back to
storing passwords in plain text. If the keyring is locked, the account shows
"Your keyring is locked. Unlock it to sync {email}." with an **Unlock** button
that asks the keyring to prompt for its password.

Removing an account (Settings › Accounts › Manage › Remove) deletes its
keyring items and every local copy of its mail, events, contacts and tasks.
Nothing is deleted on the server. For Google OAuth accounts the refresh token
is also revoked.

## Troubleshooting

| Message | What to do |
|---|---|
| "Microsoft sign-in isn't set up yet…" | Register an app (above) and set `AXENTRA_MS_CLIENT_ID` or paste the ID under Advanced |
| "We couldn't sign in. Check your password. Some providers need an app password." | Gmail, iCloud and Yahoo need an app password, not your normal password |
| "The server's security certificate isn't trusted." | The certificate didn't validate for that host name against the system trust store (Mozilla's root list when the system has none). Open **Details** to see the issuer and SHA-256 fingerprint. Axentra has no "connect anyway" |
| "We couldn't connect to {host}…" | Check the server name and port and the security setting (993/465 use SSL/TLS, 143/587 use STARTTLS) |
| "Sending is turned off for this account by your organization." | Microsoft 365 tenants often disable SMTP AUTH; use the Microsoft (Graph) sign-in instead of IMAP |
| "We couldn't find a mailbox for {email} in Microsoft 365." | The work account has no Exchange Online mailbox; add it as an IMAP account if your organization offers IMAP |

Developer notes: `cargo test -p axentra-sync` runs the protocol tests
(MIME parsing and building, threading, SPECIAL-USE role mapping, flag diffs,
Graph JSON mapping, PKCE, Retry-After, OAuth callbacks) and an IMAP round
trip against an in-process fake server. No test touches the network.
