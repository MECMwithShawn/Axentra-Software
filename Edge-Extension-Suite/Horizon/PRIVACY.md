# Privacy: Axentra Horizon

*Last updated: 13 October 2026*

Axentra Horizon is a new tab page extension. It has no servers of its own, no accounts, no analytics, no ads
and no tracking. Axentra never receives any of your data.

## What Horizon stores, and where

| Data | Where it is kept |
|---|---|
| Settings, quick links, today's focus, reminders (birthdays, events), habits and check-ins | Your browser's sync storage. It syncs to your other devices through your Microsoft or Google account if browser sync is on |
| Todos, notes, tab stashes, photo favorites and history, quotes you add, the cached background image, weather cache | Your browser's local storage on this computer |
| Photos you upload | Your browser's IndexedDB on this computer |

You can export all of it (*Settings → Backup → Export*), and *Erase all data* deletes it. Removing the
extension deletes its local data.

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

Every request goes straight from your browser to that service; Horizon adds no identifiers. Each service's own
privacy policy applies to the request. Choosing a city in the weather panel means no location lookups are made.

## Permissions

Horizon reads your tabs only when you stash them, and reads your favorites/bookmarks only to show your
favorites bar; it never changes them. If you turn on the favorites bar for websites, Horizon adds the bar to the
pages you open (it moves the page down or slides over it); it does not read, collect or send anything from those
pages. It uses your location only for weather, and only if you haven't chosen a
city. See the [permissions table](README.md#permissions).

## Contact

Questions: open an issue at <https://github.com/MECMwithShawn/Axentra-Software/issues>.
