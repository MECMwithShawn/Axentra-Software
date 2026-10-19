"""Axentra OneDrive integration for GNOME Files (Nautilus 43+).

Status badges like the Windows Explorer status column (online-only cloud,
available-locally check, always-available solid check, syncing, problem)
and a OneDrive submenu: Share, View online, Version history, Always keep
on this device, Free up space.

Badges come from the mount's `user.axentra.onedrive.status` extended attribute, so
showing them never blocks on the network. Actions go to the Axentra OneDrive
service over D-Bus asynchronously.
"""

import os
import shutil
import subprocess

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
            notify(error_title, e.message)
            return

        def finished(proxy, result):
            try:
                value = proxy.call_finish(result)
            except GLib.Error as e:
                Gio.DBusError.strip_remote_error(e)
                notify(error_title, e.message)
                return
            if on_done:
                on_done(value.unpack() if value is not None else ())

        self._proxy.call(method, params, Gio.DBusCallFlags.NONE, 120_000, None, finished)

    # ----- badges ------------------------------------------------------------

    def update_file_info(self, file):
        path = local_path(file)
        if not path:
            return
        status = item_status(path)
        emblem = EMBLEMS.get(status or "")
        if emblem:
            file.add_emblem(emblem)
        if status and is_shared(path):
            file.add_emblem("axentra-onedrive-shared")

    # ----- menus -------------------------------------------------------------

    def get_file_items(self, files):
        paths = [local_path(f) for f in files]
        if not paths or any(p is None for p in paths):
            return []
        statuses = [item_status(p) for p in paths]
        if any(s is None for s in statuses):
            return []

        top = Nautilus.MenuItem(name="AxentraOneDrive::OneDrive", label="Axentra OneDrive")
        menu = Nautilus.Menu()
        top.set_submenu(menu)

        def add(name, label, handler, *args):
            item = Nautilus.MenuItem(name=f"AxentraOneDrive::{name}", label=label)
            item.connect("activate", lambda _item: handler(*args))
            menu.append_item(item)

        single = len(paths) == 1
        if single:
            add("Share", "Share (copy link)", self._share, paths[0])
            add("ViewOnline", "View online", self._view_online, paths[0])
            if not os.path.isdir(paths[0]) and versions_command():
                add("Versions", "Version history", self._versions, paths[0])

        all_pinned = all(s == "pinned" for s in statuses)
        if all_pinned:
            add("Unpin", "✓ Always keep on this device", self._unpin, paths, files)
        else:
            add("Pin", "Always keep on this device", self._pin, paths, files)
        add("Free", "Free up space", self._free, paths, files)
        return [top]

    def get_background_items(self, folder):
        path = local_path(folder)
        if not path or item_status(path) is None:
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
