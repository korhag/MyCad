# Changelog

All notable changes to MyCad are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project versions with [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Workspace version is `0.1.0` (`Cargo.toml`).

## [Unreleased]

### Added

- Settings window (Settings → Preferences) with a persistent zoom-speed multiplier (slider and numeric value, 0.25×–10.0×). Default 1.0× keeps the original smooth wheel zoom. Apply saves the value and uses it immediately; Cancel discards the draft and leaves the current session unchanged.

### Fixed

- Sample LWPOLYLINE bulge arcs with Autodesk’s signed convention (`θ = 4·atan(bulge)`, positive = CCW). From a left-to-right chord, `+1` is the lower semicircle and `-1` the upper. Arcs are built in OCS, then each sample is mapped to WCS so negative-Z extrusion does not flip handedness.
- Evaluate SPLINE control points in homogeneous space (weight multiplied before de Boor, then divided). Nested `Kefe` splines far from the origin no longer emit giant diagonal fans across the drawing.
- Apply AutoCAD INSERT order `OCS · T(ins) · R · S · T(-base)` so nested blocks land on the plant instead of stretching into world-origin rays.

### Changed

- Ignore `/target/`, built binaries, reference `test-data/*.dwg`, and generated `MyCad-preview.*` so git tracks source rather than build artifacts.

## [0.1.0] - 2026-08-31

### Added

- Milestone 1 DWG viewer: `cad-core` document model, `dwg-import` via LibreDWG, wgpu tessellation in `cad-render`, pan/zoom viewport, and the `mycad` egui shell with `--import-only` headless preview.
