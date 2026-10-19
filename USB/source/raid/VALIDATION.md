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
