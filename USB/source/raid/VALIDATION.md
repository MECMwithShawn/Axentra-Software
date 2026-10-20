# Validation evidence — 2026-10-19

Base Axentra commit: `c6a41948406e7f131e0f85de6c1f826daa47ac7f`.
Reference fix inspected: `1d0166534c00456d6913d59e000433a668c306dd`.

Passed locally with Python 3.12.3:

- `python3 -m unittest discover -s USB/source/raid/tests -v`: 19 tests.
  Temporary files, mocked command execution and simulated sysfs only.
- `bash -n USB/source/raid/build-iso.sh`.
- `sh -n USB/source/raid/zz-axentra-imsm` and `sh -n USB/source/raid/axentra-imsm`.
- `git diff --check`.

The Python suite exercises both modes through configuration write, initramfs
inspection and GRUB update, plus refusal/failure paths. It does not run the
real package manager, initramfs generator, mdadm assembly or firmware.

Additional local validation after enabling network access:

- `cargo +1.95.0 test --locked`: **31 passed**, 0 failed, plus binary and doc-test
  targets (0 tests each). Rust 1.95.0 and an extracted Ubuntu GCC 13 toolchain
  were installed under the task workspace; no system toolchain was replaced.
- Full ShellCheck 0.9.0 (all severities) passed for the builder and both hooks.
  Literal dpkg-query placeholders and the image-provided hook-functions source
  are explicitly annotated. Media checksums now use an output file outside
  the input tree before being moved into place.
- The patch was checked against the original base with `git apply --check`.

CI runs the Python suite, shell checks and complete Rust suite. Its result is
reported separately on the draft PR; local checks do not substitute for CI or
hardware validation.

Not performed: complete remaster of a supplied Zorin ISO, physical USB writes,
live startup, real IMSM assembly/mdmon takeover, installed first boot,
Windows boot, kernel-update reboot, Secure Boot, NVRAM, OVMF or Intel firmware
tests. No physical storage was changed. These remain release blockers. This
PR should stay draft, with no merge or production-support claim, until the
README acceptance gate is exercised on disposable media and arrays.

## Integrated Linux app and full Pro remaster — 2026-10-20

The Linux GUI now embeds the preparation scripts, invokes a distinct pkexec
image worker, installs missing build tools, checks a root-owned source copy
against the entered publisher hash, and returns a separately inspected ISO.
The parent publishes without overwriting an existing destination. Stop requests
cleanup; host apt/dpkg operations finish before honoring cancellation. Selecting
RAID mode gates creation on startup evidence and forces Original image mode.
The prepared image includes offline installation/finalization instructions.
No target partition is guessed and finalization remains explicit before reboot.

Checks performed:

- 47 Rust tests (43 library, 4 GUI) and 27 Python tests passed, plus ShellCheck.
- Python tests cover worker request validation, file owner/type/symlink refusal,
  checksum failure before execution, verified source snapshot use, cancellation,
  dependency policy and clean-install archive directory ordering.
- A clean .deb install exposed missing tar directory entries. The packaging
  generator was fixed, a regression test added, and installation then passed.
  The installed /usr/bin/axentra-usb matches the optimized release binary.
- xorriso and its libisoburn dependency installed successfully on this host.
- Full Pro preparation ran through the app's shared engine via --prepare-raid
  and administrator authentication. Actual source copy/hash, extraction,
  image package installation, live initramfs generation/verification, squashfs
  compression and ISO boot-equipment replay completed.
- Source: Zorin OS 18.1 Pro, 8,381,560,832 bytes, SHA-256
  `3984789a2762d1aa359eda5fe19c757e3e5ea9a1bd5a4acc52099c3979905ded`.
- Prepared output: 7,945,977,856 bytes, independent and app SHA-256
  `9e45019e5f529117b7a3e7d2104466feeb9c36a0ea628084cb58a1c5a2e8071b`.
- The final ISO passed the app EFI/IMSM/Casper inspection. xorriso reports its
  replayed El Torito and hybrid system-area boot structures. Source and output
  BOOTX64.EFI, grubx64.efi, mmx64.efi and Casper kernel hashes match exactly.
- Independent unpacking of the final initramfs confirms mdadm/mdmon, both
  startup hooks, generic AUTO +imsm policy without host UUIDs and the preserved
  Casper UUID. Image package versions are retained in filesystem.manifest.

The full build used the shared engine's CLI; the native GUI preparation click
path has only headless coverage. No physical USB write, firmware boot, real
IMSM/mdmon transition or installed-target finalization/reboot was performed.
No existing RAID metadata, Windows files, EFI files or partition table was
modified. Firmware/installer tests remain release blockers; keep PR #1 draft.
A complete automatic installer/reboot integration is not implemented.
