"""Axentra OneDrive integration for GNOME Files (Nautilus 43+).

Status badges like the Windows Explorer status column (online-only cloud,
available-locally check, always-available solid check, syncing, problem)
and a OneDrive submenu: Share, View online, Version history, Always keep
on this device, Free up space.

Badges come from the mount's `user.axentra.onedrive.status` extended attribute, so
showing them never blocks on the network, and are looked at again when the
service says an item changed (an upload or download finishing, say). Actions
go to the Axentra OneDrive service over D-Bus asynchronously.
"""

import math
import os
import shutil
import subprocess
import time

import gi

gi.require_version("Nautilus", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gdk, Gio, GLib, GObject, Nautilus  # noqa: E402

BUS_NAME = "io.github.mecmwithshawn.AxentraOneDrive"
OBJECT_PATH = "/io/github/mecmwithshawn/AxentraOneDrive"
INTERFACE = "io.github.mecmwithshawn.AxentraOneDrive1"

EMBLEMS = {
    "online": "axentra-onedrive-online",
    "local": "axentra-onedrive-local",
    "pinned": "axentra-onedrive-pinned",
    "syncing": "axentra-onedrive-syncing",
    "error": "axentra-onedrive-error",
}


def item_status(path):
    """The item's sync status, or None when it isn't in the OneDrive folder."""
    try:
        return os.getxattr(path, "user.axentra.onedrive.status").decode()
    except OSError:
        return None


def is_shared(path):
    try:
        return os.getxattr(path, "user.axentra.onedrive.shared") == b"1"
    except OSError:
        return False


def is_pinned(path):
    """Set to "Always keep on this device", even while it's still downloading."""
    try:
        return os.getxattr(path, "user.axentra.onedrive.pinned") == b"1"
    except OSError:
        return False


def on_onedrive(path):
    """Whether the item has reached OneDrive, so it can be shared or opened online."""
    try:
        os.getxattr(path, "user.axentra.onedrive.url")
        return True
    except OSError:
        return False


def emblems_for(status, shared):
    if status is None:
        return None
    emblems = [EMBLEMS[status]] if status in EMBLEMS else []
    if shared:
        emblems.append("axentra-onedrive-shared")
    return tuple(emblems)


def badges(path):
    """The emblems an item shows, or None when it isn't in the OneDrive folder."""
    status = item_status(path)
    return emblems_for(status, status is not None and is_shared(path))


# The attributes badges() reads, as GIO names them.
BADGE_ATTRIBUTES = "xattr::axentra.onedrive.status,xattr::axentra.onedrive.shared"


def badges_from(info):
    """badges() from a Gio.FileInfo with BADGE_ATTRIBUTES."""
    return emblems_for(
        info.get_attribute_string("xattr::axentra.onedrive.status"),
        info.get_attribute_string("xattr::axentra.onedrive.shared") == "1",
    )


# Badges of items the service is still working on. They're looked at again
# now and then as well, since not every end of syncing is announced.
UNSETTLED = {EMBLEMS["syncing"], EMBLEMS["error"]}
# At least this many seconds apart, and more when a look takes long.
UNSETTLED_EVERY = 5


def affected(path, changed, folders):
    """Whether the item at `path` is one of the `changed` paths or in one,
    or is in `folders`, the folders holding them (whose badge can follow
    what's in them)."""
    if not changed:
        return False
    if path in folders:
        return True
    while True:
        if path in changed:
            return True
        parent = os.path.dirname(path)
        if parent == path:
            return False
        path = parent


def local_path(file_info):
    if file_info.get_uri_scheme() != "file":
        return None
    location = file_info.get_location()
    return location.get_path() if location else None


def notify(summary, body=""):
    try:
        Gio.bus_get_sync(Gio.BusType.SESSION, None).call(
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "Notify",
            GLib.Variant("(susssasa{sv}i)", ("Axentra OneDrive", 0, "axentra-onedrive", summary, body, [], {}, -1)),
            None,
            Gio.DBusCallFlags.NONE,
            -1,
            None,
            None,
        )
    except GLib.Error:
        pass


NOT_RUNNING = ("org.freedesktop.DBus.Error.ServiceUnknown", "org.freedesktop.DBus.Error.NameHasNoOwner")


def error_text(e):
    """What the service said, without GDBus's "GDBus.Error:<name>: " in front."""
    if Gio.DBusError.get_remote_error(e) in NOT_RUNNING:
        return "Axentra OneDrive isn't running."
    if Gio.DBusError.is_remote_error(e):
        return e.message.split(": ", 1)[-1]
    return e.message


# Filled in by install.sh: the Axentra suite app folder.
APP_DIR = "@APP@"


def versions_command():
    installed = os.path.join(APP_DIR, "AxentraOneDrive")
    default = os.path.expanduser("~/.local/share/axentra/apps/onedrive/AxentraOneDrive")
    for candidate in (installed, default, shutil.which("AxentraOneDrive")):
        if candidate and os.path.exists(candidate):
            return candidate
    return None


class AxentraOneDriveExtension(GObject.GObject, Nautilus.InfoProvider, Nautilus.MenuProvider):
    def __init__(self):
        super().__init__()
        self._proxy = None
        # Badged files by path: a weak reference and the emblems shown, so a
        # status the service changes on its own reaches open windows too.
        self._badged = {}
        # What to look at next: the paths the service said changed, and
        # whether the unsettled badges are due (the service changed at all).
        self._changed_paths = set()
        self._unsettled_due = False
        self._unsettled_next = 0.0
        # The queued look (a GLib source and when it runs), or one under way.
        self._timer = None
        self._timer_at = 0.0
        self._looking = False
        try:
            bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
            for signal, handler in (("Changed", self._changed), ("ItemsChanged", self._items_changed)):
                bus.signal_subscribe(
                    BUS_NAME,
                    INTERFACE,
                    signal,
                    OBJECT_PATH,
                    None,
                    Gio.DBusSignalFlags.NONE,
                    handler,
                )
        except GLib.Error:
            pass

    # ----- D-Bus -------------------------------------------------------------

    def _call(self, method, params, on_done=None, error_title="OneDrive couldn't do that"):
        try:
            if self._proxy is None:
                self._proxy = Gio.DBusProxy.new_for_bus_sync(
                    Gio.BusType.SESSION,
                    Gio.DBusProxyFlags.DO_NOT_LOAD_PROPERTIES | Gio.DBusProxyFlags.DO_NOT_CONNECT_SIGNALS,
                    None,
                    BUS_NAME,
                    OBJECT_PATH,
                    INTERFACE,
                    None,
                )
        except GLib.Error as e:
            notify(error_title, error_text(e))
            return

        def finished(proxy, result):
            try:
                value = proxy.call_finish(result)
            except GLib.Error as e:
                notify(error_title, error_text(e))
                return
            if on_done:
                on_done(value.unpack() if value is not None else ())

        self._proxy.call(method, params, Gio.DBusCallFlags.NONE, 120_000, None, finished)

    # ----- badges ------------------------------------------------------------

    def update_file_info(self, file):
        path = local_path(file)
        if not path:
            return
        emblems = badges(path)
        if emblems is None:
            self._badged.pop(path, None)
            return
        for emblem in emblems:
            file.add_emblem(emblem)
        seen = self._badged.get(path)
        if seen and seen[0]() == file:
            seen[1] = emblems
        else:
            self._badged[path] = [file.weak_ref(), emblems]

    def _changed(self, *_signal):
        # Something changed (a sync pass, a transfer): what was still
        # syncing may be done.
        self._unsettled_due = True
        self._schedule()

    def _items_changed(self, _bus, _sender, _path, _interface, _signal, params):
        self._changed_paths.update(params.unpack()[0])
        self._schedule()

    def _schedule(self):
        # A sync pass sends a stream of changes: look at most once a second,
        # and after the look under way, which schedules the next itself.
        if self._looking:
            return
        if self._changed_paths:
            delay = 1.0
        elif self._unsettled_due:
            delay = max(1.0, self._unsettled_next - time.monotonic())
        else:
            return
        at = time.monotonic() + delay
        if self._timer is not None:
            if self._timer_at <= at:
                return
            GLib.source_remove(self._timer)
        self._timer, self._timer_at = GLib.timeout_add(math.ceil(delay * 1000), self._recheck), at

    def _recheck(self):
        self._timer = None
        self._looking = True
        changed, self._changed_paths = self._changed_paths, set()
        folders = {os.path.dirname(p) for p in changed}
        unsettled = self._unsettled_due and time.monotonic() >= self._unsettled_next
        if unsettled:
            self._unsettled_due = False
        # Only what can have changed: going through every badged file costs
        # the service a lookup each, again for every change.
        todo = []
        for path, (ref, shown) in list(self._badged.items()):
            file = ref()
            if file is None:
                del self._badged[path]
            elif not (unsettled and UNSETTLED.intersection(shown)) and not affected(path, changed, folders):
                continue
            elif local_path(file) != path:
                # Renamed or moved: it's badged again at its new path.
                del self._badged[path]
                file.invalidate_extension_info()
            else:
                todo.append((path, shown))
        self._recheck_next(todo, time.monotonic() if unsettled else None)
        return GLib.SOURCE_REMOVE

    def _recheck_next(self, todo, sweep_started):
        # One at a time, with GIO reading them off the main loop: each is a
        # lookup in the service's database, which a big change can keep busy.
        # (nautilus-python keeps the GIL while Files is idle, so a Python
        # thread wouldn't get to run.)
        if not todo:
            self._looking = False
            if sweep_started is not None:
                took = time.monotonic() - sweep_started
                self._unsettled_next = time.monotonic() + max(UNSETTLED_EVERY, 10 * took)
            self._schedule()
            return
        path, shown = todo.pop()

        def read(gfile, result):
            try:
                now = badges_from(gfile.query_info_finish(result))
            except GLib.Error:
                now = None
            if now != shown:
                self._rebadge(path, now)
            self._recheck_next(todo, sweep_started)

        Gio.File.new_for_path(path).query_info_async(
            BADGE_ATTRIBUTES, Gio.FileQueryInfoFlags.NONE, GLib.PRIORITY_LOW, None, read
        )

    def _rebadge(self, path, now):
        entry = self._badged.get(path)
        file = entry[0]() if entry else None
        # Skip it if Files has looked at it again in the meantime.
        if file is None or entry[1] == now:
            return
        if now is None:
            del self._badged[path]
        else:
            entry[1] = now
        file.invalidate_extension_info()

    # ----- menus -------------------------------------------------------------

    def get_file_items(self, files):
        paths = [local_path(f) for f in files]
        if not paths or any(p is None for p in paths):
            return []
        statuses = [item_status(p) for p in paths]
        if any(s is None for s in statuses):
            return []
        # Lock files, ignored names and the like never sync: OneDrive has
        # nothing to offer for them.
        synced = [(p, f) for p, f, s in zip(paths, files, statuses) if s != "local-only"]
        if not synced:
            return []

        top = Nautilus.MenuItem(name="AxentraOneDrive::OneDrive", label="Axentra OneDrive")
        menu = Nautilus.Menu()
        top.set_submenu(menu)

        def add(name, label, handler, *args):
            item = Nautilus.MenuItem(name=f"AxentraOneDrive::{name}", label=label)
            item.connect("activate", lambda _item: handler(*args))
            menu.append_item(item)

        single = len(paths) == 1
        if single and on_onedrive(paths[0]):
            add("Share", "Share (copy link)", self._share, paths[0])
            add("ViewOnline", "View online", self._view_online, paths[0])
            if not os.path.isdir(paths[0]) and versions_command():
                add("Versions", "Version history", self._versions, paths[0])

        paths = [p for p, _ in synced]
        files = [f for _, f in synced]
        if all(is_pinned(p) for p in paths):
            add("Unpin", "✓ Always keep on this device", self._unpin, paths, files)
        else:
            add("Pin", "Always keep on this device", self._pin, paths, files)
        add("Free", "Free up space", self._free, paths, files)
        return [top]

    def get_background_items(self, folder):
        path = local_path(folder)
        if not path or not on_onedrive(path):
            return []
        item = Nautilus.MenuItem(name="AxentraOneDrive::ViewFolderOnline", label="View online")
        item.connect("activate", lambda _i: self._view_online(path))
        return [item]

    # ----- actions -----------------------------------------------------------

    @staticmethod
    def _refresh(files):
        for f in files:
            f.invalidate_extension_info()

    def _pin(self, paths, files):
        self._call("Pin", GLib.Variant("(as)", (paths,)), lambda _r: self._refresh(files))

    def _unpin(self, paths, files):
        self._call("Unpin", GLib.Variant("(as)", (paths,)), lambda _r: self._refresh(files))

    def _free(self, paths, files):
        def done(result):
            self._refresh(files)
            kept = result[0] if result else 0
            if kept:
                notify(
                    "Some files are still on this device",
                    f"{kept} have changes that haven't uploaded yet. They'll be freed once they sync.",
                )

        self._call("FreeUpSpace", GLib.Variant("(as)", (paths,)), done)

    def _share(self, path):
        def done(result):
            link = result[0]
            display = Gdk.Display.get_default()
            if display is not None:
                provider = Gdk.ContentProvider.new_for_value(GObject.Value(GObject.TYPE_STRING, link))
                display.get_clipboard().set_content(provider)
                notify("Link copied", f"Anyone with the link can view “{os.path.basename(path)}”.")
            else:
                notify("Sharing link", link)

        self._call("ShareLink", GLib.Variant("(sb)", (path, False)), done, "Couldn't create a link")

    def _view_online(self, path):
        self._call(
            "WebUrl",
            GLib.Variant("(s)", (path,)),
            lambda result: Gio.AppInfo.launch_default_for_uri(result[0], None),
            "Couldn't open this online",
        )

    def _versions(self, path):
        command = versions_command()
        if command:
            subprocess.Popen([command, "--versions", path], start_new_session=True)
