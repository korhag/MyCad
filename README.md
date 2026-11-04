# EntoCAD

Linux-first 2D CAD application written in Rust. Milestone 1 is a native DWG
viewer: open a production drawing, display it in a wgpu viewport, and pan/zoom
reliably.

LibreDWG is used only inside `dwg-import`. The rest of the application talks to
`cad-core`, so later DXF import and editing will not depend on LibreDWG types.

Development follows a fully open-source path. AutoCAD fidelity is the target,
but the current DXF-interchange writer does **not** preserve native AutoCAD
dynamic-block controls or every DWG object. Saving offers a separate copy when
these losses are known; it must not overwrite the source drawing. See the
[compatibility contract and implementation plan](docs/autocad-compatibility.md).

New companion files are bound to the exact saved DWG/DXF bytes. If AutoCAD or
another application changes the drawing, or a legacy helper has no binding,
EntoCAD opens the drawing without applying that helper and reports the reason.
This deliberately includes edits that keep the same block bounds. Keep legacy
helpers with their original drawings; they are not silently migrated. Missing
helpers leave the normal drawing geometry usable.

For creating parameters, connecting stretch/move actions, and building reusable
folder libraries, see the [dynamic block and library guide](docs/dynamic-blocks-and-libraries.md).

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

### macOS

Install Rust with [rustup](https://rustup.rs) and the Xcode Command Line Tools:

```bash
xcode-select --install
```

If bindgen cannot find libclang from Xcode, install LLVM with Homebrew and set
`LIBCLANG_PATH` to its library directory before running the installer:

```bash
brew install llvm
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
```

## Install as a desktop app

The three installers live in `scripts/`. They build the checked-out release
with `Cargo.lock` and install it for the current user. Install the build tools
listed above first; Linux also requires Python 3 to read Cargo's build output.
The first build can take several minutes. No administrator privileges are
needed for the app installation. Cargo may download the pinned toolchain and
project dependencies.

| OS | Install and open (run from the repository root) | Installed app |
| --- | --- | --- |
| Windows | `powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\install-windows.ps1 -Launch` | `%LOCALAPPDATA%\Programs\EntoCAD\EntoCAD.exe` |
| Linux | `bash scripts/install-linux.sh --launch` | `~/.local/bin/entocad` |
| macOS | `bash scripts/install-macos.sh --launch` | `~/Applications/EntoCAD.app` |

On Windows you can also double-click `scripts\install-windows.cmd`.
ExecutionPolicy Bypass applies to that process only; organizational policies
may still require your administrator's approved way to run local scripts.
Omit `-Launch` / `--launch` to install without opening the app.

Windows gets a Start menu entry and **Open with** entries for DWG/DXF; Linux
gets an application-menu entry and file-manager launcher. On macOS, open the
app from your user Applications folder in Finder. Pin the Start/application
menu entry to the taskbar, or drag the macOS app to the Dock. Pinning remains
your choice. Existing pins keep using the same installed path after updates.
On macOS use **File → Open** for drawings; Finder document associations are
not registered yet. The installed app is separate from Cargo's build folder,
so `cargo clean` does not remove it.

### Updating after a pull

1. Run `git pull` in the checkout that originally installed the app.
2. Run the same installer again, or open the installed EntoCAD and accept
   **Update and restart** when offered. **Settings → Check for updates** checks
   the local checkout again at any time.
3. Save your drawing and finish pending commands or block/property edits before
   accepting the update. The installer waits for EntoCAD to exit, builds and
   replaces the installed copy, then opens the app with your saved drawing.

Checks run in the background and compare the running version with the local
checkout. Builds from Git also detect a changed commit at the same version.
**Later** hides that version/revision until another change or a manual check.
EntoCAD never pulls Git automatically. Keep the checkout at its original path;
if you move it, run its installer once to update the recorded location.
Build or replacement failures preserve the previous installed app and write a
log. Windows shows build progress in a console; Linux/macOS write progress to
the log. An update can take several minutes before the app reopens.

Close EntoCAD before a manual Windows reinstall. Linux/macOS can replace their
installed files while an existing window remains open, but that window keeps
running its old build until you close and reopen it.

Installer logs are written to:

- Windows: `%LOCALAPPDATA%\EntoCAD\logs`
- Linux: `$XDG_STATE_HOME/entocad`, or `~/.local/state/entocad`
- macOS: `~/Library/Logs/EntoCAD`

Manual installs append `install.log` (`install-windows.log` on Windows).
In-app updates use timestamped `update-*.log` files; on Linux/macOS a sibling
`.startup.log` also captures shell startup errors. If an update fails, reopen
EntoCAD from its usual icon, read the log, fix the reported build prerequisite,
and run the installer again. The old `packaging/` entry points remain forwarding
wrappers so installations from earlier releases can still update.

### Uninstalling

Drawings outside the installed application and your preferences are retained.

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\install-windows.ps1 -Uninstall
```

```bash
bash scripts/install-linux.sh --uninstall
bash scripts/install-macos.sh --uninstall
```

Windows also lists EntoCAD under **Settings → Apps**. Linux supports `--prefix`
and `--data-dir`, and macOS supports `--install-dir`, for custom installation
locations; use the same options when uninstalling. In-app updates remember
these locations. See each script's `--help` (Windows `-Help`) for options.
Headless `--import-only` remains a terminal command, for example
`cargo run -p mycad -- --import-only drawing.dwg`.

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

**Window does not open.** If EntoCAD prints `EntoCAD failed to start`, update the
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

AutoCAD is not required to build EntoCAD. To check that a saved DXF or DWG passes an audit, install the free [ODA File Converter](https://www.opendesign.com/guestfiles/oda_file_converter) and point the test at it:

```powershell
$env:MYCAD_ODA_FILE_CONVERTER = "C:\Program Files\ODA\ODAFileConverter\ODAFileConverter.exe"
cargo test -p dwg-import --test reference_dwg oda_file_converter_audits_export
```

The test is skipped when that variable is unset. It converts the written DXF and DWG to AutoCAD 2018 with audit on, and fails if ODA writes an `.err` file.

## Usage

- **File → New** (`Ctrl+N`) starts an empty drawing in millimetres on layer 0. If the current drawing has unsaved edits, EntoCAD asks before replacing it. The first save uses Save As.
- **File → Open** (`Ctrl+O`) to pick a DWG. If `Plant.dwg.mycad` sits beside it, EntoCAD loads the dynamic blocks stored there. Opening the companion file itself opens the DWG.
- **File → Save** (`Ctrl+S`) overwrites a previously saved DXF in place. An opened DWG always goes through Save As with a `*-EntoCAD.dwg` copy name so the original file is not overwritten. A compact Save icon on the menu bar (tooltip **Save** / **Ctrl+S**) runs the same command. Successful saves report in the status bar (`Saved Plant.dxf`, `Saved Plant.dwg`, or `DWG saved with 3 compatibility warnings`) without a dialog. When the original drawing had classes that were not carried over, the status bar also names how many and points at Diagnostics.
- **File → Save As…** (`Ctrl+Shift+S`) writes AutoCAD 2000 DXF or **DWG AutoCAD 2000**. DWG save goes through the DXF writer, then LibreDWG. The DWG stays a normal drawing: dynamic blocks are written as ordinary static blocks, so AutoCAD opens them without EntoCAD. EntoCAD also writes `Plant.dwg.mycad` next to the drawing and uses it to restore those dynamic blocks on the next open. If the drawing changes outside EntoCAD, its previous companion is not applied. Older `.mycad` drawings still open; new drawings are not saved in that format. Paper space and extra sheets are written as layouts, including VIEWPORT entities and the sheet size. Block attributes, text styles, layer lock/plot/lineweight, hatch pattern names, dimensions, external-reference paths, and complex linetype shapes are written with the drawing. TrueType font names are saved; the viewport still draws the stroke font. An external reference is not loaded. An image or wipeout is drawn as its frame and saved as a closed polyline, because a real IMAGE object does not survive the DWG writer. If the drawing has entities EntoCAD cannot fully keep, or a complex linetype that could not be stored, a prompt lists them and asks to **Save a Copy**. Unreadable classes and lost AutoCAD dynamic controls are also listed in the save warning. The original file is never overwritten. The copy is named `*-EntoCAD`.
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

Non-graphical data LibreDWG cannot read, such as associative arrays and dynamic-block parameters, is listed in Diagnostics and the save warning and is not written to the copy.

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
