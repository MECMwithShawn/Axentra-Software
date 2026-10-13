# Privacy: Axentra Horizon

*Last updated: 13 October 2026 (0.3.10)*

Axentra Horizon is a new tab page extension. It has no servers of its own, no accounts, no analytics, no ads
and no tracking. Axentra never receives any of your data.

## What Horizon stores, and where

| Data | Where it is kept |
|---|---|
| Settings (including an ESV API key if you enter one), quick links, today's focus, reminders (birthdays, events), habits and check-ins, Board layouts | Your browser's sync storage. It syncs to your other devices through your Microsoft or Google account if browser sync is on |
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
| Lorem Picsum (`picsum.photos`) | Unsplash backgrounds | Nothing beyond a normal image request |
| Open-Meteo (`api.open-meteo.com`, `geocoding-api.open-meteo.com`) | Weather and city search | Your location's coordinates rounded to about 100 m, or the city name you type |
| GeoJS (`get.geojs.io`) | Approximate location when device location is unavailable and you haven't chosen a city | Your IP address (as with any web request) |
| BigDataCloud (`api.bigdatacloud.net`) | The place name for your device location | Your location's coordinates |
| Servers you add as links (e.g. `localhost`, `nas.local`) | Showing whether they're up and their page title | A normal page request |
| bible-api.com | The daily verse in WEB, ASV or BBE (KJV is built in and needs no request) | The verse reference, e.g. `John 3:16` |
| Crossway (`api.esv.org`) | The daily verse in ESV, only if you enter your own ESV API key | The verse reference and your key |
| Sites you open in floating windows or on the Board | Showing them | Whatever any visit to that site sends; you're signed in as in a normal tab |

Every request goes straight from your browser to that service; Horizon adds no identifiers. Each service's own
privacy policy applies to the request. Choosing a city in the weather panel means no location lookups are made.

## Permissions

Horizon reads your tabs only when you stash them. It reads your favorites/bookmarks to show your favorites bar,
and changes them only when you edit, delete or add a favorite from the bar. The favorites bar for websites is on by default: Horizon adds the bar to the
pages you open (it moves the page down or slides over it) and does not read, collect or send anything from those
pages. Turn it off in *Settings → Bookmarks & links*, or for one site from the bar's ⋯ menu. It uses your location only for weather, and only if you haven't chosen a
city.

With *Load almost any site* on (Settings → Board), Horizon removes the frame-blocking headers (`X-Frame-Options`,
`Content-Security-Policy`) from sites loaded inside its own pages only: the Board tab and floating windows on the new
tab. For YouTube players there it sends this repository's address as the referrer, which YouTube requires. Pages you
open in normal tabs are never changed. Turning on *Open new windows in Axentra Spaces* hands the address you choose
to the Axentra Spaces app on your PC; nothing leaves your computer. See the [permissions table](README.md#permissions).

## Contact

Questions: open an issue at <https://github.com/MECMwithShawn/Axentra-Software/issues>.
