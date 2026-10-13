# Store listing: Axentra Horizon

## Name

Axentra Horizon

## Short description (max 250 characters)

A calm new tab: Bing and Windows Spotlight backgrounds, your favorites bar on every site, todos, notes, habits,
birthday reminders, a tab stash, focus timer and soundscapes. No account, no ads, no tracking.

## Description

Axentra Horizon turns every new tab into a calm, focused start page, and keeps your favorites one click away
on every website.

**Beautiful backgrounds.** Bing's daily image in 4K, Windows Spotlight, or Unsplash photography. Favorite the ones
you love, upload your own, or use a gradient. Location and photographer are credited on screen.

**Your day at a glance.** A large clock, a greeting, today's main focus, a daily quote, world clocks and a 6-day
weather forecast.

**Never miss a date.** Birthdays (“Mom turns 60”), anniversaries, bills and events, repeating yearly, monthly or
weekly, with desktop notifications on the day or up to a month ahead. Import your calendar from an .ics file.

**Build habits.** Daily or chosen weekdays, streaks, completion rates, a heatmap and a gentle daily reminder.

**Favorites everywhere.** Your Edge favorites bar on the new tab and on every website, with folders, in a
see-through glass or solid style. Choose per site, or press Alt+Shift+F to show or hide it.

**Tame your tabs.** Stash a window's tabs to clear your tab bar and restore them later, from the new tab, the
toolbar button, Alt+Shift+S or a right-click.

**Get things done.** Todo lists, notes, a focus timer with notifications, breathing exercises, zen mode and
soundscapes (rain, ocean, fireplace and more), all built in.

**Private by design.** No account and no sign-in. No servers, analytics or ads. Your settings sync through the
Microsoft account you're signed into Edge with; everything else stays on your computer.

## Category

Productivity

## Search terms

new tab, start page, dashboard, Bing wallpaper, Windows Spotlight, favorites bar, bookmarks bar, todo, habit
tracker, birthday reminder, tab manager, pomodoro

## Permission justifications

Paste into the certification notes.

- **Read and change all your data on all websites (`http://*/*`, `https://*/*`):** used only to draw the user's
  favorites bar at the top of web pages (favbar.js, a closed shadow root). The script does not read page content,
  collect data or make network requests. Users can turn it off globally or per site.
- **Bookmarks:** read-only, to display the favorites bar. Horizon never modifies bookmarks.
- **Tabs:** to save (stash) and reopen the user's tabs on request.
- **Scripting:** to register the favorites bar script on websites, and to remove it when the user turns it off.
- **Alarms, notifications:** reminders, habit reminders and the focus timer.
- **Geolocation:** local weather; users can choose a city instead.
- **Storage, unlimited storage:** settings, todos, notes, habits, reminders and the cached background image.
- **Search:** to search with the browser's default search engine.
- **Context menus:** the “Stash this tab” item.
- **Favicon:** site icons beside favorites and links.
- **Host access to bing.com, fd.api.iris.microsoft.com, res.public.onecdn.static.microsoft, picsum.photos,
  open-meteo.com, get.geojs.io, bigdatacloud.net:** backgrounds, weather and place names.
- **localhost, 127.0.0.1, file:///:** opening and checking the user's own locally hosted pages.

No remote code is loaded; all JavaScript ships in the package and is unminified.
