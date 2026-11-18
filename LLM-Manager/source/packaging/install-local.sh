#!/usr/bin/env bash
# Build Axentra LLM Manager and install it for the current user (no sudo):
# the executable and icon, an application-menu launcher, and an Axentra Command entry.
# Rerun after changing the source. Pass cargo flags through, e.g. ./install-local.sh --offline
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
source_dir=$(dirname "$here")
data=${XDG_DATA_HOME:-$HOME/.local/share}
app=$data/axentra/llm-manager
launchers=$data/applications
command_apps=$data/axentra/command/apps

cargo build --release --locked "$@" --manifest-path "$source_dir/Cargo.toml"
built=${CARGO_TARGET_DIR:-$source_dir/target}/release/llm-manager

mkdir -p "$app" "$launchers" "$command_apps"
# Write beside the target and rename, so a running copy keeps its old file.
install -m 755 "$built" "$app/llm-manager.new"
strip "$app/llm-manager.new" 2>/dev/null || true
mv -f "$app/llm-manager.new" "$app/llm-manager"
install -m 644 "$source_dir/assets/llm-manager-256.png" "$app/axentra-llm-manager.png"

sed -e "s|@EXECUTABLE@|$app/llm-manager|" -e "s|@ICON@|$app/axentra-llm-manager.png|" \
    "$here/axentra-llm-manager.desktop.in" > "$launchers/axentra-llm-manager.desktop"

# Axentra Command reads per-user app entries from this folder; it wants absolute paths.
sed -e "s|~/.local/share/axentra/llm-manager|$app|g" \
    -e "s|~/code/Axentra-Software/LLM-Manager/source|$source_dir|" \
    -e "s|~/code/Axentra-Software/LLM-Manager|$(dirname "$source_dir")|" \
    "$source_dir/axentra.app.json" > "$command_apps/llm-manager.json"

update-desktop-database -q "$launchers" 2>/dev/null || true
echo "Installed Axentra LLM Manager to $app"
