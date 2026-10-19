# Zorin 18 IMSM/RST RAID 0 build stage (experimental)

This is an explicit Linux developer build stage for Axentra USB, followed by a
manual target finalization step **before the installer’s first reboot**. It is
not yet a hardware-qualified release or a one-click GUI feature. The desktop
writer never runs code from an ISO. Windows can write the resulting ISO using
**Original image** mode; remastering requires a Linux host and root privileges.

## Architecture and scope

`src/image.rs` inspects ISO9660 and the real Casper initramfs. `operation.rs`
performs verbatim writes or a restricted FAT32/NTFS extraction. Neither installs
packages or changes the live filesystem. Adding files beside an unchanged
squashfs cannot fix the installed OS. Therefore `build-iso.sh` rebuilds the live
squashfs and initramfs before passing its output through the existing writer.
The image inspector now reports whether Axentra IMSM startup files are present;
this is evidence of preparation, not evidence of successful RAID assembly.

The reference repository at commit
`1d0166534c00456d6913d59e000433a668c306dd` describes the missing-initramfs BusyBox
failure. Its wiping/partition preparation is deliberately excluded. `dmraid`
is not installed alongside mdadm: this implementation uses Linux md for IMSM
and avoids a competing device-mapper assembly path. It never disables Intel
platform checks, changes RAID firmware mode, creates arrays, forces degraded
assembly, or modifies partition tables. It supports existing complete IMSM RAID
0 volumes, a direct ext4 root partition, GPT ESP and UEFI x64. LUKS, LVM,
non-IMSM arrays, legacy boot, multiple/layered squashfs images and other Zorin
versions require a separately validated implementation.

## Build a trusted ISO

Use a disposable Linux build VM with enough free space for the extracted ISO,
expanded filesystem, repacked filesystem and output ISO. Verify the source
ISO’s publisher checksum yourself. Root package scripts from the **trusted**
ISO execute in a chroot; this is not a security sandbox for untrusted images.
The host needs `xorriso squashfs-tools initramfs-tools-core cpio gzip xz-utils
zstd python3 util-linux`, and package repository access. The ISO must contain a
matching `/boot/vmlinuz-*` and module tree for its Casper kernel; otherwise the
build stops rather than borrowing the host kernel.

```sh
sudo ./raid/build-iso.sh /path/to/verified-zorin.iso /path/to/zorin-imsm.iso /path/to/new-build-work
```

Output and work paths must not exist, and output must be outside the work tree.
The work tree and original source are retained for review on failure. Mounts
are private, cleaned up in reverse order, and host block devices and firmware
variables are not mounted into the build chroot. `mdadm`, `initramfs-tools` and
Python are installed using the image’s repositories. The mdadm package includes
`mdmon`, udev assembly rules and its distribution initramfs hooks. Additional
hooks force md/RAID0, VMD/NVMe/AHCI and efivarfs support, enable `AUTO +imsm -all`,
and attempt normal non-degraded assembly in Casper premount (live) and local-top
(installed root) before filesystem discovery. The live
configuration contains **no build-host array UUIDs**. The live-media UUID and
Casper boot mode are preserved only in the live initramfs, not in target hooks.

The builder uses gzip for the new initramfs (the historical `initrd.zstd`
filename is retained for GRUB), verifies its contents and Casper UUID, retains
mdadm in Ubiquity manifests, updates filesystem sizes and checksums, and replays
the original ISO’s boot equipment using xorriso. It preserves the source EFI
executables and kernel. Package versions are recorded in the filesystem
manifest and source/output SHA-256 in `image-sha256.txt`. Repository updates
mean builds are traceable, **not byte-for-byte reproducible**; use a pinned apt
snapshot for reproducible package inputs.

Select the output ISO in Axentra and use **Original image**. The existing
compatibility profile may reject the rebuilt Casper revision/compression; its
checks are intentionally not relaxed. Inspection and read-back do not prove
firmware bootability or Secure Boot trust.

## Install without wiping Windows or IMSM metadata

Back up Windows, recovery information and the existing ESP first. Boot the USB
in UEFI mode with the existing RST configuration. Inspect `lsblk -f`,
`cat /proc/mdstat` and `sudo mdadm --detail --scan`. All stripe members and the
expected complete IMSM RAID 0 volume must be present. Stop on missing members,
unexpected volume capacity or platform errors. Do not use `--force`,
`IMSM_NO_PLATFORM`, member-disk formatting or RAID/AHCI mode switching.

Use the installer’s manual partitioning. In **dual boot**, use only previously
prepared free space for Linux, reuse the existing ESP **without formatting**,
and retain Windows, MSR and recovery partitions. In **single boot**, manually
review the desired Linux layout; these tools still do not wipe or repartition
anything. Never target the physical RAID members. No device number or
partition number in these tools identifies your root or ESP.

The installer remains an external program: these scripts cannot prevent a user
from selecting its destructive “erase disk” option or clicking Restart early.
There is no automatic installer success hook or reboot interception in this
preview. Choose **Continue Testing**, then finalize explicitly.

## Finalize the installed OS before reboot

The installed target must already contain mdadm/mdmon and the Axentra hooks.
Mount the **verified target filesystem UUIDs** under a separate directory, such
as `/mnt/installed`; mount the existing ESP at its `boot/efi`, and mount separate
`/boot` there first if applicable. Prepare chroot API mounts at `dev`, `proc`,
`sys`, `run` (`mount --rbind` followed by `mount --make-rslave` is suitable).
These mounts are owned by the caller; the finalizer neither guesses devices nor
unmounts them. Confirm `/etc/fstab` uses the matching filesystem UUIDs for root,
ESP and any separate boot partition.

```sh
sudo axentra-raid-finalize --target /mnt/installed --mode dual
# Or, for a reviewed Linux-only layout:
sudo axentra-raid-finalize --target /mnt/installed --mode single
```

The tool verifies the mounted Zorin ext4 root is on an IMSM RAID 0 volume,
checks UEFI and the mounted GPT FAT ESP, and requires the installer’s Linux EFI
loader. Dual boot also requires Microsoft’s existing EFI loader on that ESP.
It displays the proposed configuration and requires `FINALIZE <root-filesystem-UUID>`.
It resolves the selected root volume and its IMSM container by **md UUID**, writes
an idempotent managed block in `mdadm.conf`, preserves unrelated configuration,
backs up the initial config, and rebuilds **all installed kernel initramfs**.
It inspects the rebuilt images for mdadm/mdmon, udev assembly rules, the startup
script, RAID0 driver and exact persistent configuration. Future distribution
kernel/initramfs updates use the retained hook and current `mdadm.conf`.

Dual boot adds a UUID-based Windows chainloader entry; both modes regenerate
and syntax-check GRUB. There is **no `grub-install`**, ESP formatting, EFI binary
replacement or NVRAM boot-order write. The tool hashes every existing EFI file
before and after and fails if anything changed. It does not install missing
packages during finalization, since package maintainer scripts might write EFI
files. A failure means **do not reboot**; review its message and fix the target.
The config backup is `mdadm.conf.axentra-before`. On success, unmount the chroot
mounts in reverse order, sync, and reboot manually.

## Validation and acceptance gate

Run from `USB/source`:

```sh
python3 -m unittest discover -s raid/tests -v
bash -n raid/build-iso.sh
sh -n raid/zz-axentra-imsm
sh -n raid/axentra-imsm
cargo test --locked
```

Python tests use temporary files, simulated sysfs and mocked command execution;
no real disk is mounted, assembled or formatted. They cover dual/single mode,
Windows preservation, changing md numbers, idempotence, malformed metadata,
mount/config refusal, cancellation and failed initramfs checks. CI also runs
shell checks and the complete existing Rust suite. See `VALIDATION.md` for the
checks actually run for this change.

Before release, build the owner-provided verified Zorin ISO, inspect the rebuilt
squashfs/initramfs and xorriso boot report, boot a spare USB on the intended
Intel firmware, and test both layouts on disposable complete arrays. Verify
live discovery, installer visibility, first and second installed boot, Windows
boot, unchanged Windows/recovery partitions and EFI files, mdmon lifecycle,
shutdown, and a subsequent kernel update/reboot. OVMF can exercise UEFI paths
but cannot establish Intel RST/VMD firmware compatibility, NVRAM behavior,
Secure Boot revocations or the owner’s physical array. Keep the PR in draft
until those checks have evidence.

References: [Ubuntu mdadm.conf](https://manpages.ubuntu.com/manpages/noble/man5/mdadm.conf.5.html),
[mdmon lifecycle](https://www.man7.org/linux/man-pages/man8/mdmon.8.html),
[owner’s reference fix](https://github.com/MECMwithShawn/zorin-imsm-raid0-busybox-fix).
