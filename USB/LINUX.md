# Axentra USB on Zorin Linux

**0.1.1 development build**, verified 2026-10-19 on **Zorin OS 18.1 / Ubuntu 24.04 x86_64**, using Rust 1.99.1. This is a native Rust/egui application. No Python or .NET application runtime is required.

## Verification

- `cargo test --locked`: **30 tests passed**, none failed or ignored. Tests use temporary files and synthetic device data.
- `cargo build --release --locked`: native x86_64 Linux release executable built successfully on Zorin.
- Native X11 startup passed under an isolated Xvfb display with software OpenGL and disposable XDG directories. A visible **1160 × 840** Axentra USB window opened and stayed running. This checks startup, not every UI interaction or rendering correctness.
- Three packaging regression tests pass, including rejection of a missing/invalid glibc baseline.
- Debian package metadata/payload were inspected, the extracted executable matched the tested release executable byte for byte, and APT's simulated installation resolved the declared dependencies. The package was not installed system-wide.

Physical USB writes, formatting, read-back on a real device, firmware boot, Secure Boot, Wayland and the host GPU path remain unverified. No image was selected and no device operation was started by the smoke test.

## Build on Zorin 18

Install Rust 1.99.1 and the development dependencies:

```sh
sudo apt install build-essential pkg-config libx11-dev libxi-dev libxcursor-dev libxrandr-dev libxkbcommon-dev libwayland-dev libegl1-mesa-dev libgl1-mesa-dev libudev-dev
cargo test --locked --manifest-path USB/source/Cargo.toml
cargo build --release --locked --manifest-path USB/source/Cargo.toml
```

For the isolated startup check, install `xvfb`, `xdotool` and `dbus-x11`, then run from the repository root:

```sh
timeout 40s dbus-run-session -- xvfb-run -a bash tools/smoke-usb-linux.sh USB/source/target/release/axentra-usb /tmp/axentra-usb-smoke
```

## Package

Python 3.11+ is used only to assemble packages during development:

```sh
python3 USB/source/packaging/package.py \
  --linux USB/source/target/release/axentra-usb \
  --linux-glibc-min 2.39 \
  --output USB/source/dist
```

The explicit baseline must match the build host/sysroot. Native Zorin 18 / Ubuntu 24.04 packages conservatively declare **glibc 2.39+**. An older glibc 2.35-targeted build is a separate artifact and requires its own build and validation. Merely passing `2.35` to the packager does not make a Zorin 18 binary compatible with Zorin 17.

The package installs `/usr/bin/axentra-usb`, its desktop entry and icon, and declares graphics/storage dependencies. Packages are development artifacts until explicitly released; no public Linux release is implied by this validation.
