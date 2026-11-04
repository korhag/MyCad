# Validation of the five GUI workflow improvements

Environment: managed Debian Linux, Rust 1.98.1, and an isolated 1440×1000 Xvfb
session using software GL. No user drawing or preferences were changed during
acceptance. The implementation remains fully open source with no new dependency.

## Build and regression coverage

The final `cargo build -p mycad --locked --offline` completed without compiler
warnings. `git diff --check` passed.

Full workspace validation command:

```sh
cargo test --workspace --locked --offline --no-fail-fast
```

This run passed all core, I/O, renderer, viewport and import suites, including
all 11 reference-DWG tests and 17 round-trip tests. One app assertion still
expected an internal stale-preview status message that the GUI now suppresses.
After updating that assertion to check preservation of the useful status,
`cargo test --workspace --locked --offline --no-run` compiled without warnings
and the complete app suite passed: **350 tests, zero failures**. No production
code changed after the last successful GUI retest. Across these runs, all 884
reported tests pass; two optional external-tool audits return early, as described
below, so 882 tests executed without those tools.

The added regressions exercise atomic Properties edits and Undo/Redo, mixed
values and invalid input, preservation of configured dynamic geometry and snap
and measurement indices, source-only authoring geometry and nested occurrence
isolation, action metadata editing and cache invalidation, temporary Test-mode
boundaries, staged library placement and stale results, helper verification,
dock navigation, background block-tree refresh, and clearing removed selections.

The Save As review was exercised with actual egui button clicks: Cancel left the
drawing and filesystem unchanged; Save this copy wrote a DXF that was reimported
and checked for the expected point geometry. A stale save completion also cannot
replace the current drawing's helper status.

## GUI walkthrough

- Edited a dynamic instance's Y coordinate from 60 to 80 while retaining its
  configured Width=200 appearance; Undo restored position and appearance.
- Opened the existing dynamic definition through the new Properties shortcut.
- Followed Parameter / Actions / Geometry / Test, edited and renamed a Stretch
  action, and checked source-geometry endpoint highlights at Width=100.
- Entered Test using the instance's Width=200, previewed a 90-degree rotation,
  and returned to source authoring. Drawing commands were blocked during Test.
- Browsed background thumbnails, searched a personal description, marked a
  favorite, and verified persistence after normal quit/relaunch.
- Snapped a library ghost to an endpoint, rotated with R, placed with one click,
  undid/redid the insertion, and canceled another placement with Escape.
- Verified the new block appears in the Blocks tree and compatibility summary;
  Undo removes it and clears its selection and library-save target.
- Followed compatibility links to Diagnostics and named block definitions.

The GUI log remained empty. All temporary GUI and Xvfb processes were stopped.
The native Save As file chooser could not be exercised in this headless session
because no desktop file-chooser portal was available. The subsequent save-review
and file-write path was covered by the integration test described above.

## Responsiveness smoke

A 30-entity DWG was used with `--perf-baseline`. Routine interaction, edit, index
lookup and clone samples were at most 0.27 ms; opening tessellation took 1.83 ms.
Worker import took 456.47 ms while the full reference tests were running.
These are small-fixture smoke results, not a comparative speedup or a guarantee
for large company drawings. Library previews are bounded and generated on
workers; the placed block retains complete geometry even if its ghost is
simplified. Dynamic metadata caching avoids repeated per-frame target copies.

## Scope and limits

The compatibility panel describes existing importer/writer capabilities; it does
not add native AutoCAD dynamic-control preservation. AutoCAD/ODA are unavailable
here, so the two optional external-audit tests return early without running those
tools. Helper verification records the last open/save, not continuous filesystem
monitoring.

Library descriptions are personal preferences, portable through settings rather
than embedded in shared assets. Tilted INSERT transforms remain read-only in
Properties; attached attributes restrict nonuniform/reflected scaling to avoid
text distortion. Linked-text actions use their target identity instead of a
stored action name. Placement editing selects existing anchors without moving
shared anchor definitions. Native scale and array actions remain outside the
supported action model.
