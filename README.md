# MyCad

Linux-first 2D CAD application written in Rust. Milestone 1 is a native DWG
viewer: open a production drawing, display it in a wgpu viewport, and pan/zoom
reliably.

LibreDWG is used only inside `dwg-import`. The rest of the application talks to
`cad-core`, so later DXF import and editing will not depend on LibreDWG types.

## Layout

| Crate | Role |
| --- | --- |
| `cad-core` | Native document/entity model (f64 world coordinates) |
| `cad-viewport` | Camera, Zoom Extents, cursor-centered zoom, pan |
| `cad-render` | Tessellation + wgpu viewport renderer |
| `dwg-import` | LibreDWG FFI → `cad-core::Document` |
| `mycad` | egui chrome (menus, selection, properties, settings) |

## Linux build

```bash
sudo apt install build-essential pkg-config clang libclang-dev cmake \
    libgtk-3-dev libxcb-shape0-dev libxcb-xfixes0-dev
cargo build --release -p mycad
./target/release/mycad "test-data/KD-1413-260825 Assir Poultry Internal Logistics.dwg"
```

`libclang` is required to compile the vendored LibreDWG C sources via
`libredwg-sys`.

On Windows, `cargo build --release` can hit an MSVC internal compiler error in
`decode.c`. Use a debug run instead, or keep the workspace `libredwg-sys`
release `opt-level = 1` override:

```powershell
cargo run -p mycad -- "test-data/KD-1413-260825 Assir Poultry Internal Logistics.dwg"
```

## Usage

- **File → Open** to pick a DWG.
- Pass a path on the command line for repeatable testing.
- **Left-click** an entity to select it (line, circle, polyline, block insert, and other drawable types). Nested block geometry selects the parent block.
- **Ctrl+click** or **Shift+click** adds or removes entities from the selection.
- Click empty space or press **Esc** to clear the selection.
- The **Home** ribbon (above the viewport by default) starts LINE and Distance with one click. Drag its tab to another edge, float it, collapse it, or restore it from **View → Show Home**. Layouts saved before Home gain the ribbon once on upgrade.
- The **Properties** panel (left by default) shows a compact read-only inspector for the current selection. Drag its tab to another edge, float it in a window, resize the split, or collapse the leaf. **View → Show Properties** brings it back; **View → Reset layout** restores the default arrangement.
- Mouse wheel zooms around the cursor; middle-mouse drag pans.
- Double-click the viewport or **Ctrl+E** / **Cmd+E** for Zoom Extents.
- Diagnostics lists DWG version, entity counts, unsupported types, extents and timings.

Linetype scale uses definition dash lengths × entity linetype scale × drawing `$LTSCALE`. Paper-space / viewport scaling (`PSLTSCALE`, `MSLTSCALE`) is not applied in this milestone.

### Shortcuts and portable settings

**Settings → Preferences** can rebind selection, pan, and zoom-extents. Bindings match modifiers exactly, so Ctrl+Click does not also fire Click. Conflicts are listed in the dialog.

Use **Export…** / **Import…** to copy a JSON settings file between machines. The file includes zoom speed, shortcuts, and panel layout. It does not include drawings. Import loads into the dialog; **Apply** commits it. Older files missing new fields still load. A newer `schema_version` than this build is rejected.

Headless import (no GUI):

```bash
cargo run -p mycad -- --import-only "test-data/KD-1413-260825 Assir Poultry Internal Logistics.dwg"
```

## License

GPL-3.0-or-later (required by LibreDWG).

Toolbar icons are [Phosphor Icons](https://phosphoricons.com/) (MIT), bundled in the application binary through `egui-phosphor`.
