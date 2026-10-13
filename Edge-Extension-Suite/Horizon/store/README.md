# Microsoft Edge Add-ons store: submission kit

Everything needed to publish Axentra Horizon on the
[Microsoft Edge Add-ons store](https://microsoftedge.microsoft.com/addons). Status: **not yet submitted**.

## Why publish on the store

Installed unpacked (Developer mode), Edge treats Horizon as untrusted:

- At startup Edge shows **“Turn off extensions in developer mode”**; choosing it disables Horizon.
- After updates Edge may ask **“Change back to Microsoft's new tab page?”**; choosing it removes Horizon's new tab.
- Moving or renaming the unpacked folder changes the extension's ID, so Edge treats it as a different extension
  with empty storage.

Store installs are signed by Microsoft, never flagged, update automatically and keep their data.

## Steps

1. Register for the Edge extensions program in [Microsoft Partner Center](https://partner.microsoft.com/dashboard/microsoftedge/overview)
   (free) with the Axentra Microsoft account.
2. **Create new extension** → upload the release zip (`AxentraHorizon-<version>.zip`; `manifest.json` is at its root).
3. Fill in the listing from [listing.md](listing.md), the logo from `logo-300.png` and the screenshots from
   [`../screenshots/`](../screenshots/) (1280 × 800).
4. Privacy policy URL:
   `https://github.com/MECMwithShawn/Axentra-Software/blob/main/Edge-Extension-Suite/Horizon/PRIVACY.md`
5. Paste the permission justifications from [listing.md](listing.md#permission-justifications) into the
   certification notes, then submit. Review usually takes a few business days.

## After it's approved

- Add the store link to the Horizon README (Download section) and the suite README.
- Existing unpacked users: **Settings → Backup → Export** in the unpacked copy, install from the store, remove the
  unpacked copy, then **Import** the backup. Synced data (settings, links, reminders, habits) also returns on its
  own through the browser account.
- Future releases: upload each new zip as an update in Partner Center; users update automatically.

## Files

| File | Use |
|---|---|
| `logo-300.png` | Store logo (300 × 300) |
| [listing.md](listing.md) | Name, summary, description, search terms, category, permission justifications |
| [`../screenshots/`](../screenshots/) | Store screenshots (1280 × 800) |
