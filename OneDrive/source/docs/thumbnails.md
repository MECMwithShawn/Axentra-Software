# Cloud thumbnails

Windows shows thumbnails of online-only photos without downloading them, and so does Axentra OneDrive. It writes OneDrive's own previews of photos and videos into the [freedesktop thumbnail cache](https://specifications.freedesktop.org/thumbnail-spec/latest/), where GIO, GTK file choosers, GNOME Files and Eye of GNOME look before reading a file.

## How it works

* **Where:** `$XDG_CACHE_HOME/thumbnails/{normal,large,x-large,xx-large}/<md5 of the file:// URI>.png` (128, 256, 512 and 1024 px on the longest side). Each PNG carries `Thumb::URI`, `Thumb::MTime` and `Thumb::Size` matching what `stat` reports through the mount, so GIO marks it `thumbnail::is-valid`.
* **When:** lazily, and in the background.
  * When a file manager, image viewer or GTK file chooser lists a folder to show it (GLib's `pool-*` worker threads, Nautilus, Eye of GNOME and a few others), the folder's online-only photos and videos are queued: `normal` and `large` from one download each, three at a time, with the most recently listed folder first. Scripts, `find`, backup tools and indexers walk whole trees, so their listings fetch nothing. Listing the same folder again within a minute doesn't queue it again.
  * Listings a file manager makes only to count a folder's items (GNOME Files' "N items" for each folder in view, the total in Properties, and the OneDrive folder's own count in your home folder) fetch nothing. A folder counts as shown when it is opened, even while the others are still being counted. One opened straight after the folder it's in, before it was counted, can't be told from a count, and neither can the OneDrive folder itself; those are queued as soon as a thumbnailer or the file chooser asks for one of their photos. A file manager doesn't ask again for a photo whose thumbnailer was turned away (while offline, or before OneDrive had a preview), so whenever the OneDrive folder is listed, its photos like that are queued.
  * When a thumbnailer (such as `gdk-pixbuf-thumbnailer -s 256`) opens an online-only photo, OneDrive's thumbnail is saved at the size it was asked for and the thumbnailer is turned away. GNOME Files looks for the thumbnail again when a thumbnailer gives up, and finds it.
  * When the desktop portal's file chooser (`pool-xdg-deskto`) opens an online-only photo, it is turned away too once a thumbnail is saved. Files it reads that can't have a thumbnail still download.
  * After a thumbnail is saved, the service sets the `user.axentra.onedrive.thumbnail` attribute on the file through the mount (the value means nothing). The resulting change notification (inotify `IN_ATTRIB`, GIO `ATTRIBUTE_CHANGED`) makes open windows look at the file again and show the thumbnail.
* **Never** for files that are on this device or set to *Always keep on this device*: the system's thumbnailers can read those.
* **Removed with the item:** when it's deleted, renamed or moved (the path is in the name), or its content changes, here or online. Unlinking this PC, moving the OneDrive folder or `./uninstall.sh --purge` removes them all.
* Every download that does happen is still logged with the program that asked for it: `journalctl --user -u axentra-onedrive | grep "on demand"`.

Turn it off with `cloud_thumbnails = false` in `~/.config/axentra/onedrive/config.toml`.

## D-Bus: `GetThumbnail`

For programs that make their own thumbnails, such as the Zorin "File Explorer", so they don't read (and so download) online-only files.

| | |
|---|---|
| Bus | session |
| Name | `io.github.mecmwithshawn.AxentraOneDrive` |
| Object | `/io/github/mecmwithshawn/AxentraOneDrive` |
| Interface | `io.github.mecmwithshawn.AxentraOneDrive1` |
| Method | `GetThumbnail(s path, s size) → (s thumbnail)` |

**Arguments**

* `path`: absolute path of a file inside the OneDrive folder, as it appears on disk (for example `/home/you/OneDrive/Pictures/IMG_0001.JPG`).
* `size`: `normal` (128 px), `large` (256), `x-large` (512), `xx-large` (1024), or a pixel count such as `"300"` (rounded up to the next of those). `""` means `large`.

**Returns** the absolute path of a PNG in the freedesktop cache (see above), or `""` when there is no thumbnail:

* the file isn't a photo or video, or OneDrive has no preview of it (yet: see below);
* the file is on this device (`user.axentra.onedrive.status` is `local`, `pinned`, `syncing` or `local-only`) and no thumbnailer has made one yet. Read the local file yourself; that doesn't download anything.

The file itself is never downloaded. For an online-only file the first call fetches OneDrive's preview (typically 100–500 ms); later calls return the same PNG straight away until the file changes. OneDrive makes previews a while after a file is uploaded, so when it has none, it is asked again 2 minutes later, then less and less often while it still has none, up to once a day (for a video OneDrive can't preview, say). A file made just now is asked about again within minutes all along. Until then, calls return `""` straight away, even after the service restarts. Don't remember a `""` answer for good: ask again later, for example the next time the folder is shown. The image keeps its aspect ratio and is never scaled up. Read the PNG right away or copy it: the service deletes it when the file is deleted, renamed, moved or changed, and GNOME prunes old thumbnails.

**Errors**

| Error | When |
|---|---|
| `org.freedesktop.DBus.Error.InvalidArgs` | the path isn't in the OneDrive folder, or the size is unknown |
| `org.freedesktop.DBus.Error.FileNotFound` | no such file in OneDrive |
| `org.freedesktop.DBus.Error.Failed` | OneDrive couldn't be reached or returned an error |
| `org.freedesktop.DBus.Error.TimedOut` | no answer from OneDrive within 20 seconds |
| `org.freedesktop.DBus.Error.ServiceUnknown` / `NameHasNoOwner` | Axentra OneDrive isn't running: treat as no thumbnail |

**Calling it**

* Call asynchronously, with a D-Bus timeout of at least 25 s. Up to six calls are fetched at once and the rest wait, so ask for what's on screen first.
* To know whether a file is online-only without opening it: `os.getxattr(path, "user.axentra.onedrive.status") == b"online"`. Never `open()` an online-only file to make a thumbnail; that downloads it.
* To refresh when a thumbnail arrives on its own (for example from a folder listing by another app), watch the folder: the file gets an `ATTRIBUTE_CHANGED` event.

```sh
gdbus call --session --dest io.github.mecmwithshawn.AxentraOneDrive \
  --object-path /io/github/mecmwithshawn/AxentraOneDrive \
  --method io.github.mecmwithshawn.AxentraOneDrive1.GetThumbnail \
  "$HOME/OneDrive/Pictures/IMG_0001.JPG" large
# ('/home/you/.cache/thumbnails/large/0e3c….png',)

axentra-onedrive thumbnail ~/OneDrive/Pictures/IMG_0001.JPG --size large
```

```python
from gi.repository import Gio, GLib

BUS = ("io.github.mecmwithshawn.AxentraOneDrive",
       "/io/github/mecmwithshawn/AxentraOneDrive",
       "io.github.mecmwithshawn.AxentraOneDrive1")

def get_thumbnail(path, size, done):
    """Calls done(png_path_or_None) on the main loop."""
    def finished(bus, result):
        try:
            (png,) = bus.call_finish(result).unpack()
        except GLib.Error:
            png = ""  # not running, offline, not in OneDrive...
        done(png or None)
    Gio.bus_get_sync(Gio.BusType.SESSION).call(
        *BUS, "GetThumbnail", GLib.Variant("(ss)", (path, size)),
        GLib.VariantType("(s)"), Gio.DBusCallFlags.NONE, 25000, None, finished)
```
