<img src="logo.png" alt="Axentra Mail logo" width="96" align="right">

# Axentra Mail

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
