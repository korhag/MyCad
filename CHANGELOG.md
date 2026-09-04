# Changelog

All notable changes to MyCad are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project versions with [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Workspace version is `0.3.0` (`Cargo.toml`).

## [Unreleased]

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
- Replace the Home icon-tile strip with an adaptive ribbon: horizontal icon+text commands, height-based Micro/Compact/Normal/Expanded density, width-based group overflow instead of a horizontal scrollbar, content-sized controls, and a persistent Layer chip that stays visible as the window shrinks.
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
