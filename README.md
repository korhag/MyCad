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
| `cad-io` | Native DXF write and PDF export from `cad-core::Document` |
| `dwg-import` | LibreDWG FFI → `cad-core::Document`; DXF import and DXF-interchange DWG save |
| `mycad` | egui chrome (menus, selection, properties, settings) |

## Build

Rust 1.98.1 is pinned in `rust-toolchain.toml`. With [rustup](https://rustup.rs)
installed, the first `cargo` command downloads that toolchain.

`libclang` is required to compile the vendored LibreDWG C sources via
`libredwg-sys`. A C++ toolchain and the windowing libraries below are required
for the native window.

### Windows

Install rustup, the MSVC C++ build tools (Desktop development with C++), and
LLVM so bindgen can load `libclang.dll`:

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install LLVM.LLVM
```

Open a new terminal so `cargo` is on `PATH`. If the build cannot find libclang:

```powershell
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
```

Then:

```powershell
cargo run --release
cargo run -p mycad -- "test-data/KD-1413-260825 Assir Poultry Internal Logistics.dwg"
```

### Linux

Debian and Ubuntu:

```bash
sudo apt install build-essential pkg-config clang libclang-dev \
    libgtk-3-dev libxcb-shape0-dev libxcb-xfixes0-dev mesa-vulkan-drivers
cargo build --release -p mycad
./target/release/mycad "test-data/KD-1413-260825 Assir Poultry Internal Logistics.dwg"
```

Fedora:

```bash
sudo dnf install gcc pkgconf-pkg-config clang clang-devel gtk3-devel \
    libxcb-devel mesa-vulkan-drivers
```

Arch Linux:

```bash
sudo pacman -S --needed base-devel pkgconf clang gtk3 libxcb mesa vulkan-intel vulkan-radeon
```

Install the Vulkan driver that matches the GPU (`vulkan-intel`, `vulkan-radeon`,
or the NVIDIA package). `mesa-vulkan-drivers` is the Debian and Fedora name for
the same drivers.

### Troubleshooting

**Compiler crashes.** If `rustc` or MSVC dies with `STATUS_ACCESS_VIOLATION`
(`0xC0000005`), `STATUS_HEAP_CORRUPTION` (`0xC0000374`), or MSVC `C1001` while
compiling unrelated crates, the machine is unstable. Update the BIOS so the CPU
microcode includes Intel's 13th/14th-gen fixes (`0x12F` or newer), load Intel
Default Settings, and turn off an aggressive RAM overclock (XMP) until the
build is reliable. Meanwhile:

```powershell
cargo run --release -j 1
```

The workspace already builds vendored LibreDWG at `opt-level = 1` and `mycad`
with one codegen unit, because MSVC and rustc have crashed on those units on
affected machines.

**Linker errors after a crash.** `LNK1207: incompatible PDB format`, or any
error about a corrupt `.pdb`, `.rlib`, or `.rmeta` file, means a crashed or
interrupted build left a bad file in `target`. Cargo keeps reusing it. Clear
the release output and build again (use `cargo clean` with no flag to clear
debug output too):

```powershell
cargo clean --release
cargo build --release -p mycad
```

**Window does not open.** If MyCad prints `MyCad failed to start`, update the
GPU driver. To force a graphics backend (useful in a virtual machine or over
Remote Desktop):

```powershell
$env:WGPU_BACKEND = "gl"
cargo run -p mycad
```

```bash
WGPU_BACKEND=vulkan cargo run -p mycad
```

`dx12`, `vulkan`, and `gl` are valid.

### Checking a saved file with ODA File Converter

AutoCAD is not required to build MyCad. To check that a saved DXF or DWG passes an audit, install the free [ODA File Converter](https://www.opendesign.com/guestfiles/oda_file_converter) and point the test at it:

```powershell
$env:MYCAD_ODA_FILE_CONVERTER = "C:\Program Files\ODA\ODAFileConverter\ODAFileConverter.exe"
cargo test -p dwg-import --test reference_dwg oda_file_converter_audits_export
```

The test is skipped when that variable is unset. It converts the written DXF and DWG to AutoCAD 2018 with audit on, and fails if ODA writes an `.err` file.

## Usage

- **File → New** (`Ctrl+N`) starts an empty drawing in millimetres on layer 0. If the current drawing has unsaved edits, MyCad asks before replacing it. The first save uses Save As.
- **File → Open** (`Ctrl+O`) to pick a DWG. If `Plant.dwg.mycad` sits beside it, MyCad loads the dynamic blocks stored there. Opening the companion file itself opens the DWG.
- **File → Save** (`Ctrl+S`) overwrites a previously saved DXF in place. An opened DWG always goes through Save As with a `*-MyCad.dwg` copy name so the original file is not overwritten. A compact Save icon on the menu bar (tooltip **Save** / **Ctrl+S**) runs the same command. Successful saves report in the status bar (`Saved Plant.dxf`, `Saved Plant.dwg`, or `DWG saved with 3 compatibility warnings`) without a dialog. When the original drawing had classes that were not carried over, the status bar also names how many and points at Diagnostics.
- **File → Save As…** (`Ctrl+Shift+S`) writes AutoCAD 2000 DXF or **DWG AutoCAD 2000**. DWG save goes through the DXF writer, then LibreDWG. The DWG stays a normal drawing: dynamic blocks are written as ordinary static blocks, so AutoCAD opens them without MyCad. MyCad also writes `Plant.dwg.mycad` next to the drawing and uses it to restore those dynamic blocks on the next open. A block edited outside MyCad stays static. Older `.mycad` drawings still open; new drawings are not saved in that format. Paper space and extra sheets are written as layouts, including VIEWPORT entities and the sheet size. Block attributes, text styles, layer lock/plot/lineweight, hatch pattern names, dimensions, external-reference paths, and complex linetype shapes are written with the drawing. TrueType font names are saved; the viewport still draws the stroke font. An external reference is not loaded. An image or wipeout is drawn as its frame and saved as a closed polyline, because a real IMAGE object does not survive the DWG writer. If the drawing has entities MyCad cannot fully keep, or a complex linetype that could not be stored, a prompt lists them and asks to **Save a Copy**. Classes that were not carried over are listed in Diagnostics instead of that prompt. The original file is never overwritten. The copy is named `*-MyCad`.
- **File → Export → PDF…** opens a plot dialog, then writes a vector PDF of plottable model-space geometry (not a viewport screenshot). Paper A4–A0, portrait or landscape, extents, fit to page, color or monochrome, and 5/10/15 mm margins. The drawing path and dirty state are unchanged. A finished export reports `Exported Plant.pdf` in the status bar.
- Pass a path on the command line for repeatable testing.
- **Left-click** an entity to select it (line, circle, polyline, block insert, and other drawable types). Nested block geometry selects the parent block.
- **Ctrl+click** or **Shift+click** adds or removes entities from the selection.
- Click empty space or press **Esc** to clear the selection.
- The **Home** ribbon (above the viewport by default) starts drawing and editing commands. Drag its tab to another edge, float it, collapse it, or restore it from **View → Show Home**. Layouts saved before Home gain the ribbon once on upgrade.
- The **Command** dock (below the viewport by default) accepts the same commands by name or alias. Enter or Space runs the text. An empty Enter repeats the last command. Esc clears the line, or cancels the command when the line is already empty. **View → Show Command Line** brings the dock back. Layouts saved before this version gain the dock once.
- Draw: LINE (`L`), POLYLINE (`P`, `PL`), CIRCLE (`C`), ARC (`A`), RECTANGLE (`R`, `REC`), ELLIPSE (`EL`), POLYGON (`POL`), POINT (`PO`).
- Modify: MOVE (`M`), COPY (`CO`, `CP`), ROTATE (`RO`), MIRROR (`MI`), SCALE (`SC`), ERASE (`E`), STRETCH (`S`), TRIM (`TR`), EXTEND (`EX`), OFFSET (`O`).
- Measure: DISTANCE (`D`, `DI`), ANGLE (`AA`), RADIUS (`RAD`), AREA.
- The **Properties** panel (left by default) shows a compact read-only inspector for the current selection. Drag its tab to another edge, float it in a window, resize the split, or collapse the leaf. **View → Show Properties** brings it back; **View → Reset layout** restores the default arrangement.
- Mouse wheel zooms around the cursor; middle-mouse drag pans.
- Double-click the viewport or **Ctrl+E** / **Cmd+E** for Zoom Extents.
- Diagnostics lists DWG version, entity counts, unsupported types, classes that are not carried over on save, extents and timings.

Linetype scale uses definition dash lengths × entity linetype scale × drawing `$LTSCALE`. Paper-space / viewport scaling (`PSLTSCALE`, `MSLTSCALE`) is not applied in this milestone. A complex linetype such as FENCELINE1 keeps its shape and text in the saved file. The viewport and PDF draw the dash lengths only.

### Not preserved yet

A saved file is a normal AutoCAD 2000 drawing. These parts of an opened AutoCAD file are listed in the save prompt and are not written back yet:

- TABLE
- MULTILEADER

Non-graphical data LibreDWG cannot read, such as associative arrays and dynamic-block parameters, is listed in Diagnostics and is not written to the copy.

Model-space lines, polylines, arcs, circles, text, hatches, blocks, attributes, dimensions, and layout sheets are written and can be edited. Text styles are saved; TrueType faces are not drawn. External references keep their path and draw nothing until the file is present. An opened image or wipeout is shown as its frame; the save writes that frame as a closed polyline rather than an IMAGE or WIPEOUT entity.

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
