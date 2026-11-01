# AutoCAD compatibility contract and implementation plan

Status: proposed architecture based on the company-library requirements. The
user confirmed that original AutoCAD dynamic controls must remain editable after
saving in EntoCAD. The user selected a fully open-source implementation. Continue with LibreDWG
and open-source dependencies; commercial SDK adoption is outside this plan. This
document records the target behavior and the audited gaps; it does not claim
that the current implementation meets this contract.

## Required behavior

The DWG is the authoritative drawing. A user must be able to open an AutoCAD
drawing in EntoCAD, use supported editing operations, save it, and reopen it in
the company's AutoCAD versions without unintended loss of appearance, geometry,
block definitions, attributes, layouts, references, or native behavior.

Compatibility means both visual fidelity and editable data fidelity. Matching a
screenshot or successfully opening a file is insufficient. Byte-identical output
is not required after editing; preserving the drawing's intended semantics is.

| Content | Authoritative storage | Without an EntoCAD helper |
| --- | --- | --- |
| Ordinary drawing geometry, blocks, attributes and layouts | DWG | Remains usable in both applications |
| Existing AutoCAD dynamic definitions, parameter/action graphs and instance values | DWG | Remains native AutoCAD dynamic content; EntoCAD exposes only operations it can support faithfully |
| EntoCAD-specific block behavior and editor metadata | Optional hidden companion | Its evaluated geometry remains ordinary usable DWG blocks |
| Unsupported native or third-party DWG data | Original DWG database | Preserved through unrelated edits where the selected backend demonstrably supports preservation |

An AutoCAD dynamic block must not silently become a static block on normal Save.
An unsupported editing operation must be explained and restricted rather than
flattening native data implicitly. If the backend cannot safely preserve a
drawing, normal Save must not be presented as a fidelity-preserving operation.
An explicitly requested exchange export may flatten content, with a concrete
loss report and a separate destination that preserves the original.

## Optional hidden companion

The companion contains only EntoCAD extensions and the identifiers needed to
associate them with the drawing. It must never be the only copy of normal DWG
geometry, nor the only place storing an original AutoCAD dynamic definition.

- Opening a DWG automatically looks for its companion. Missing, corrupt,
  unsupported-version or mismatched helpers leave the DWG usable.
- Extensions may be restored only when the application can establish that they
  correspond to the current drawing and affected blocks. A helper must not undo
  edits made in AutoCAD or recreate intentionally deleted definitions.
- Bind new helpers to the exact successfully written drawing content and a
  schema/generation identifier. Initially, a drawing-content mismatch disables
  automatic restoration conservatively. More selective restoration requires a
  tested semantic comparison of geometry, text, properties, nested definitions
  and relevant dependencies; entity counts and bounds are not enough.
- Stage and validate drawing and helper outputs before replacement. Since two
  independent file replacements are not one atomic transaction, use generation
  matching and recovery rules so an interrupted save cannot activate an old
  helper against a new drawing. A helper-write failure must remain visible and
  must not mark app-only changes safely saved.
- Failed evaluation of app-native dynamic geometry aborts the save before
  replacing either destination.
- Hide newly written helpers using platform-appropriate conventions. Preserve
  discovery of existing `Plant.dwg.mycad` files during migration. Hiding is a
  file-management convenience, not access control.
- Rename, copy and Save As must account for the companion. Copying only the DWG
  must still produce a useful drawing; copying the validated pair retains the
  EntoCAD extensions.

## Audited implementation gaps

| Current implementation | Consequence |
| --- | --- |
| `dwg-import/src/lib.rs` converts the imported database to `cad-core::Document` and frees the original graph | Unrepresented native data cannot pass through unchanged |
| `dwg-import/src/export.rs` writes a new DXF and converts it to R2000/R2004 DWG | This is an exchange path, not native-database preservation |
| `cad-core/src/evaluate.rs` materializes dynamic blocks and clears their dynamic definitions for export | The companion restores EntoCAD controls, not AutoCAD's original controls |
| `dwg-import/src/acad_dynamic.rs` maps a subset of native parameters/actions | Visibility, linear, flip, rotation and point support does not establish support for lookup, array, scale, polar, XY or alignment behavior |
| `cad-core/src/fingerprint.rs` uses entity count and 2D bounds for companion matching | Same-bounds changes to geometry, text or properties can be overwritten on restoration |
| `mycad/src/app.rs::export_snapshot` replaces evaluation errors with a fallback document | Save can proceed without the intended evaluated geometry |
| `cad-core/src/document.rs::has_save_loss` excludes unhandled object classes | The current save warning does not represent every preservation risk |
| `cad-io/src/companion.rs` creates `Plant.dwg.mycad` without hiding it | Hidden-helper behavior is not implemented |

Other conversions also need explicit fidelity accounting: IMAGE/WIPEOUT frames,
MLINE, MULTILEADER, proxy/custom objects, references, fonts and layouts. A file
must not be classified as safe merely because all visible entities were read.

## Implemented safety milestone (0.56.1)

- Companion schema 2 binds metadata to SHA-256 of the actual saved drawing bytes.
  Unbound schema 1 helpers and changed drawings are left unmodified on open, with
  a warning; old helper files are not automatically migrated by trusting weak
  fingerprints. Even a harmless external rewrite requires revalidation before
  extensions can be restored.
- Dynamic evaluation errors abort export before replacing drawing/helper files.
- Save warnings include native AutoCAD dynamic controls and unreadable classes.
  Lossy copies must not overwrite their source or filesystem aliases.
- Linux icon-build gating and font discovery are corrected so the open-source
  development workflow can run. Missing proprietary fonts remain explicitly
  reported and use the existing stroke fallback.

This milestone does not implement native AutoCAD dynamic-block preservation,
hidden-helper migration, or a complete two-file transaction. The gap table above
records the audit baseline; the implemented safeguards supersede its helper,
error-fallback and warning entries.

## Backend decision

Introduce a DWG document/session boundary which owns the native drawing database
through open, edit and save. `cad-core` remains the view/edit representation, but
changes must be applied to the native database with stable object identity.
Saving must not reconstruct the entire drawing solely from the 2D model.

The selected route is to investigate native-database preservation using
LibreDWG and open-source code against the company library corpus. Commercial
options below are retained only as the earlier comparison, not planned work. Do not assume that buying a DWG SDK supplies every dynamic-block
evaluation or authoring operation. Verify read, preserve, evaluate and edit
capabilities separately, including required modules and redistribution terms.
The existing GPL-3.0-or-later/LibreDWG licensing also needs review before combining
or distributing a proprietary backend; a process boundary alone is not proof of
license compatibility.

Keep the current LibreDWG path available as an explicitly scoped exchange path
where useful. If the project must remain entirely open source, investigate what
can be preserved with that backend and publish its tested limits. Do not promise
arbitrary lossless AutoCAD round-tripping from the current implementation.

Confirmed requirement: native AutoCAD controls must survive edits. No commercial
SDK has been selected, downloaded or installed, and no purchase or licensing
commitment has been made. The exact AutoCAD versions and company-required block
behaviors must be included in the qualification corpus before a compatibility
claim is made.

### What a commercial SDK would mean

A commercial SDK supplies a vendor-controlled DWG implementation under contract.
It does not automatically supply AutoCAD's full editor or dynamic-block engine.
Read/preserve, evaluate existing parameters, edit parameter values, and author
new native definitions must each be verified. Existing AutoCAD seats should not
be assumed to include SDK development or application redistribution rights.

| Route | Open-source implications | Main tradeoff |
| --- | --- | --- |
| Continue with LibreDWG and extend native-database preservation | Can retain the current GPL open-source approach | Highest implementation and interoperability-validation uncertainty; unsupported native objects remain a limiting factor |
| Evaluate ODA Drawings SDK or Autodesk RealDWG | Introduces a proprietary dependency; an SDK-dependent edition is not fully open source | May reduce DWG engineering effort, but requires verified capabilities, supported platforms and license compatibility |
| Evaluate an integration running with the company's licensed AutoCAD installations | An integration may have open source code, but the complete workflow still requires proprietary AutoCAD | Uses the target application's facilities; requires deployment and automation terms to be checked and does not provide a standalone open-source CAD application |

Before selecting a commercial option, obtain current vendor terms for fees,
renewals, supported platforms, redistribution, deployment and any optional
modules needed for dynamic blocks. Exact prices and product entitlements are not
verified in this assessment. Review the existing GPL obligations and copyright
ownership before considering a combined distribution or any relicensing. Making
an adapter optional or putting it in another process does not by itself resolve
those questions.

Selected sequence: define the preservation interface and acceptance corpus,
then prove native preservation using LibreDWG. Unsupported data or operations
must remain explicitly restricted. Do not add proprietary dependencies.

## Implementation order

1. Protect current users: accurate loss accounting, abort failed materialization,
   robust companion binding, and no silent restoration over outside edits.
2. Build a capability matrix and acceptance corpus from existing company
   drawings and libraries, including native dynamic blocks in multiple states.
3. Prove LibreDWG native-database preservation with unchanged open/save and unrelated
   edits, then prove native dynamic-control behavior in real AutoCAD.
4. Integrate the open-source native session and edit mapping. Preserve unsupported
   data and restrict operations for which preservation is unproven.
5. Add the hidden-companion migration and recovery workflow, then qualify the
   combined workflow against the supported AutoCAD versions.

## Acceptance checks

- AutoCAD → EntoCAD → AutoCAD: compare geometry, appearance, units, layer/style
  properties, layouts, attributes, references and native dynamic controls.
- Repeat with an unrelated edit. Verify untouched native and custom data remains
  intact rather than being regenerated as a reduced drawing.
- Exercise supported dynamic controls in EntoCAD, save, and verify both their
  values and continued editability in AutoCAD. Exercise unsupported cases and
  verify preservation with a clear editing restriction.
- EntoCAD-native dynamic block → DWG plus helper: AutoCAD opens its evaluated
  geometry without a plugin; EntoCAD restores extensions from a validated pair.
- Delete or corrupt the helper: the DWG still opens with its normal geometry.
- Modify text, interior vertices, layer/color/Z, nested blocks and definitions in
  AutoCAD while retaining entity counts and bounds: stale helper data must not
  override those edits. Test deleted definitions and unrelated drawings too.
- Simulate evaluation failure, write failure and interruption between the two
  file replacements. Verify destination preservation, recovery and dirty state.
- Audit exports in actual supported AutoCAD versions and check for recovery,
  erased objects and changed dynamic controls. An ODA or LibreDWG self-round-trip
  is useful additional evidence, not a substitute for the target application.

Current tests already cover focused geometry round-trips and companion
restoration, and optional AutoCAD/ODA audits exist. Those external audits return
early when their tools are absent, so they must be reported as unexecuted. The Linux build-script and font-discovery issues found during onboarding are
fixed in the first safety milestone; large-drawing rendering still needs
separate performance qualification.
