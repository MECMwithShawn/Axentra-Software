# Axentra Edge Extension Suite

Browser extensions by Axentra for Microsoft Edge (they also run in Google Chrome). Each extension has its own
folder with its documentation, screenshots, release notes and security scan results. Extension packages
(`.zip`) are attached to this repository's [Releases](https://github.com/MECMwithShawn/Axentra-Software/releases),
not stored in the folders.

| Extension | What it is | Latest | Browsers |
|---|---|---|---|
| [Axentra Horizon](Horizon/) | A calm new tab: Bing and Windows Spotlight backgrounds, focus, todos, notes, habits, birthday and date reminders, tab stash, a favorites bar for every website, focus timer and soundscapes | 0.3.1 | Edge, Chrome (120+) |

## Installing an extension from a release

Until an extension is published on the Microsoft Edge Add-ons store, install it unpacked:

1. Download the extension's `.zip` from its release and check the SHA-256 (see the extension's
   `releases/` notes): `Get-FileHash .\AxentraHorizon-0.3.1.zip -Algorithm SHA256` (PowerShell).
2. Extract it to a folder you'll keep (the browser loads it from there).
3. Open `edge://extensions`, turn on **Developer mode**, click **Load unpacked** and choose the folder.

Edge may show a "Turn off extensions in developer mode" reminder on start-up for unpacked extensions; that's
expected until the store version is available.

## Folder layout

```
Edge-Extension-Suite/
  <Extension>/
    README.md        overview, features, permissions, installation
    PRIVACY.md       what data the extension stores and which servers it contacts
    screenshots/     current screenshots (1280 x 800, the Edge Add-ons store size)
    releases/        release notes and checksums for each version
    security/        antivirus (VirusTotal) results
```
