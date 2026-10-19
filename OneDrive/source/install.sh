#!/usr/bin/env bash
# Install Axentra OneDrive for the current user (no sudo), in the Axentra suite
# layout: the app in ~/.local/share/axentra/apps/onedrive, plus icons, the
# Nautilus extension, a systemd user service and the tray autostart entry.
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
tray_was_open=""
if pkill -x AxentraOneDrive 2>/dev/null; then tray_was_open=1; fi
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
mkdir -p "$DATA/applications" "$CONFIG/autostart" "$CONFIG/systemd/user"
fill extras/desktop/axentra-onedrive.desktop "$DATA/applications/axentra-onedrive.desktop"
fill extras/desktop/axentra-onedrive-autostart.desktop "$CONFIG/autostart/axentra-onedrive.desktop"
fill extras/systemd/axentra-onedrive.service "$CONFIG/systemd/user/axentra-onedrive.service"

nautilus=""
if python3 -c "import gi; gi.require_version('Nautilus', '4.0')" 2>/dev/null; then
  mkdir -p "$DATA/nautilus-python/extensions"
  fill extras/nautilus/axentra_onedrive_nautilus.py "$DATA/nautilus-python/extensions/axentra_onedrive_nautilus.py"
  nautilus=1
else
  echo "Note: python3-nautilus isn't installed, so file badges and the right-click menu are skipped."
  echo "      sudo apt install python3-nautilus, then run this script again."
fi

gtk-update-icon-cache -q -t -f "$DATA/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$DATA/applications" 2>/dev/null || true
systemctl --user daemon-reload
systemctl --user enable axentra-onedrive.service >/dev/null

if [ -z "$upgraded" ] && systemctl --user is-active --quiet axentra-onedrive.service; then
  # Reinstall over a running copy: switch to the new build.
  systemctl --user restart axentra-onedrive.service
  if [ -n "$tray_was_open" ]; then
    setsid -f "$APP/AxentraOneDrive" >/dev/null 2>&1 < /dev/null
  fi
  echo
  echo "Updated Axentra OneDrive to this build ($version) and restarted it."
  exit 0
fi

if [ -n "$upgraded" ]; then
  # It was running before the upgrade: bring it straight back.
  systemctl --user start axentra-onedrive.service
  setsid -f "$APP/AxentraOneDrive" >/dev/null 2>&1 < /dev/null
  echo
  echo "Upgraded to Axentra OneDrive $version. Sync and the tray icon are running again."
  echo "The command is now: axentra-onedrive (status, pin, free, share, ...)"
  [ -n "$nautilus" ] && echo "Restart Files to load the new badges and menu:  nautilus -q"
  exit 0
fi

case ":$PATH:" in *":$BIN:"*) ;; *) echo "Add $BIN to your PATH." ;; esac
mount_point="$("$APP/axentra-onedrive" folder --print 2>/dev/null || true)"
mount_point="${mount_point:-$HOME/OneDrive}"
if [ -d "$mount_point" ] && [ -n "$(ls -A "$mount_point" 2>/dev/null)" ] && ! mountpoint -q "$mount_point"; then
  echo "Warning: $mount_point already has files in it. Move them out (or pick another"
  echo "         folder with: axentra-onedrive setup --folder ~/SomewhereElse) before starting."
fi

cat <<MSG

Installed Axentra OneDrive $version in $APP. Next:
  1. axentra-onedrive setup        connect your Microsoft app registration (once)
  2. axentra-onedrive login        sign in in your browser
  3. systemctl --user start axentra-onedrive
  4. Open "Axentra OneDrive" from the app menu (it starts with your session from then on)
MSG
if [ -n "$nautilus" ]; then
  echo "  5. nautilus -q                   restart Files to load the badges and menu"
fi
