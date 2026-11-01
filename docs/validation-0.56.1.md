# Validation of the open-source safety milestone

Validated on the managed Debian Linux environment with Rust 1.98.1.

## Build and tests

- `cargo build -p mycad --locked --offline`: passed without compiler warnings.
- The reusable install script now builds the entire workspace and passed.
- `cargo test --workspace --locked --offline --no-fail-fast`: completed. It
  reported 782 passing cases and one failure: the reference import test still
  asserted that unread native classes must not trigger a save warning.
- Updated that assertion to require the new warning and reran it using
  `cargo test --workspace --locked --offline reference_dwg_imports_without_panic`:
  passed. All other tests had passed in the full run. The entire full command was
  not repeated after this assertion-only correction.
- Two of the runner's passing cases are optional ODA/AutoCAD audit functions
  that return early when their tools are absent. Those external audits were
  **not executed**. Excluding them, 781 distinct tests passed across the full run
  and corrected-test rerun. Doc-test targets contained zero tests.
- All 281 application tests and all 12 save-worker tests passed. Companion tests
  include matching drawing bytes, same-size outside edits, legacy/unbound data,
  and actual DXF reimport with changed color and unchanged geometry bounds.
- `reference_dwg_save_reimports` and the complete sample-set dynamic-block check
  passed. The reference group took 542 seconds; it was CPU-heavy, not an
  indefinitely hung process. A late stack sample was in post-import entity-ID
  assignment; this single sample does not attribute the entire runtime.

## Functional checks and limits

A temporary two-line fixture derived from a bundled sample was written to DWG,
reimported with both entities intact, opened in the native GUI on Xvfb/Mesa GL,
and rendered. Headless import and `--perf-baseline` completed for that fixture;
DWG, DXF and PDF export completed, and interaction/edit samples were below 16 ms.
This tiny fixture is only a functional smoke check, not a company-library
performance qualification.

The original 103,016-entity sample imported but the GUI continued preparing its
geometry for several minutes with substantial memory use. That GUI check was
stopped without claiming successful large-drawing rendering. A concurrent
headless check of the same large drawing was also stopped to limit memory use.

No real AutoCAD interoperability audit was possible in this environment. Native
AutoCAD dynamic definitions are still flattened by the current exchange writer;
the new warnings and source protection make that limitation explicit. Hidden
helper naming and native DWG database preservation remain future milestones.

The new helper schema deliberately rejects unbound legacy helpers and any
externally changed drawing. Preserve original drawing/helper pairs: this release
does not infer or silently migrate a legacy helper's relationship to a drawing.
