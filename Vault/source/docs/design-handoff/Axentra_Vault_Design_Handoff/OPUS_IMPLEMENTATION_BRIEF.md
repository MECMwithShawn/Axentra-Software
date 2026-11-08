# Axentra Vault | Opus implementation brief

## Objective
Implement the supplied visual concepts as an optional, production-quality **Floating Modular** interface for the existing Axentra Vault Windows desktop password manager, written in Rust with egui/eframe. Keep **Fully Custom** as a possible future theme. This is a UI integration task, not a rewrite of vault storage, crypto, authentication or data models.

## Reference assets
- `floating_modular_concept.png`: primary visual target, floating title/search bar, navigation, item list, credential detail, security and activity panels.
- `fully_custom_concept.png`: secondary visual reference for chamfered frame accents, vault door icon language, palette and chrome.

These are *concept images*, not implementation-accurate screenshots. Do not reproduce misleading placeholder security state or Windows desktop wallpaper/taskbar as part of the application.

## First: inspect and report
1. Inspect `Cargo.toml` and lockfile to determine precise egui, eframe, winit and renderer versions and existing feature flags.
2. Identify current startup entry point, `eframe::App` implementation, window configuration, widget utilities, icon module, authentication state, vault listing/detail components and theme system.
3. Identify existing global keyboard shortcuts, focus rules, password masking, clipboard clearing and screen privacy handling.
4. Present a short compatibility and change plan *before editing*. Preserve existing architecture and dependencies unless strictly needed.

## Implementation direction
- First build a **single OS window** with a borderless, translucent outer surface and opaque, independently laid-out egui panels. This is easier to maintain and avoids independent Windows taskbar/focus/Alt-Tab entries per module. Panels should *look* detached with spacing, shadows and custom painted borders. Do not treat them as separate native HWNDs by default.
- Provide optional draggable/resizable panel arrangements **inside** the app with reasonable constraints and persistence using existing settings storage. Do not permit security-sensitive panels to drift offscreen.
- Use a compact floating top bar with Axentra Vault name, the existing vault door icon, search and window controls.
- Include navigation, vault list, credential details and a collapsible security sidebar. Activity is optional if current application has the supporting data.
- Keep a compact fallback for smaller windows, high DPI and accessibility text scaling.
- Use a palette of opaque charcoal/navy panels, subtle cyan outlines and glow. Do not make credential fields transparent. Treat Windows Mica/Acrylic as an optional Windows-specific enhancement, not a prerequisite; egui transparency alone is not background blur.
- Use the **existing actual icon geometry** rather than recreating a mismatched brand asset.

## Window integration requirements
- Use the exact current eframe/egui APIs after checking installed source and Cargo versions, not speculative code copied from a different release.
- Borderless windows need custom titlebar drag/resize hit-testing and usable minimize, maximize/restore and close controls; retain keyboard focus, accessibility and Windows snap behavior where feasible.
- Distinguish visual chamfered panels from a truly non-rectangular OS hit-test region. Start with visual clipping; do not promise actual shaped HWND click-through without explicit native Windows work.
- Implement sensible opaque fallback if transparency or compositor integration is unavailable, on Remote Desktop, or under high contrast settings.
- Honor Windows reduced-motion settings for animations; provide an accessible no-motion fallback.

## Security / UX invariants
- Authentication must govern **all** secret data and actions. When vault is locked, **never** show usernames, passwords, URLs, notes or TOTP belonging to locked content. The concept mockups are illustrative only.
- Never infer breach-free or sync-secure status from visual design. Display only state derived from real implementations.
- Password text stays masked by default, and reveal/copy actions remain gated by existing policy. Preserve current security controls, clipboard handling, timeout/auto-lock and vault memory practices.
- Do not log credentials, sensitive user data or the contents of protected widgets.
- Search and master-password input must be responsive immediately. No startup overlay should block input, and the startup animation depicts the vault locking, never appearing unlocked before successful authentication.

## Phased deliverables
1. **Design tokens:** palette, typography, spacing, corner/chamfer geometry, state colors, hover/focus/disabled visuals.
2. **Shell prototype:** floating modular layout and custom chrome, connected to synthetic/demo entries with clearly labelled mock data. Keep existing UI switchable.
3. **Integration:** wire modules into real existing app state while preserving security invariant boundaries.
4. **Quality:** test Windows 11, light/dark Windows settings, 100/125/150/200% scaling, small/large displays, keyboard navigation, reduced motion, high contrast and RDP fallback.
5. **Delivery:** supply source diffs, screenshots, dependency changes, build/test commands, and a feature flag or config switch to revert to original layout.

## Acceptance criteria
- Existing vault open/lock/unlock, CRUD, search and clipboard functions still behave as before.
- No protected data leaks into locked UI, rendering, logs or notifications.
- Custom chrome works for drag, resize, maximize/restore, minimize and close on supported Windows targets.
- No blank window / black corners in transparency-disabled environments; unsupported effects degrade gracefully.
- The floating modules maintain readable contrast and respond at high DPI.
- `cargo fmt`, `cargo clippy`, and `cargo test` pass for the repository's configured toolchain (report pre-existing failures separately).
- The repository's normal Windows release build succeeds.

## Workflow instruction for Opus
Work directly in the **existing Axentra Vault repository**. Review current code before deciding on changes. Show a concise plan, implement the smallest viable shell behind a toggle, compile and test it, and provide evidence and screenshots. Do not silently modify cryptography, authentication, data storage or password policies. Ask for permission before making destructive or architectural changes.
