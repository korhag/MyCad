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
| `mycad` | egui chrome (menus, status, diagnostics) |

## Linux build

```bash
sudo apt install build-essential pkg-config clang libclang-dev cmake \
    libgtk-3-dev libxcb-shape0-dev libxcb-xfixes0-dev
cargo build --release -p mycad
./target/release/mycad "test-data/KD-1413-260825 Assir Poultry Internal Logistics.dwg"
```

`libclang` is required to compile the vendored LibreDWG C sources via
`libredwg-sys`.

## Usage

- **File → Open** to pick a DWG.
- Pass a path on the command line for repeatable testing.
- Mouse wheel zooms around the cursor; middle-mouse drags pan.
- Double-click the viewport for Zoom Extents.
- Diagnostics lists DWG version, entity counts, unsupported types, extents and timings.

Headless import (no GUI):

```bash
cargo run -p mycad -- --import-only "test-data/KD-1413-260825 Assir Poultry Internal Logistics.dwg"
```

## License

GPL-3.0-or-later (required by LibreDWG).
