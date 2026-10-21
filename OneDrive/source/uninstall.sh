#!/usr/bin/env bash
# Remove Axentra OneDrive for the current user. Your settings, sign-in and
# downloaded copies are kept unless you pass --purge. Files on OneDrive aren't touched.
set -euo pipefail
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}"
BIN="${BIN:-$HOME/.local/bin}"
APP="$DATA/axentra/apps/onedrive"
purge=""
[ "${1:-}" = "--purge" ] && purge=1

systemctl --user disable --now axentra-onedrive-tray.service 2>/dev/null || true
pkill -x AxentraOneDrive 2>/dev/null || true
systemctl --user disable --now axentra-onedrive.service 2>/dev/null || true
if [ -n "$purge" ] && [ -x "$APP/axentra-onedrive" ]; then
  # Unlink first: that also removes the previews of your photos it put in
  # ~/.cache/thumbnails, which only its database knows about.
  "$APP/axentra-onedrive" logout --yes >/dev/null 2>&1 || true
fi
rm -f "$BIN/axentra-onedrive" \
  "$CONFIG/systemd/user/axentra-onedrive.service" "$CONFIG/systemd/user/axentra-onedrive-tray.service" \
  "$CONFIG/autostart/axentra-onedrive.desktop" \
  "$DATA/applications/axentra-onedrive.desktop" "$DATA/nautilus-python/extensions/axentra_onedrive_nautilus.py" \
  "$DATA/axentra/command/apps/onedrive.json" \
  "$DATA/icons/hicolor/scalable/apps/axentra-onedrive.svg" "$DATA"/icons/hicolor/*/apps/axentra-onedrive.png "$DATA"/icons/hicolor/scalable/emblems/axentra-onedrive-*.svg
rm -rf "$APP"
systemctl --user daemon-reload 2>/dev/null || true

EXT=axentra-onedrive@mecmwithshawn.github.io
gnome-extensions disable "$EXT" 2>/dev/null || true
enabled="$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || true)"
case "$enabled" in
  *"'$EXT'"*)
    rest="$(printf '%s' "$enabled" | sed "s/, '$EXT'//; s/'$EXT', //; s/'$EXT'//")"
    gsettings set org.gnome.shell enabled-extensions "$rest" 2>/dev/null || true ;;
esac
rm -rf "$DATA/gnome-shell/extensions/$EXT"

if [ -n "$purge" ]; then
  rm -rf "$CONFIG/axentra/onedrive" "$DATA/axentra/onedrive" "$CACHE/axentra/onedrive"
  echo "Removed Axentra OneDrive and its settings, sign-in and local copies."
else
  echo "Removed Axentra OneDrive. Settings and local copies are kept (run with --purge to remove them)."
fi
