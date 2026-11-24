#!/usr/bin/env bash
# Run inside a disposable X server: dbus-run-session -- xvfb-run -a bash tools/smoke-usb-linux.sh BINARY LOG_DIRECTORY
# This checks window startup only. It never selects an image or starts a device operation.
set -euo pipefail
binary=$(realpath "${1:?Pass the native USB executable}")
logs=$(realpath -m "${2:?Pass a log directory}")
mkdir -p "$logs"
profile=$(mktemp -d)
app_pid=''
cleanup() {
    if [[ -n "$app_pid" ]]; then
        kill "$app_pid" 2>/dev/null || true
        wait "$app_pid" 2>/dev/null || true
    fi
    rm -rf "$profile"
}
trap cleanup EXIT
export XDG_CONFIG_HOME="$profile/config" XDG_DATA_HOME="$profile/data"
export XDG_CACHE_HOME="$profile/cache" XDG_RUNTIME_DIR="$profile/runtime"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
unset WAYLAND_DISPLAY WAYLAND_SOCKET
export XDG_SESSION_TYPE=x11 WINIT_UNIX_BACKEND=x11 LIBGL_ALWAYS_SOFTWARE=1
"$binary" > "$logs/gui.log" 2>&1 &
app_pid=$!
for attempt in {1..30}; do
    kill -0 "$app_pid"
    # The X server is private to this test; do not depend on toolkit PID properties.
    window=$(xdotool search --onlyvisible --name '^Axentra USB$' 2>/dev/null | head -1 || true)
    if [[ -n "$window" ]]; then
        xdotool getwindowgeometry --shell "$window" > "$logs/window.txt"
        sleep 2
        kill -0 "$app_pid"
        echo 'PASS: native Axentra USB window opened and stayed running.'
        exit 0
    fi
    sleep 1
done
echo 'FAIL: no native Axentra USB window appeared within 30 seconds.' >&2
exit 1
