# Axentra engineering requirements

- Every desktop application's application logic and native services must be Rust and support native execution on Zorin OS 18.x / Ubuntu 24.04 x86_64, alongside Windows.
- Existing HTML/CSS/JavaScript interfaces may be retained. Horizon remains a browser extension; its native companion must migrate to Rust. Build-time tooling and third-party native libraries are not application-language migrations.
- Do not replace application logic with Python/.NET launchers or use Wine to satisfy Linux support. Existing non-Rust applications remain migration work until their features have been ported and tested.
- Keep platform APIs behind Windows/Linux modules and target-specific Cargo dependencies. Implement Linux behavior; do not silently stub security, encryption, privacy, network or device operations.
- Preserve existing application behavior and user data. Record platform limitations explicitly. A successful build or window startup does not prove feature parity.
- Desktop main windows must use controls integrated into the existing interface, without a separate operating-system title bar. Preserve minimize, maximize/restore, dragging, edge resizing, accessible button labels, and existing unsaved-work/close-to-tray policies. Check compact and alternative layouts as well as the default layout. Horizon uses its browser's window controls.
- Keep private source repositories separate from this public distribution repository unless the user requests publishing them.
- For each app, verify a locked Rust build, meaningful tests, native Zorin startup and representative workflows using disposable data. Record source revision/hash, toolchain, OS/session, package dependencies, test results and remaining checks before declaring support.
- Start with `docs/RUST-ZORIN.md` for the recovered-source baseline. The historical Linux preview records describe different snapshots and must not be treated as current build results.
