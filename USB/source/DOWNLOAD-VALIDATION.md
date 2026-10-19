# ISO download validation (2026-10-19)

The GUI and `--download` CLI use the same Rust HTTPS transfer and saved-file
SHA-256 verification engine. No elevated worker or storage-write API is called
by this path.

## Automated checks

- `cargo test --locked`: 41 library tests and 2 GUI tests passed. The 10 new
  downloader tests cover successful publication/progress, unknown-length
  chunked responses, bad checksums, truncated/empty bodies, HTTP errors,
  unsafe input URLs, invalid hashes, existing/concurrently created destinations,
  cancellation during transfer/hash/header waits, redirects and stalled bodies.
- The GUI tests render the download panel headlessly and exercise the verified
  result's asynchronous handoff into image inspection. They do not automate
  the native save dialog or a physical USB operation.
- Existing 19 Python RAID tests and full ShellCheck passed.
- An optimized Linux x86-64 preview binary built successfully with Rust 1.95.0.
  Windows runtime behavior has not been exercised.

## Real Zorin transfer

The test source is Zorin OS 18.1 Core 64-bit from the University of Maryland
mirror linked by Zorin's official download page:

- https://zorin.com/os/download/18/core/
- https://mirror.umd.edu/zorin/18/Zorin-OS-18.1-Core-64-bit.iso
- https://help.zorin.com/docs/getting-started/check-the-integrity-of-your-copy-of-zorin-os/

The pinned publisher SHA-256 is:

```text
44649b97bd307fc4c8529205d098ebbf98575a1e1ba2ee7d7005b697af1721d5
```

A real transfer was cancelled after a partial file exceeded 1 MiB. The command
exited with the cancellation message, no completed ISO existed, and no partial
file remained.

A subsequent full transfer using the optimized app's `--download` command
completed successfully: 3,909,091,328 bytes, matching the publisher SHA-256 above.
Independent `sha256sum` and the app's `--inspect` produced the same hash.
Inspection found the x64 removable-media EFI loader and passed the reviewed
Casper/NTFS profile. It correctly warns that IMSM support is not established:
this is the stock ISO, not a remastered RAID-enabled image.

The desktop preview launched on the Linux host without a startup error. Native
mouse/file-dialog interaction has not been automated. The real network tests
exercise the exact engine used by the GUI, through the app's CLI.

Reproduce with the built app (use a new output path):

```sh
axentra-usb --download https://mirror.umd.edu/zorin/18/Zorin-OS-18.1-Core-64-bit.iso 44649b97bd307fc4c8529205d098ebbf98575a1e1ba2ee7d7005b697af1721d5 Zorin-OS-18.1-Core-64-bit.iso
sha256sum Zorin-OS-18.1-Core-64-bit.iso
axentra-usb --inspect Zorin-OS-18.1-Core-64-bit.iso inspection.json
```

An earlier process interruption left an unverified `.part` file, which was
removed explicitly before completion of the retry. Process-kill recovery and
resuming downloads are not implemented; no partial file is published as an ISO.

This validates download integrity, not RAID support or bootability. No USB was
written, no Windows files were changed, and no RAID configuration, partition
layout or EFI files were modified. PR #1 remains draft pending its separate
full-image and hardware acceptance gate.
