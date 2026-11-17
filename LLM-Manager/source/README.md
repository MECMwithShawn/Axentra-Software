# Axentra LLM Manager source

A native Rust desktop app for the local Ollama daemon, built with eframe and egui 0.36.2. The [product page](../README.md) describes what it does. This page is for building and changing it.

## Build and run

```sh
cargo test --locked
cargo build --locked
./target/debug/llm-manager
```

Add `--offline` when the pinned dependencies are already in the local Cargo cache.

`packaging/install-local.sh` builds a release and installs it for the current user: the executable and icon under `~/.local/share/axentra/llm-manager`, a launcher from `packaging/axentra-llm-manager.desktop.in`, and a per-user Axentra Command entry made from `axentra.app.json`. It replaces the executable by renaming a new file into place, so a copy that is already open keeps running.

## Layout

| File | What it holds |
|---|---|
| `src/main.rs` | The window, the sampling thread and the load and unload requests |
| `src/telemetry.rs` | The Ollama client, the `nvidia-smi` reader and their tests |
| `src/ui.rs` | Everything drawn: title strip, toolbar, Models and Resources |
| `assets/` | The app icon, the state tiles and the script that renders the tiles |
| `packaging/` | The launcher template and the per-user install script |
| `tools/ollama_stub.py` | A stand-in Ollama for testing |

## How it behaves

- The app samples Ollama and the GPU every two seconds on a background thread. **Refresh** asks for a sample straight away.
- **Load** sends an empty generate request with a five-minute keep-alive. **Unload** sends one with a keep-alive of zero. Either one shows as pending until a later sample confirms the model's new state. If no sample confirms it within 40 seconds, the app says so.
- Only one change runs at a time.
- Models that Ollama reports as remote, or whose tag ends in `cloud`, are left out. The app refuses to load them even if asked.
- A reading that is unavailable shows as an em dash. A model's own VRAM is kept apart from the whole GPU's VRAM.

## Testing against a stub

`LLM_MANAGER_OLLAMA_ADDRESS` points the app at another Ollama address. Only loopback addresses such as `127.0.0.1:18080` are accepted. Anything else is ignored and the app uses `127.0.0.1:11434`.

```sh
python3 tools/ollama_stub.py 18080 &
LLM_MANAGER_OLLAMA_ADDRESS=127.0.0.1:18080 ./target/debug/llm-manager
```

Start the stub with `fail` as a second argument to make it refuse load and unload requests. The stub lists five made-up models, one with a very long name. GPU readings still come from the real `nvidia-smi`; run the app with a `PATH` that lacks it to see the no-GPU state.

## Look

The interface follows Lab Manager, a GTK app by the same author: its palette, card and pill shapes, glass state tiles and compact rows.

- **Window.** The window has no system header. The app draws its own title strip. Drag the strip to move the window, double-click it to maximize, and drag any edge or corner to resize.
- **Font.** Text uses Inter when the system has it installed, and egui's bundled font otherwise. Inter is read from the system at start. It is not bundled here.
- **State tiles.** The tiles in the rows are embedded as raw RGBA. `python3 assets/render-tiles.py` renders them again from the SVG recipe in that script. It needs Python with GdkPixbuf.
- **Rows.** Each row is laid out with exact rectangles, so the metric columns and the action button line up from row to row at any width.

## App icon

`assets/llm-manager-256.png` is the app icon, and `assets/llm-manager.svg` is its editable master. The build embeds the PNG and decodes it once, for both the native window icon and the 42 × 42 mark in the header. The launcher uses an installed copy of the same file. The model rows and the GPU heading keep the server-rack tile, so the processor appears only as the app's own mark.

## License

MIT. See [LICENSE](LICENSE). Dependencies and their versions are pinned in [Cargo.lock](Cargo.lock).
