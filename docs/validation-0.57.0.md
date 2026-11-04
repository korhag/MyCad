# Validation of authoring, libraries and exchange improvements

Environment: managed Debian Linux, Rust 1.98.1, fully open-source dependencies.

## Build and tests

- `cargo build -p mycad --locked --offline` passed with zero compiler warnings.
- `cargo test --workspace --locked --offline --no-fail-fast` completed with
  809 reported passes and one failure: the reference export test still compared
  raw source counts against output that now includes evaluated static block
  definitions. All application tests (298), the sample-library dynamic checks,
  and the 16 round-trip integration tests passed.
- Corrected that test's expected counts to retain every original diagnostic
  count and add the exact generated-definition inventory. It also checks that
  original containers retain their inventories, and compares LWPOLYLINE counts,
  closed flags and bounds against the materialized drawing. Attached attribute
  totals are checked separately. No entity types are ignored.
- The new counting-helper regression passed. The corrected reference test passed
  via `cargo test --workspace reference_dwg_save_reimports -- --nocapture` in
  426.53 seconds, with zero compiler warnings. The full command was not repeated
  after this test-only correction; production code remained unchanged.
- Across the full run and corrected follow-up runs, 809 distinct tests executed
  successfully. Two additional reported passes are optional AutoCAD/ODA audits
  that return early when their external tools are unavailable; those audits did
  not execute. Doc-test targets contained no tests.

## GUI and interaction checks

A temporary native fixture with two differently configured dynamic frames was
opened using Xvfb and Mesa GL. The check entered the dynamic block editor,
created a size by picking points, enabled a minimum while leaving the maximum
unbounded, reviewed the settings, and confirmed that Create closed the wizard
and selected the new parameter. The library controls rendered clearly. An
attempt to open the native folder picker could not proceed because this
headless session has no desktop D-Bus/portal session. Library save, insert and
undo have automated coverage, but were not exercised end to end through the
native folder picker.

`--perf-baseline` completed on the tiny derived DWG fixture. Interaction and
derived-data operations measured 0.00–0.24 ms; exports measured 0.21–1.48 ms.
Cold import took 70.90 ms on its worker. This final baseline includes the native
attribute-link and alignment fixes. The authoring GUI check preceded those
backend-only fixes. This fixture does not qualify large company drawings for
the interaction budget.

## Performance measurement

The reproducible workload in
`crates/cad-core/examples/dynamic_materialize_benchmark.rs` materializes 1,000
identically configured instances of a dynamic block containing 2,000 lines.
One Move action changes a line, and assertions verify both the moved geometry
and sharing of the generated definition.

In the same development build configuration, three trials of the previously
committed implementation took 684, 679 and 675 ms. The new implementation took
6.54, 2.92 and 2.85 ms. The first trial starts with an empty evaluation cache;
the later trials reuse it. These timings cover materialization only, including
document cloning. They do not measure import, rendering, frame rate, or general
large-drawing responsiveness.

Run the committed example with:

```sh
cargo run -p cad-core --example dynamic_materialize_benchmark --offline
```

## Compatibility limits

The export regression uses different dynamic configurations, nested blocks,
rotation, nonuniform scaling, attributes and geometry bounds. It reads written
DXF/DWG through LibreDWG without applying a companion. This checks this writer
and reader together; it is not independent validation in AutoCAD.

No real AutoCAD or ODA audit tool is available in this environment. Native
AutoCAD dynamic control graphs are still flattened by the exchange writer;
the original-file protection and loss warnings remain necessary. The Windows
hidden-attribute implementation cannot be executed on this Linux host.
