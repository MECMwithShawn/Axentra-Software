# Axentra USB

A native Rust desktop app for Windows and Linux, with a Rufus/Etcher-style workflow and explicit boot-chain diagnostics. Version 0.1.1 is a **preview**: physical USB writes and actual firmware booting have not been validated on hardware.

The application, image reader, initramfs inspector, copy engine, raw writer, verification and privilege worker are Rust. The interface uses egui/eframe. There is **no Python, Electron, web server or browser runtime**. Windows storage management uses Microsoft's built-in PowerShell storage cmdlets; Linux formatting uses the system's standard storage utilities.

## Downloads and installation

**Windows x64:** run `AxentraUSB-0.1.1-Setup.exe` to install into `Program Files\Axentra\USB` with Start menu and optional desktop shortcuts. Alternatively, extract `AxentraUSB-0.1.1-Windows-x64.zip` and run `AxentraUSB.exe` for portable use. Both builds are unsigned. Setup requires administrator authorization; the app normally runs unelevated and requests elevation only for physical device operations.

**Zorin / Ubuntu x64:** the previously built Linux packages remain at 0.1.0; the 0.1.1 source includes the updated launcher icon, but no new Linux binary is claimed. Install the Debian package, then open **Axentra USB** from the applications menu:

```sh
sudo apt install ./axentra-usb_0.1.0_amd64.deb
```

The package contains a native x86-64 Linux ELF executable, installs `/usr/bin/axentra-usb`, an application-menu entry and icon, and declares its required runtime dependencies. It targets glibc 2.35 or newer, suitable for Zorin OS 17/18 and Ubuntu 22.04/24.04. Installation and GUI execution on Zorin still need validation.

The Linux portable archive contains the same executable. Extract it and run `./axentra-usb` from the extracted directory; the system dependencies listed in `packaging/package.py` must already be present. No Rust toolchain is needed to run either binary. An OpenGL-capable graphics driver is required.

## Create or verify a USB

1. Choose an uncompressed `.iso` or `.img`. Axentra hashes it and inspects its boot files. Optionally paste the publisher's SHA-256; a mismatch blocks writing.
2. Explicitly select the USB. Internal and system disks are excluded. Devices without stable serial/device identifiers are blocked.
3. Choose **Original image** or the inspected **UEFI compatibility** profile.
4. Review the exact device, image and write method. Creation requires typing the displayed device-specific erase phrase. **Creation erases all existing data on that USB.**
5. Authorize the operating system's device-access prompt. The worker repeats source and device checks, writes the media, and performs read-back verification.
6. Safely eject using the operating system. Test real booting in the firmware boot menu. Alienware uses F12 during startup.

The image, executable and job records cannot reside on the target USB. Stop requests take effect at the next safe boundary. A stopped or failed creation leaves incomplete media and never produces a verification-success result.

**Verify existing USB** reads and compares every byte against an original-image flash. An extracted compatibility layout intentionally differs from the raw ISO, so that mode performs per-file verification during creation instead.

## Creation methods

**Original image:** copies the source image verbatim, clears stale backup partition metadata outside its byte range where space allows, flushes writes, and compares every image byte. A matching partition table alone is never treated as verification. A raw Linux installer may look empty or unformatted in Windows; cancel Windows' format prompt. Raw writing does not make every ISO USB-bootable. Windows installer extraction is not supported.

**UEFI compatibility:** creates GPT, a 1 GiB FAT32 EFI System Partition and an NTFS data partition. It copies EFI programs, GRUB assets, kernel, initramfs and `.disk` metadata to FAT32, then all image contents to NTFS. The original EFI binaries and GRUB menu remain intact. Relative distribution-index symlinks are materialized as file/directory copies. Every copied file is hashed and compared after flushing.

The compatibility profile is limited to the exact Casper and helper-script revision reviewed in the provided Zorin OS 18.1 Pro x64 image. Its checks include the live-media UUID, NTFS tools, FUSE support, Casper's default boot mode and required boot assets. New or differently compressed initramfs revisions fail closed for this profile. Original-image mode remains available when the ISO reader supports the image.

## Findings behind this build

The supplied ISO is 8,381,560,832 bytes. Its `filesystem.squashfs` is **7,458,250,752 bytes**, exceeds FAT32's single-file limit, and spans multiple ISO extents. Axentra reads all extents explicitly.

Despite its `initrd.zstd` filename, this image has three early uncompressed CPIO archives followed by gzip. The inspected startup environment includes NTFS-3G and built-in FUSE support. Casper scans partitions, accepts NTFS, finds `/casper` live files and matches `.disk/casper-uuid` to `2c5c63a7-b60a-4787-8134-b3728fbe9ed0`.

This supports testing the FAT32 + NTFS profile; it does not prove that Alienware firmware will boot it. UEFI permits FAT12, FAT16 and FAT32 removable media, so FAT12 alone does not establish the cause of the earlier failure. Axentra never changes Secure Boot or other firmware settings. Signature trust/revocation and actual boot remain separate checks. See the [UEFI specification](https://uefi.org/specs/UEFI/2.10/13_Protocols_Media_Access.html) and [Zorin installation guide](https://help.zorin.com/docs/getting-started/install-zorin-os/).

## Validation and limits

See `VALIDATION.txt` for the release's actual tests. Core tests use ordinary temporary files and synthetic devices; none formats a real disk. Windows device inventory and image inspection have been exercised. The Linux binary is cross-compiled and structurally checked but has not been run on Zorin. Actual Windows/Linux formatting, physical USB read-back, Alienware boot and live-session startup require testing with a spare USB.

No runtime telemetry, account, cloud service or automatic network access is used. Exported reports contain local file paths and device identifiers; review them before sharing. Worker events stay in the private temporary job directory while the application owns it. Firmware boot is always reported as untested.

## Build and development

Rust 1.95 or newer is required; `Cargo.lock` pins dependencies. On Windows, `./build.ps1` runs tests, builds the app, creates the Inno Setup 6 installer, and packages portable/source ZIPs with SHA-256 checksums in `dist/`. Inno Setup 6 and Python 3.11+ are development prerequisites for packaging; neither is required to run the app. The editable logo and regeneration instructions are in `assets/`.

```sh
cargo test --locked
cargo build --release --locked
```

On Linux, building requires a C linker plus the normal X11/Wayland development environment for eframe. This release was cross-linked from Windows with Zig 0.15.2 targeting `x86_64-linux-gnu.2.35`.

Read-only commands:

```sh
axentra-usb --inspect /path/to/image.iso report.json
axentra-usb --list-devices devices.json
axentra-usb --hash-entry /path/to/image.iso casper/filesystem.squashfs entry-sha256.txt
```

Supply an output filename on Windows because the desktop executable does not attach a terminal. The worker accepts only typed verify/raw/compatible requests. Image data is never executed. All unsafe Rust is isolated to platform calls for device control, privilege launching and locking.

`packaging/package.py` assembles `.deb`, portable archives and checksums from already-built native binaries. Python is a development packaging tool only and is not part of the app or its runtime dependencies.

Axentra USB is independently implemented and is not affiliated with Rufus, balenaEtcher or Zorin. No installation ISO, OS bootloader or Zorin proprietary content is redistributed.
