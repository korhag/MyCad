# Validation of the second performance and robustness pass

Environment: managed Debian Linux with Rust 1.98.1. The implementation remains
fully open source.

## Build, regression and interaction checks

`cargo test --workspace --locked --offline --no-fail-fast` completed without
failures or compiler warnings. It reported 840 passing tests, including the
reference-DWG corpus and DWG/DXF round-trip tests. Two optional external-audit
tests return early when AutoCAD/ODA tooling is unavailable; those tools were not
available in this environment.

The GUI check then exposed a Test-mode preview-routing bug. After correcting it,
the complete affected suites passed again: 278 core, 313 app and 71 renderer
tests. Four new regressions cover occurrence isolation, nested previews, invalid
paths, and agreement between rendering, snapping and measurements. The import
corpus was unchanged and was not repeated after this preview-only correction.
Together these runs cover 844 currently reported passing tests, of which 842
executed without the optional external tools.

The final `cargo build -p mycad --locked --offline` completed without warnings.
`git diff --check` passed. A Linux GUI smoke test verified angle-parameter
creation, required-pivot validation, a retained draft after invalid input,
successful Rotate creation, visible 90° and 180° Test-mode previews, and reset
to the original geometry. Other block occurrences remained unchanged.

A final 30-entity DWG performance smoke measured worker-thread import at
77.76 ms, tessellation at 0.14 ms, snap-index construction at 0.12 ms and edit
refresh at 0.22 ms. These small-fixture timings establish that the exercised
paths work; they do not measure responsiveness on large drawings.

## Controlled development-build benchmarks

These are reproducible CPU workloads, not viewport frame-rate claims. Setup
and correctness assertions are outside the timed sections. The corresponding
examples are in `crates/cad-core/examples/`.

| Workload | Before | After | Example |
| --- | --- | --- | --- |
| Dependency collection/evaluation through 12 shared definitions, three references per level | 271–279 ms | 0.08–0.14 ms | `dynamic_dependencies_benchmark` |
| Flip plus visibility affecting 10,000 lines | 368–376 ms | 16–21 ms | `dynamic_actions_benchmark` |
| Ordinary append in 100,000 model entities plus 200,000 block members | 7.46–7.56 ms per edit | 1.87–1.98 ms per edit | `entity_edit_benchmark` |
| Tail removal in the same drawing | 5.53–5.60 ms per edit | 0.001 ms per edit | `entity_edit_benchmark` |

For example:

```sh
cargo run -p cad-core --example entity_edit_benchmark --offline
```

Dependency traversal now revisits a shared definition only when a shallower path
exposes more descendants. This preserves traversal limits and dependency-based
cache invalidation. Entity indexing touches only the changed space's suffix;
the append measurement still includes the existing drawing-order scan.

The original 768,200-entity reference DWG imported in about 14.9 seconds before
these changes. The much longer acceptance run was dominated by a 218 MB generated
DXF: a baseline import took 175.7 seconds, mostly in LibreDWG's parser. The earlier
single stack sample near ID assignment was insufficient to identify that cost.
An experimental parser change did not show a repeatable improvement: the full
file took 184.4 seconds in a run overlapping compilation, and alternating smaller
fixture trials were inconsistent. That change was removed. Large DXF parsing
remains a known bottleneck; no improvement is claimed for it in this release.

## Scope and limitations

The authoring changes expose existing native rotation, reflection, placement,
visibility and text behavior; they do not add native scale or array actions.
The numerical benchmarks do not establish a frame budget for every company
drawing. Native AutoCAD control-graph preservation remains unsupported by the
exchange writer, and no independent AutoCAD/ODA audit is available here.
