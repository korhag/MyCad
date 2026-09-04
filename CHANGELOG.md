# Changelog

All notable changes to MyCad are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project versions with [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Workspace version is `0.17.1` (`Cargo.toml`).

## [Unreleased]

## [0.17.1] - 2026-09-04

### Fixed

- Open large DWGs without crashing when linework exceeds one GPU vertex buffer. Capacity no longer rounds up past the device limit (the previous 384 MiB `mycad.linevb` allocation), and bigger meshes split across multiple buffers.

## [0.17.0] - 2026-09-04

### Changed

- Clicking another Draw, Modify, or Measure tool now cancels the current command (same cleanup as Esc) and starts the new one in one click. Clicking the already-active tool stays a no-op, and Esc still returns to Idle.
- Ignore the local `test-data/` directory so large PDFs and preview images stay off GitHub.

## [0.16.0] - 2026-09-04

### Added

- Save (Ctrl+S) now overwrites a DWG that MyCAD already wrote (`*-MyCad.dwg`, or the file chosen in this session). Imported third-party DWGs still open Save As so a production drawing is not replaced until fidelity coverage is broader.
- Extend DXF/DWG round-trip fixtures with POINT, POLYLINE, negative LWPOLYLINE bulge, non-zero Z, entity linetype scale, and `$INSUNITS` inches so interchange is checked beyond millimetre LINE/ARC/CIRCLE drawings.

### Changed

- Label unsaved-changes as **Don't Save**, and name Save As filters **DXF Drawing** and **DWG Drawing - AutoCAD 2000**.
- Report LibreDWG DWG write failures with the target path so `Save failed:` status text names the file.

## [0.15.0] - 2026-09-04

### Added

- Add a compact Save icon on the menu-bar quick-access strip (tooltip **Save** / **Ctrl+S**) so the Home ribbon stays a drawing toolbar. File still carries Open, Save, Save As, and Export PDF.

### Changed

- Report successful saves in the status bar as `Saved Plant.dxf`, `Saved Plant.dwg`, or `Exported Plant.pdf` without a dialog. A DWG with interchange fallbacks reports `DWG saved with 3 compatibility warnings`. Failures use `Save failed: …`.

## [0.14.0] - 2026-09-04

### Added

- Add round-trip tests that write a primitive-filled document to DXF, import it again, convert that DXF to DWG through LibreDWG, and compare coordinates, angles, radius, bulge, block transforms, entity counts, layers, linetypes, and extents instead of only checking that a file exists.
- Add `dwg_import::import_dxf` so DXF interchange uses the same process-wide LibreDWG lock and native convert path as DWG import.
- Add PDF export checks for a valid file, requested page size, non-zero vector operators, geometry inside the printable region, and omitted hidden or frozen layers.

### Fixed

- Write MTEXT rotation as the DXF 11 x-axis vector and omit group 72 so LibreDWG does not treat flow direction as a column count (`DWG_ERR_INVALIDDWG`).
- Write `$TDCREATE` / `$TDUCREATE` / `$TDUPDATE` / `$TDUUPDATE` as a real Julian day so LibreDWG's R2000 decoder does not call `strftime` on a zero calendar date and abort with `STATUS_STACK_BUFFER_OVERRUN` when the existing DWG importer reads a file we just encoded.
- Write named-block BLOCK_RECORD rows (not `*MODEL_SPACE`) so INSERT group 2 resolves to a block name, start DXF handles at `0x100` so written entities do not collide with LibreDWG's reserved `*MODEL_SPACE` handle (`0x1F`) and truncate the ENTITIES section, and keep ACI palette indexes when LibreDWG also fills a matching truecolor.
- Restore named INSERT definitions from BLOCK/ENDBLK sequences when LibreDWG does not create BLOCK_HEADER owned lists, fall back to the INSERT `block_name` text field, and keep SPLINE after other model-space types so a spline read error cannot drop TEXT, HATCH, and LEADER.
- Treat INSERT column/row counts of zero as 1×1 so a missing DXF array size does not look like an empty block array.

## [0.13.0] - 2026-09-04

### Added

- Add a vector PDF plot dialog (A4–A0, portrait/landscape, extents, fit to page, color or monochrome, 5/10/15 mm margins) that writes CAD geometry to the page. Hidden and frozen layers are omitted because only `layer.is_plottable()` content is plotted; the viewport, grid, selection, and OSNAP markers are not captured.

## [0.12.0] - 2026-09-04

### Changed

- Keep Ctrl+S from overwriting an imported DWG. Save opens a Save As dialog with a `*-MyCad.dwg` copy name so the original production file stays intact until DWG round-trip coverage is mature.
- Warn before any DXF or DWG save when `unsupported_total()` is greater than zero, and offer **Save a Copy** or **Cancel** instead of silently dropping layouts, proxies, XREFs, and other content the native model does not store.

## [0.11.0] - 2026-09-04

### Added

- Add File → Save As **DWG AutoCAD 2000**, written as Document → temporary DXF → LibreDWG `dxf_read_file` / `dwg_write_file` → temporary DWG → atomic replace. Save (Ctrl+S) overwrites an opened or previously saved DWG in place the same way.

### Changed

- Keep the DXF writer as the only Document → CAD mapping. DWG save reuses that interchange file instead of a second entity serializer. AutoCAD 2004 and later DWG versions stay out of the UI until round-trip validation exists.

## [0.10.0] - 2026-09-04

### Added

- Add File → Save (Ctrl+S) and File → Save As (Ctrl+Shift+S) for DXF, plus File → Export → PDF. New drawings and opened DWG files use Save As; a previously saved DXF overwrites in place and clears the dirty star.
- Write DXF and PDF through a same-directory temp file that replaces the target only after a complete flush, so a crash or serializer error cannot truncate the previous drawing.

### Changed

- Treat PDF as export only: it does not change `document.source_path` or call `history.mark_clean()`.
- Replace the unsaved-changes warning that MyCad cannot write DWG with Save / Don't save / Cancel, and bind File → Open to Ctrl+O.

## [0.9.0] - 2026-09-04

### Added

- Add File → Save As DXF to write the native document as AutoCAD 2000 (`AC1015`) through `cad-io`, including HEADER, LTYPE, LAYER, BLOCKS, and ENTITIES.
- Preserve coordinates, Z, layer, ByLayer/ByBlock/ACI/TrueColor, linetype, entity linetype scale, visibility, bulges, closed polylines, arcs, ellipses, splines, block insert/scale/rotation, TEXT/MTEXT, `$INSUNITS`, and `$LTSCALE` on DXF save.

### Changed

- Export DIMENSION as the visible anonymous-block geometry with a SaveReport warning instead of skipping it or inventing a DIMENSION entity. MLINE, INSERT attributes, ATTDEF, HATCH spline edges, and varying-Z LWPOLYLINE follow the same explode-and-warn fallback so geometry is never silently discarded.

## [0.8.0] - 2026-09-04

### Added

- Add a `cad-io` crate that writes native DXF and PDF from `cad-core::Document`, keeping serialization out of the app, ribbon, and renderer.
- Add `dwg-import::convert_dxf_to_dwg` so DXF-to-DWG conversion uses the existing LibreDWG process-wide mutex instead of a second FFI path.

## [0.7.0] - 2026-09-04

### Added

- Add Move, Copy, Rotate, Mirror, Scale, and Erase as one shared modification command with selection-first and command-first workflows.
- Add exact planar transforms in cad-core for LINE, POLYLINE, CIRCLE, ARC, POINT, ELLIPSE, SPLINE, SOLID, LEADER, MLINE, TEXT, MTEXT, and top-level INSERT instances, including live GPU preview without retessellating the drawing.
- Add a compact context-sensitive right-click menu with Repeat of the last completed command, one Modify submenu, and command-specific Confirm/Undo/Cancel actions.
- Add Delete-key erase while idle, using the existing rebindable input map and leaving numeric dynamic-input editing untouched.

### Changed

- Enable the Home Modify ribbon buttons and add a top-level Modify menu that dispatch to the same command handlers as the right-click menu.
- Keep Hatch, Dimension, and non-world OCS geometry out of Move/Copy/Rotate/Mirror/Scale by aborting the whole selection with a short type list instead of approximating.

## [0.6.1] - 2026-09-04

### Added

- Add Endpoint and Midpoint snaps from accepted Line and Polyline vertices before the command commits, merged with drawing snaps inside the same 9-pixel aperture.

### Fixed

- Close a Polyline by snapping to the first vertex, pressing C, or right-click Close without storing the start point twice, and keep explicit Close working when OSNAP is off.
- Keep Length and Angle dynamic-input fields on the snapped point unless a typed or locked value forbids that snap.

## [0.6.0] - 2026-09-04

### Added

- Add inspect-only Distance, Angle, Radius, and Area tools that never create Dimension entities, dirty the drawing, or enter Undo history.
- Add a shared double-precision measurement model so the viewport overlay, status prompt, Properties panel, and clipboard copy the same formatted values and `$INSUNITS` labels, including area units such as `mm²`.
- Add a semantic `MeasureIndex`, built once per drawing like object snaps, so Radius and Angle pick exact Circle, Arc, and straight segment geometry—including uniformly scaled nested INSERTs—instead of tessellated display paths.
- Add a transient amber measurement overlay with constant-pixel markers, a compact result card (Copy / Close), live Distance/Area previews, and a top-level Measure menu beside the Home ribbon commands.

### Changed

- Change Distance to keep OSNAP, live ΔX/ΔY/angle beside the cursor, a measurement line until the second click, and the finished result until Esc, Close, file load, or a document edit.
- Enable Angle, Radius, and Area on the Home ribbon while keeping the super-tight adaptive layout and content-sized controls.

### Fixed

- Reject non-uniformly scaled Circles as ellipses with a clear message instead of reporting a false radius.
- Reject open and self-intersecting Area boundaries with a short explanation, and keep clockwise and counterclockwise closed polylines, including bulge arcs, reporting the same area.

## [0.4.0] - 2026-09-04

### Added

- Add Polyline, Circle, Arc, and Rectangle drawing commands alongside LINE, so Basic Draw can create the geometry types the renderer already displays.
- Add compact Length/Angle, Radius, and Width/Height fields beside the cursor for exact numeric input, including Tab to lock a value and Enter to accept the point.
- Add a viewport right-click menu (rebindable, default unmodified right-click) with Finish/Undo/Close during Line and Polyline, Back/Cancel during Circle, Arc, and Rectangle, and Properties, layer, repeat, and Zoom Extents when idle.
- Add a tested `arc_from_three_points()` constructor so three-point arcs pass through the middle point for both clockwise and counterclockwise clicks.

### Changed

- Change drawing commands to share one `CommandState` / `CommandKind` foundation and a generic `commit_geometry` path that assigns the current layer, records one history edit, and appends display and snap data without rebuilding the drawing.
- Enable the Home ribbon and Draw menu entries for Polyline, Circle, Arc, and Rectangle, highlight the active tool, and keep a second drawing command from starting until the current one finishes or is canceled.

### Fixed

- Keep right-drag pan from opening the context menu, and keep a right-click from both opening the menu and placing a point.
- Pin `mycad` release `codegen-units` to 1 so Windows `cargo run --release` does not die in rustc with `STATUS_HEAP_CORRUPTION`.

## [0.3.0] - 2026-09-04

### Added

- Add an interactive LINE command with first/next-point prompts, live preview, repeated segments, Enter or right-click to finish, Esc to cancel, and one undoable transaction per invocation.
- Add persistent ORTHO (F8) and OSNAP (F3) drafting controls to the status bar, including coordinates and Shift temporary ORTHO reversal while a command requests a point.
- Add semantic Endpoint, Midpoint, and Center object snaps with fixed-screen-aperture markers and nested INSERT transforms, avoiding false snaps from tessellated curve samples.
- Add a dockable Home ribbon above the viewport with History, Draw, Modify, Measure, and current-layer controls. LINE and Distance start with one click; later drawing, modify, and measurement tools stay visible but disabled until their releases.
- Add a one-time dock-layout migration that places Home above the viewport for layouts saved before the toolbar existed, without reopening Home after the user closes it.
- Add Undo/Redo (Ctrl+Z / Ctrl+Y) so accepted LINE segments can be reversed as a single command.
- Add a Distance measurement command that reports length, ΔX, ΔY, and angle without modifying the drawing, using imported $INSUNITS when available.
- Add stable `EntityId` values, current-layer creation defaults from DWG `$CLAYER`, and `$INSUNITS` import so new geometry inherits ByLayer properties and measurements can name millimetres or inches.
- Add a dirty indicator in the window title and status bar, with discard confirmation on Open or Quit. Editing stays in memory and never overwrites the source DWG.
- Add a Cursor rule that work is not finished until `cargo` reports no errors and no warnings.

### Changed

- Change LINE commits to append tessellation, object snaps, extents, and GPU vertices for the new segment so drawing stays interactive on large DWGs. Undo, redo, and file load still rebuild the full display.
- Replace the Home icon-tile strip with an adaptive ribbon: horizontal icon+text commands, a compact desktop density that does not grow with unused Home height, width-based group overflow instead of a dock scrollbar, content-sized controls, and a persistent Layer chip.
- Size the default Home split from a ~50 px leaf height instead of a window-percentage, lower the dock splitter floor so Home can shrink to about 42 px, and keep the separator easy to grab.
- Fix blank Home ribbons after resizing by using one stable dock-body height for responsive layout and recovering oversized saved splits once.
- Mark a cleared undo history as the clean baseline so loaded drawings are not dirty and release builds no longer warn about unused `mark_clean`.

## [0.2.1] - 2026-09-03

### Fixed

- Ignore dock splitter drags in the viewport so resizing Properties or Diagnostics no longer draws or commits a selection box.

## [0.2.0] - 2026-09-03

### Added

- Settings window (Settings → Preferences) with Viewport, Display, and Shortcuts tabs. Zoom-speed multiplier (0.25×–10.0×, default 1.0×) keeps the original smooth wheel zoom; Apply saves immediately and Cancel discards the draft.
- Dockable workspace: Viewport, Properties, and Diagnostics are `egui_dock` tabs that can split, float, collapse, close, and restore from **View**. Layout persists in settings JSON.
- Click selection of top-level entities, including nested `INSERT` geometry selecting the parent block, with a Properties inspector for the current set.
- AutoCAD-style box selection: left-to-right window (fully inside), right-to-left crossing (any touch), live candidate highlight, rubber-band colors on the Display tab, and Ctrl/Shift toggle. Esc cancels an in-progress box without changing the committed selection.

### Changed

- Ignore `/target/`, built binaries, reference `test-data/*.dwg`, and generated `MyCad-preview.*` so git tracks source rather than build artifacts.
- On Windows, compile `libredwg-sys` at `opt-level = 1` in release so MSVC 14.41 does not ICE on LibreDWG `decode.c`.
- Draw committed and live-preview highlights as GPU overlay passes over the existing CAD buffers instead of tessellating every edge through egui.
- Require a changelog version section and a `Cargo.toml` version bump for every git-bound product change (`.cursor/rules/changelog.mdc`).

### Fixed

- Sample LWPOLYLINE bulge arcs with Autodesk’s signed convention (`θ = 4·atan(bulge)`, positive = CCW). From a left-to-right chord, `+1` is the lower semicircle and `-1` the upper. Arcs are built in OCS, then each sample is mapped to WCS so negative-Z extrusion does not flip handedness.
- Evaluate SPLINE control points in homogeneous space (weight multiplied before de Boor, then divided). Nested `Kefe` splines far from the origin no longer emit giant diagonal fans across the drawing.
- Apply AutoCAD INSERT order `OCS · T(ins) · R · S · T(-base)` so nested blocks land on the plant instead of stretching into world-origin rays.
- Paint selection as independent line pairs so block highlights no longer grow giant connector spikes at far zoom.
- Restore left-drag box select for saved shortcut maps that only stored left-click Select; click and drag share the same button.
- Stop area-select lag and the ~318 MB `egui_vertex_buffer` panic by overlay-batching GPU ranges, indexing pick bounds, and recomputing box candidates only when the pointer moves.
- Fix startup wgpu validation (`uniform buffer 96 bytes, shader expected 112`) caused by WGSL `vec3` padding on overlay uniforms, so the empty File → Open window opens without a crash.

## [0.1.0] - 2026-08-31

### Added

- Milestone 1 DWG viewer: `cad-core` document model, `dwg-import` via LibreDWG, wgpu tessellation in `cad-render`, pan/zoom viewport, and the `mycad` egui shell with `--import-only` headless preview.
