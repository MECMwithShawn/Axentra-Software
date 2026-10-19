#!/usr/bin/env bash
# Regenerate the app icon in each colour offered in Settings > Appearance.
# Usage: extras/icons/render-variants.sh /path/to/Axentra-Software
# Ice is the Icon Kit's own onedrive icon; the others swap only its accent.
set -euo pipefail
kit="${1:?path to the Axentra-Software checkout}/Icon-Kit"
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cp -r "$kit/renderer" "$work/renderer"
declare -A accents=([ice]="184, 222, 255" [periwinkle]="139, 151, 255" [azure]="72, 170, 255")
for colour in ice periwinkle azure; do
  sed -E "s/\"onedrive\" => \[[0-9, ]+\]/\"onedrive\" => [${accents[$colour]}]/" \
    "$kit/renderer/src/suite.rs" > "$work/renderer/src/suite.rs"
  CARGO_TARGET_DIR="$work/target" cargo run -q --release --manifest-path "$work/renderer/Cargo.toml" -- "$work/out-$colour"
  mkdir -p "$here/apps/$colour"
  for n in 16 24 32 48 64 128 256 512; do
    cp "$work/out-$colour/onedrive-$n.png" "$here/apps/$colour/axentra-onedrive-$n.png"
  done
  cp "$work/out-$colour/onedrive-128.png" "$here/../../crates/ui/ui/icons/app/$colour.png"
done
cmp -s "$here/apps/ice/axentra-onedrive-512.png" "$kit/assets/onedrive-512.png" \
  && echo "Ice matches the Icon Kit." || { echo "Ice differs from the Icon Kit" >&2; exit 1; }
