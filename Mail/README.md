<img src="logo.png" alt="Axentra Mail logo" width="96" align="right">

# Axentra Mail

<!-- local-release-preparation -->
## Prepared version: 0.1.2

This version is prepared locally and is awaiting approval to upload a GitHub Release draft.
The previously documented version was **unreleased (handoff build 0.1.0)**.
See [release notes, build status and SHA-256 hashes](releases/0.1.2.md) and [changelog](CHANGELOG.md).

- Source is now 0.1.2, ahead of the handoff's 0.1.0 installer. Fixed UTF-8 slicing crashes that could permanently stop sorting; per-message panic handling skips failures and records crashes in crash.log.
- Sorting status page shows progress, waiting/sorted counts, timing, recent decisions and category totals, with pause and resume.
- Resizable folder, message and reading panes remember their widths; double-click resets a divider.
- AI work no longer blocks opening, archiving or moving messages. Fixed native TLS for Microsoft HTTPS calls and made sign-in cancellation release the screen immediately.
- Per-user Inno installer includes optional desktop/startup entries and preserves the local mail copy on uninstall unless removal is selected.

Prepared `AxentraMail-0.1.2-Setup.exe` installs for the current user to `%LOCALAPPDATA%\Programs\Axentra Mail`, without administrator rights. It adds a Start menu entry plus optional desktop and Start with Windows entries. Uninstall preserves your mail cache unless you explicitly choose to remove it. A Microsoft app registration, mailbox setup and optional local Ollama model are still required; the installer does not configure those accounts.

Pending: Code signing and fresh VirusTotal scan pending.; Draft release assets remain local at Shawn's request. Pushing and publishing still require approval.
<!-- /local-release-preparation -->

An Outlook.com mail client for Windows that cleans up your inbox with AI that runs **on your own PC**. Spam and
phishing are caught and held for you. Promotions, newsletters and app notifications are filed away automatically
(always undoable). Your mail is never sent to an AI company. There's no Axentra server, account, subscription or
tracking.

![Axentra Mail](screenshots/inbox.jpg)

**Status: not released yet.** Axentra Mail is in testing. Installers will appear on this repository's
[Releases](https://github.com/MECMwithShawn/Axentra-Software/releases) page, with checksums and virus-scan results
here, once it's ready.

## Features

| Area | What you get |
|---|---|
| **Private AI sorting** | Every new message is read by a local model (Ollama, for example Qwen3 14B) on your PC and labelled: personal, work, needs action, receipt or account, security, newsletter, promotion, notification, spam or phishing, with a one-line summary |
| **Automatic filing, safely** | Low-risk mail moves to *Promotions*, *Newsletters* and *Social & alerts* folders on its own, but only with bulk-mail headers, from senders you never write to, and never anything that looks like a code, receipt, bill or account notice |
| **Spam and phishing wait for you** | *To review* shows the evidence: links whose text names one site but go to another, failed SPF/DKIM/DMARC, replies redirected to another domain. Nothing suspicious moves without your OK |
| **Undo everything** | *Activity & undo* lists every move the AI or you made, with one-click undo |
| **Sorting status** | Live queue progress, timing, recent decisions and category totals, with pause and resume. A failed message is skipped instead of stopping the worker |
| **Resizable panes** | Drag the folder, message-list and reading-pane dividers; widths are remembered and double-click resets them |
| **Cleanup** | Mail that was already in your inbox when you connected gets suggestions, grouped so one click files a whole group |
| **Needs a reply** | Real people waiting for your answer, as the AI reads it |
| **Senders** | Who fills your mailbox and how much of it you read. Set a rule per sender (always keep, always file, always junk) or unsubscribe with the sender's own link |
| **AI reply drafts** | Say what you want ("yes to Saturday, ask about 7:30") and get a draft written on your PC. Nothing is sent until you press Send |
| **Safe reading** | Email is shown without scripts in a sandbox, tracking images are blocked until you allow them, and links open only after you've seen where they really go. Saved attachments are marked as downloaded from the internet, so Windows and Office treat them with care |
| **Local search** | Fast full-text search of the mail stored on your PC |
| **Guided setup** | A step-by-step setup screen walks you through connecting your mailbox, with links to each Microsoft page, values to copy, and plain-language help for Microsoft's error codes |

## Screenshots

| | |
|---|---|
| ![Inbox](screenshots/inbox.jpg) The AI labels and summarizes each message, and flags who's waiting for a reply | ![Phishing held for review](screenshots/phishing.jpg) Phishing held for review, with the evidence |

Screenshots use the built-in demo mailbox (made-up mail).

## How your mail stays private

- Axentra Mail talks to Microsoft directly (`login.microsoftonline.com` and `graph.microsoft.com`), which already
  holds your mailbox. You sign in on Microsoft's own page in your browser, so the app never sees your password.
- It uses **your own** free Microsoft app registration, created during setup. No Axentra server sits in between.
- The AI runs in Ollama on your PC. The app refuses any AI address that isn't on your PC (`127.0.0.1`), so mail
  text can't be sent to another computer for AI work.
- Your sign-in is stored encrypted with Windows (DPAPI), readable only by your Windows account on that PC.
- Email content is treated as untrusted. The AI can only choose from fixed labels, and plain rules decide what may
  move, so an email that tries to talk the AI into something can't make it act.

## System requirements

- Windows 10 or 11, 64-bit, with the Microsoft Edge WebView2 Runtime (built into Windows 11)
- An Outlook.com, Hotmail or Live mailbox (a personal Microsoft account)
- [Ollama](https://ollama.com) with a model such as `qwen3:14b` (about 9 GB). A GPU with 12 GB or more of memory
  is recommended; mail still works without the AI, sorting just waits
- A free Microsoft Entra app registration. The setup screen walks you through it in about 5 minutes

## Your data

| What | Where |
|---|---|
| Local copy of your mail, AI labels, sender rules, undo log | `%LOCALAPPDATA%\Axentra\Mail\mail.db` |
| Sign-in (refresh token) | `%LOCALAPPDATA%\Axentra\Mail\account.bin`, encrypted with Windows DPAPI |
| Settings | `%LOCALAPPDATA%\Axentra\Mail\settings.json` |

Signing out removes the local copy of your mail. Nothing is deleted from your mailbox. Uninstalling keeps it unless
you choose to remove it.

## License

Axentra Mail is free software under the MIT license.
