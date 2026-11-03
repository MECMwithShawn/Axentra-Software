# Axentra USB

A native Rust desktop app for Windows and Linux, with a Rufus/Etcher-style workflow and explicit boot-chain diagnostics. Version 0.1.1 is a **preview**: physical USB writes and actual firmware booting have not been validated on hardware.

The application, image reader, initramfs inspector, copy engine, raw writer, verification and privilege worker are Rust. The interface uses egui/eframe. The desktop UI has no Electron, web server or browser runtime. Optional Linux RAID image preparation uses bundled Python/shell helpers and system build utilities. Windows storage management uses Microsoft's built-in PowerShell storage cmdlets; Linux formatting uses the system's standard storage utilities.

## Downloads and installation

**Windows x64:** run `AxentraUSB-0.1.1-Setup.exe` to install into `Program Files\Axentra\USB` with Start menu and optional desktop shortcuts. Alternatively, extract `AxentraUSB-0.1.1-Windows-x64.zip` and run `AxentraUSB.exe` for portable use. Both builds are unsigned. Setup requires administrator authorization; the app normally runs unelevated and requests elevation only for physical device operations.

**Zorin / Ubuntu x64:** the previously built Linux packages remain at 0.1.0; the 0.1.1 source includes the updated launcher icon, but no new Linux binary is claimed. Install the Debian package, then open **Axentra USB** from the applications menu:

```sh
sudo apt install ./axentra-usb_0.1.0_amd64.deb
```

The package contains a native x86-64 Linux ELF executable, installs `/usr/bin/axentra-usb`, an application-menu entry and icon, and declares its required runtime dependencies. It targets glibc 2.35 or newer, suitable for Zorin OS 17/18 and Ubuntu 22.04/24.04. Installation and GUI execution on Zorin still need validation.

The Linux portable archive contains the same executable. Extract it and run `./axentra-usb` from the extracted directory; the system dependencies listed in `packaging/package.py` must already be present. No Rust toolchain is needed to run either binary. An OpenGL-capable graphics driver is required.

## Create or verify a USB

1. Choose an uncompressed `.iso` or `.img`, or enable **Download an ISO** to download and verify one. Axentra hashes it and inspects its boot files. Optionally paste the publisher's SHA-256; a mismatch blocks writing.
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

No runtime telemetry or automatic downloads are used. Explicit ISO downloads contact the entered HTTPS server and its HTTPS redirects. Signed download URLs are not included in exported reports. Exported reports contain local file paths and device identifiers; review them before sharing. Worker events stay in the private temporary job directory while the application owns it. Firmware boot is always reported as untested.

## Build and development

Rust 1.95 or newer is required; `Cargo.lock` pins dependencies. On Windows, `./build.ps1` runs tests, builds the app, creates the Inno Setup 6 installer, and packages portable/source ZIPs with SHA-256 checksums in `dist/`. Inno Setup 6 and Python 3.11+ are development prerequisites for packaging; neither is required for the Windows app. The editable logo and regeneration instructions are in `assets/`.

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

`packaging/package.py` assembles `.deb`, portable archives and checksums from already-built native binaries. Python is used for development packaging and the optional Linux RAID preparation worker; ordinary ISO downloads and USB writes use the Rust engine.

Axentra USB is independently implemented and is not affiliated with Rufus, balenaEtcher or Zorin. No installation ISO, OS bootloader or Zorin proprietary content is redistributed.

## Experimental Zorin IMSM/RST RAID 0 images

The [Linux RAID build stage](raid/README.md) can prepare a trusted Zorin 18 ISO
with mdadm/mdmon in the live and installed filesystem. Write its output using
**Original image**, then run the guarded installed-target finalizer before the
first reboot. The Linux app can now run this preparation stage; it is not automatic RAID
support for arbitrary ISOs or a hardware-qualified release. Manual partitioning
must preserve Windows and the existing ESP in dual-boot installations.


## Download and verify an ISO (preview)

In **Source image**, enable **Download an ISO**. Select **Zorin OS 18.1 Core**
to populate the public mirror URL and publisher SHA-256, or paste a direct
HTTPS URL and its matching 64-character SHA-256 from the publisher. Pro editions
require your own authorized direct download link; Axentra does not handle
purchases, browser login sessions or cookies. Use **Publisher checksums** to
check the pinned version against Zorin’s site.

Choose **Download and verify…** and select a new `.iso` filename. Progress shows
received bytes and the total when provided by the server. The existing **Stop**
button cancels during connection, transfer or verification. Network waits poll
cancellation every 100 ms and fail after 30 seconds without progress. Closing
the app also cancels and waits for its download worker to clean up.

Downloads use a unique temporary file in the destination directory. The saved
file is hashed after the transfer, then published without overwriting an
existing destination only when the publisher SHA-256 matches. HTTP errors,
truncation, checksum mismatches, write errors and cancellation remove the
partial file. An abrupt process kill or power loss can leave an identifiable
`.axentra-download-*.part` file, which is never treated as a verified ISO.
Downloads restart from the beginning; resume is not implemented.

Verified downloads automatically enter the existing image-inspection flow.
Downloading does not select a USB, elevate privileges, start the writer or
change partitions. Image verification and actual bootability remain distinct.

For reproducible testing, the same downloader is exposed on the command line:

```sh
axentra-usb --download HTTPS_URL PUBLISHER_SHA256 /path/to/new-image.iso
```

Ctrl+C cancels this command. It prints progress to stderr and a JSON receipt
(path, byte count and SHA-256) to stdout. The GUI exports this receipt alongside
its inspection report, without the download URL or its query parameters.
See [download validation](DOWNLOAD-VALIDATION.md) for automated and real-Zorin
test evidence.


## Prepare RAID images in the Linux app (experimental)

After downloading or selecting a trusted Zorin 18 ISO, enter its publisher
SHA-256 and enable **Prepare for Intel RAID**. Choose **Prepare RAID image…**
and a new output filename. Authenticate in the system administrator dialog.
The app installs missing image-build dependencies (including xorriso) through
apt, then prepares a separate ISO and checks its EFI/IMSM startup contents.
It selects the result in **Original image** mode. Select a USB afterward;
the existing device-specific erase confirmation is still required.

Preparation requires Linux with Python 3 and pkexec, internet access, 80 GiB
free in /var/tmp plus output space, and a source ISO owned by the current
user. The .deb includes Python 3 as a dependency; portable Linux users must
install Python 3 and pkexec first. Windows can write an already prepared ISO
but cannot run the Linux preparation stage. Embedded scripts ship inside the
native binary; no repository checkout is needed. Package scripts from the
trusted ISO execute as root inside the build chroot: use only publisher-verified
images. A disposable Linux build VM remains recommended.

Stop requests cleanup; apt/dpkg setup must finish before cancellation takes
effect. The root-owned build tree and temporary output are removed on normal
failure/cancellation. Abrupt termination or power loss may leave temporary
files. Completed files are published without overwriting an existing output.
The source checksum is rechecked from a root-owned snapshot before building.
Host mdadm/bootloader packages are not installed by this step.

The prepared live image includes **Axentra RAID installation instructions**
in its applications menu. Manual partitioning and explicit target finalization
before reboot are still required; this preview does not intercept Ubiquity's
erase/restart actions. Both Windows-preserving and single-boot layouts use the
same prepared ISO and the appropriate finalizer mode. Do not treat successful
image preparation as successful Alienware/RST or Secure Boot validation.

Equivalent image-only CLI (no USB write):

```sh
axentra-usb --prepare-raid SOURCE.iso PUBLISHER_SHA256 NEW_OUTPUT.iso
```
