# Floating Modular interface: implementation report

Implements `docs/design-handoff/Axentra_Vault_Design_Handoff/OPUS_IMPLEMENTATION_BRIEF.md`. The primary target was `floating_modular_concept.png`.

## Environment checked before changes
- eframe / egui / egui-winit / egui_glow / egui_extras 0.36.2, winit 0.30.13, wgpu 30.0.1 and glow 0.17.0 (eframe default features).
- `eframe::App::clear_color`, `ViewportBuilder::with_transparent` / `with_decorations`, and `ViewportCommand::{StartDrag, BeginResize, Maximized, Minimized, Decorations}` were verified in the installed sources.

## What changed
| Area | Files |
|---|---|
| Layout switch (Floating / Classic), stored in `%APPDATA%\Axentra Vault\layout.txt` | `app/src/settings.rs`, `app/src/app.rs` |
| Frameless transparent window, floating title bar, window controls, edge resize, opaque fallback | `app/src/chrome.rs`, `app/src/main.rs` |
| Module design tokens (gap 14, padding 16, chamfer 14, bracket 22, module fill `#0a131e`, cyan edges) | `app/src/chrome.rs`, `app/src/theme.rs` |
| Floating layout: navigation, vault list, details, security overview, 2FA, activity, Generator, Settings | `app/src/session/floating.rs` |
| Shared per-frame logic split out of the classic layout (idle lock, shortcuts, categories) | `app/src/session.rs` |
| Favourites and tags helpers; TOTP enabled in the `keepass` crate | `crates/vault-core/src/organize.rs`, `crates/vault-core/Cargo.toml` |
| New icons: star, star-filled, tag, settings, launch, plus, more, check, cross, activity, unlock | `app/assets/icons/` |

Dependency changes: `windows-sys` features `Win32_UI_Accessibility` and `Win32_UI_Shell`, and the `keepass` feature `totp`. No new crates were added.

## Switching layouts
- **Floating → Classic:** Settings → Layout → Classic.
- **Classic → Floating:** the "Floating layout" button in the toolbar.

The choice is remembered. Classic restores the normal Windows title bar.

## Security invariants
- **Locked state:** the locked screen renders only the top bar and the unlock card. Test `floating_locked_screen_shows_no_vault_data` checks that no entry titles, navigation or actions appear before unlock.
- **Masking:** passwords and protected fields stay masked until Show.
- **Copying:** goes through the existing clipboard path (12 s clear, excluded from clipboard history and cloud clipboard).
- **Auto-lock and memory:** auto-lock and zeroisation are unchanged.
- **Recent Activity:** kept in memory for this session only. It records the action and the entry title, never values.
- **Launch Site:** opens only `http://` and `https://` addresses through `ShellExecuteW`. Test `only_web_addresses_launch` covers this.
- **Startup animation:** unchanged. Input works from frame zero, and the lock is closed on every frame.

## Deviations from the concept (intentional)
| Concept | Implementation | Why |
|---|---|---|
| "No known breaches" | Not shown | No breach check exists; claims come only from real state |
| "Vault synced" / "Last sync" | "Saved" or "Unsaved changes" | No sync feature; OneDrive sync happens outside the app |
| "Windows Hello Ready" | "Passkey" / "Password" | Windows Hello is not implemented; phone passkey is |
| Brand logos (GitHub, Discord, …) | Initial tiles coloured by category area | Trademarks; fetching icons would reveal the user's sites |
| Auto-fill on Windows activity | Not shown | No auto-type or auto-fill yet |
| Wallpaper, taskbar, slogans outside the window | Not drawn | The brief says not to reproduce them |
| Draggable or rearrangeable modules | Fixed responsive layout | Optional in the brief; avoids panels drifting off-screen |
| Acrylic / Mica blur | Transparency without blur | egui cannot blur; Mica needs native DWM work |
| Entry editor as an in-window module | Still its own OS window | Unchanged from the existing app; candidate for a later pass |

## Transparency on Windows
eframe's default renderer is wgpu. On Windows its DirectX/Vulkan swapchains present opaque, so the transparent gaps showed white. The app now enables eframe's `glow` feature and selects `Renderer::Glow` (OpenGL), which composites per-pixel alpha.

How this was verified: a build was placed over a topmost solid-magenta window, and screen pixels were sampled from a DPI-aware script. All four gap points read pure magenta (255, 0, 255), and module areas drew normally.

## Responsive behaviour
- **Navigation rail:** collapses to icons below 1180 px of content width.
- **Security column:** hides below 1240 px. It can also be hidden from the panel ("Hide") and restored from the details "⋯" menu.

## Build and test
```
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test -p axentra-vault -p vault-core
interop/run.ps1                                        # KeePass round trip
cargo test -p axentra-vault render_previews -- --ignored   # writes app/assets/preview/*.png
cargo build --release -p axentra-vault
```
Results at hand-off:
- tests: 25 UI, 39 core, 2 interop, all passing (plus KeePassLib verification: ALL PASSED)
- lint: zero clippy warnings
- release build: succeeds

## Screenshots
Rendered offscreen from the real application:
- `app/assets/preview/floating-locked.png`
- `app/assets/preview/floating-main.png`

The offscreen renderer has no desktop behind the window, so the transparent gaps show as white there.

## Not yet verified (needs the real desktop)
- Windows 11 at 125/150/200 % scaling
- Snap Layouts on the custom maximise button
- the Remote Desktop and high-contrast opaque fallback
- keyboard-only navigation through every module

The fallback code paths exist (`chrome::transparency_available`) but have not been exercised on those systems.
