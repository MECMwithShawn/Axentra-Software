#!/usr/bin/env bash
# Install Axentra OneDrive for the current user (no sudo), in the Axentra suite
# layout: the app in ~/.local/share/axentra/apps/onedrive, plus icons, the
# Nautilus extension, systemd user services for sync and the tray icon.
# An earlier install under the project's working name, Cirrus, is upgraded in
# place: its sign-in, sync state and downloaded files are kept.
set -euo pipefail
cd "$(dirname "$0")"

CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}"
BIN="${BIN:-$HOME/.local/bin}"
APP="$DATA/axentra/apps/onedrive"
CARGO="${CARGO:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}"

[ -x "$CARGO" ] || { echo "cargo wasn't found. Install Rust from https://rustup.rs" >&2; exit 1; }
command -v fusermount3 >/dev/null || { echo "Axentra OneDrive needs FUSE 3: sudo apt install fuse3" >&2; exit 1; }

echo "Building Axentra OneDrive (release)..."
"$CARGO" build --release --bin axentra-onedrive --bin AxentraOneDrive
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"

# ----- upgrade from Cirrus ------------------------------------------------------
upgraded=""
if [ -f "$CONFIG/systemd/user/cirrus.service" ] || [ -d "$CONFIG/cirrus" ]; then
  echo "Upgrading the earlier Cirrus install..."
  pkill -x cirrus-ui 2>/dev/null || true
  systemctl --user disable --now cirrus.service 2>/dev/null || true
  rm -f "$CONFIG/systemd/user/cirrus.service" "$CONFIG/autostart/cirrus-ui.desktop" \
    "$DATA/applications/cirrus.desktop" "$DATA/nautilus-python/extensions/cirrus_nautilus.py" \
    "$DATA/icons/hicolor/scalable/apps/cirrus.svg" "$DATA"/icons/hicolor/scalable/emblems/cirrus-*.svg \
    "$BIN/cirrus" "$BIN/cirrus-ui"
  rm -rf "$DATA/doc/cirrus"
  move() {
    if [ -e "$1" ] && [ ! -e "$2" ]; then
      mkdir -p "$(dirname "$2")"
      mv "$1" "$2"
    fi
  }
  move "$CONFIG/cirrus" "$CONFIG/axentra/onedrive"
  move "$DATA/cirrus" "$DATA/axentra/onedrive"
  move "$CACHE/cirrus" "$CACHE/axentra/onedrive"
  upgraded=1
fi

# ----- the app -------------------------------------------------------------------
# Stop the tray app while its files are replaced (the service, or a copy
# started by hand or by an older install).
systemctl --user stop axentra-onedrive-tray.service 2>/dev/null || true
pkill -x AxentraOneDrive 2>/dev/null || true
install -Dm755 target/release/AxentraOneDrive "$APP/AxentraOneDrive"
install -Dm755 target/release/axentra-onedrive "$APP/axentra-onedrive"
install -Dm644 README.md "$APP/README.md"
rm -f "$APP/axentra-onedrive.svg"
for colour in ice periwinkle azure; do
  for f in extras/icons/apps/$colour/*.png; do install -Dm644 "$f" "$APP/icons/$colour/$(basename "$f")"; done
done
install -Dm644 extras/icons/apps/ice/axentra-onedrive-512.png "$APP/axentra-onedrive.png"
printf '%s\n' "$version" > "$APP/VERSION"
mkdir -p "$BIN"
ln -sf "$APP/axentra-onedrive" "$BIN/axentra-onedrive"

# ----- desktop integration ----------------------------------------------------------
fill() { sed "s|@APP@|$APP|g" "$1" > "$2"; }
# The app icon in the colour chosen in Settings > Appearance (ice by default).
colour="$(sed -n 's/^icon_color = "\(ice\|periwinkle\|azure\)"$/\1/p' "$CONFIG/axentra/onedrive/config.toml" 2>/dev/null || true)"
colour="${colour:-ice}"
rm -f "$DATA/icons/hicolor/scalable/apps/axentra-onedrive.svg"
for n in 16 24 32 48 64 128 256 512; do
  install -Dm644 "extras/icons/apps/$colour/axentra-onedrive-$n.png" "$DATA/icons/hicolor/${n}x${n}/apps/axentra-onedrive.png"
done
for f in extras/icons/emblems/*.svg; do
  install -Dm644 "$f" "$DATA/icons/hicolor/scalable/emblems/$(basename "$f")"
done
mkdir -p "$DATA/applications" "$CONFIG/systemd/user"
fill extras/desktop/axentra-onedrive.desktop "$DATA/applications/axentra-onedrive.desktop"
fill extras/systemd/axentra-onedrive.service "$CONFIG/systemd/user/axentra-onedrive.service"
# The tray app is a user service tied to the desktop session, so it comes
# back if it crashes or is closed by something else (it used to be an
# autostart entry, which nothing restarted).
fill extras/systemd/axentra-onedrive-tray.service "$CONFIG/systemd/user/axentra-onedrive-tray.service"
rm -f "$CONFIG/autostart/axentra-onedrive.desktop"
# List the app in Axentra Command, with its card in the icon colour.
case "$colour" in periwinkle) accent="#8B97FF" ;; azure) accent="#48AAFF" ;; *) accent="#B8DEFF" ;; esac
mkdir -p "$DATA/axentra/command/apps"
sed -e "s|@APP@|$APP|g" -e "s|@DATA@|$DATA|g" -e "s|@ACCENT@|$accent|g" \
  extras/command/onedrive.json > "$DATA/axentra/command/apps/onedrive.json"

# Set when Files has to restart to load a new copy of the extension.
# GNOME on Wayland doesn't let apps place their windows: a small Shell
# extension opens the panel above the tray icon, with the focus.
EXT=axentra-onedrive@mecmwithshawn.github.io
ext_login=""
if command -v gnome-shell >/dev/null; then
  install -Dm644 -t "$DATA/gnome-shell/extensions/$EXT" extras/gnome-shell/$EXT/metadata.json extras/gnome-shell/$EXT/extension.js
  if ! gnome-extensions enable "$EXT" 2>/dev/null; then
    # The running Shell doesn't know it yet (Wayland loads extensions at
    # sign-in): list it as enabled for the next one.
    enabled="$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || true)"
    case "$enabled" in
      *"'$EXT'"*) ;;
      "" | "@as []") gsettings set org.gnome.shell enabled-extensions "['$EXT']" 2>/dev/null || true ;;
      *) gsettings set org.gnome.shell enabled-extensions "${enabled%]}, '$EXT']" 2>/dev/null || true ;;
    esac
    ext_login=1
  fi
fi

nautilus=""
if python3 -c "import gi; gi.require_version('Nautilus', '4.0')" 2>/dev/null; then
  ext="$DATA/nautilus-python/extensions/axentra_onedrive_nautilus.py"
  mkdir -p "$(dirname "$ext")"
  fill extras/nautilus/axentra_onedrive_nautilus.py "$ext.new"
  if cmp -s "$ext.new" "$ext"; then
    rm -f "$ext.new"
  else
    mv -f "$ext.new" "$ext"
    nautilus=1
  fi
else
  echo "Note: python3-nautilus isn't installed, so file badges and the right-click menu are skipped."
  echo "      sudo apt install python3-nautilus, then run this script again."
fi

gtk-update-icon-cache -q -t -f "$DATA/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$DATA/applications" 2>/dev/null || true
systemctl --user daemon-reload
systemctl --user enable axentra-onedrive.service axentra-onedrive-tray.service >/dev/null 2>&1
# (Re)start a service, even one that failed or gave up restarting.
start() {
  systemctl --user reset-failed "$1" 2>/dev/null || true
  systemctl --user restart "$1" || echo "Couldn't start $1: journalctl --user -u $1" >&2
}
# The tray icon always runs as its service when a desktop session is, so it
# comes back if it crashes. On a first install it's where you sign in.
tray=""
start_tray() {
  if systemctl --user is-active --quiet graphical-session.target; then
    start axentra-onedrive-tray.service
    tray=1
  fi
}

# Set up before (or upgraded from Cirrus), whether or not sync is running
# right now: a reinstall is often how a stopped or failing service gets fixed.
if grep -qs '^client_id = ".' "$CONFIG/axentra/onedrive/config.toml"; then
  start axentra-onedrive.service
  start_tray
  echo
  if [ -n "$upgraded" ]; then
    echo "Upgraded to Axentra OneDrive $version and started it again."
    echo "The command is now: axentra-onedrive (status, pin, free, share, ...)"
  else
    echo "Updated Axentra OneDrive to this build ($version) and restarted it."
  fi
  if [ -n "$ext_login" ]; then
    echo "Sign out and back in once so GNOME opens the panel above the tray icon."
  fi
  if [ -n "$nautilus" ]; then
    echo "Restart Files to load the new badges and menu:  nautilus -q"
  fi
  exit 0
fi

case ":$PATH:" in *":$BIN:"*) ;; *) echo "Add $BIN to your PATH." ;; esac
mount_point="$("$APP/axentra-onedrive" folder --print 2>/dev/null || true)"
mount_point="${mount_point:-$HOME/OneDrive}"
if [ -d "$mount_point" ] && [ -n "$(ls -A "$mount_point" 2>/dev/null)" ] && ! mountpoint -q "$mount_point"; then
  echo "Warning: $mount_point already has files in it. Move them out (or pick another"
  echo "         folder with: axentra-onedrive setup --folder ~/SomewhereElse) before starting."
fi

start_tray
cat <<MSG

Installed Axentra OneDrive $version in $APP. Next:
  1. axentra-onedrive setup        connect your Microsoft app registration (once)
  2. axentra-onedrive login        sign in in your browser
  3. systemctl --user start axentra-onedrive
MSG
if [ -n "$nautilus" ]; then
  echo "  4. nautilus -q                   restart Files to load the badges and menu"
fi
if [ -n "$tray" ]; then
  echo "The tray icon is running and starts with your session; its Sign in button does steps 1 and 2."
else
  echo "The tray icon starts with your next desktop session."
fi
if [ -n "$ext_login" ]; then
  echo "  Then sign out and back in once so GNOME opens the panel above the tray icon."
fi
