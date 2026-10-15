# Privacy: Axentra Horizon

*Prepared update: 14 October 2026 (0.6.0; unpublished)*

Axentra Horizon is a new tab extension with no Axentra account or analytics. Optional music playback contacts media providers, artwork hosts and audio sources; the built-in pinned-song fallback can contact an owner-specific server. These requests expose the normal network information any request sends to its destination.

## What Horizon stores, and where

| Data | Where it is kept |
|---|---|
| Settings (including an ESV API key and the Mail button's address if you set them), quick links, today's focus, reminders (birthdays, events), habits and check-ins, Board layouts | Your browser's sync storage. It syncs to your other devices through your Microsoft or Google account if browser sync is on |
| Todos, notes, tab stashes, photo favorites and history, quotes you add, floating windows, the cached background image and verse, weather cache | Your browser's local storage on this computer |
| Photos you upload | Your browser's IndexedDB on this computer |

If you turn on automatic backups, Horizon writes a daily backup file of all of the above (including your uploaded
photos) to the folder you choose, such as your OneDrive or Google Drive folder; that drive's own app uploads it under
its own privacy terms. Horizon never uploads anything itself. You can also export a file manually (*Settings → Backup &
restore*), and *Erase all data* deletes everything Horizon stores in the browser (backup files you saved stay where
they are). Removing the extension deletes its local data.

## What Horizon sends, and to whom

Horizon contacts these services directly from your browser, only to show what you see on the page:

| Service | Why | What it receives |
|---|---|---|
| Bing (`www.bing.com`) and Microsoft's Spotlight service (`fd.api.iris.microsoft.com`, `res.public.onecdn.static.microsoft`) | Bing daily image and Windows Spotlight backgrounds | Your language/region setting (e.g. `en-US`) |
| Lorem Picsum (`picsum.photos` and its image server `fastly.picsum.photos`) | Unsplash backgrounds | Nothing beyond a normal image request |
| Open-Meteo (`api.open-meteo.com`, `geocoding-api.open-meteo.com`) | Weather and city search | Your location's coordinates rounded to about 100 m, or the city name you type |
| GeoJS (`get.geojs.io`) | Approximate location when device location is unavailable and you haven't chosen a city | Your IP address (as with any web request) |
| BigDataCloud (`api.bigdatacloud.net`) | The place name for your device location | Your location's coordinates |
| Sites you add as links | If you leave the name blank, one request to read the page's title; for servers on your network (e.g. `localhost`, `nas.local`), checking whether they're up | A normal page request |
| bible-api.com | The daily verse in WEB, ASV or BBE (KJV is built in and needs no request) | The verse reference, e.g. `John 3:16` |
| Crossway (`api.esv.org`) | The daily verse in ESV, only if you enter your own ESV API key | The verse reference and your key |
| Sites you open in floating windows or on the Board | Showing them | Whatever any visit to that site sends; you're signed in as in a normal tab |

Every request goes straight from your browser to that service; Horizon adds no identifiers. Each service's own
privacy policy applies to the request. Choosing a city in the weather panel means no location lookups are made.

## Permissions

Horizon checks the addresses of your open tabs to find its own Board tab (so sites are unblocked only inside
Horizon's pages) and to add the favorites bar to websites that are already open. It saves tab titles and addresses
only when you stash them; tab information is never stored otherwise, and Horizon never sends it anywhere (stashes
are included in backup files if you turn on backups, see above). It reads your favorites/bookmarks to show your favorites bar,
and changes them when you edit, delete or add a favorite, or approve a favorites-organizer plan. Organizer changes support undo. The favorites bar for websites is on by default: Horizon adds the bar to the
pages you open (it moves the page down or slides over it) and does not read, collect or send anything from those
pages. Turn it off in *Settings → Bookmarks & links*, or for one site from the bar's ⋯ menu. It uses your location only for weather, and only if you haven't chosen a
city.

With *Load almost any site* on (Settings → Board), Horizon removes the frame-blocking headers (`X-Frame-Options`,
`Content-Security-Policy`) from sites loaded inside its own pages only: the Board tab and floating windows on the new
tab. Whatever that setting, YouTube players inside those pages are sent this repository's address as the referrer,
which YouTube requires; nothing about you is included. Pages you open in normal tabs are never changed.

Windows and Board tiles give the sites inside them only what media players need (autoplay, protected media, full
screen, picture-in-picture, copying to the clipboard). They can't ask for your camera, microphone, location, screen or
clipboard contents through Horizon.

Horizon hands a site's address to the Axentra Spaces app on your PC (through an `axentra-spaces://` link that Edge asks
you to allow the first time) when *Open new windows in Axentra Spaces* is on, when you use the Window panel's *Open in
Axentra Spaces* link, when you open Outlook, Gmail, Google Calendar or Teams from the Window panel (these always go to Spaces there,
because they can't run inside a page), and when you click an *Open in Axentra Spaces* button. Nothing leaves your computer. See the [permissions table](README.md#permissions).

## Prepared 0.6.0 additions

- **Most-visited sites:** the `topSites` permission reads the browser's list locally for common-site shortcuts. The list is not uploaded by Horizon.
- **Music:** YouTube/YouTube Music, Spotify, artwork hosts (including YouTube thumbnails), user-supplied audio URLs and the built-in owner-server audio fallback receive normal media/page requests when used. The source currently identifies that fallback host as `18.216.130.112`; its suitability for a public release is pending review.
- **Local audio:** imported music files and downloaded copies are held in IndexedDB on this computer. The offscreen document keeps playback running after a new tab closes; media-provider terms and browser playback restrictions still apply.
- **Favorites organizer:** proposals use bookmark titles, URLs and folders locally; applying a plan requires approval. Organizer undo data is not reused when importing a backup from another computer.

## Contact

Questions: open an issue at <https://github.com/MECMwithShawn/Axentra-Software/issues>.
