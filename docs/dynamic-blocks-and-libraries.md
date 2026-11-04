# Dynamic blocks and reusable libraries

EntoCAD keeps dynamic behavior in its native files and in a companion beside a
saved DWG/DXF. The exchange drawing contains the evaluated block geometry, so it
can be opened without that companion. Development remains fully open source.

## Choose what a parameter controls

A native dynamic block can combine geometry changes, visibility, labels, and
inputs to nested blocks. The authoring dock exposes these existing actions:

| Action | Typical input | Result |
| --- | --- | --- |
| Stretch | Size or numeric offset | Moves selected endpoints or vertices |
| Move | Size or numeric offset | Moves complete selected objects |
| Rotate | Angle or Choice | Rotates selected geometry about a picked pivot |
| Flip / mirror | On/Off or Choice | Reflects selected geometry across a picked axis |
| Visibility | On/Off or Choice | Shows or hides selected parts |
| Placement | Choice | Places a selected group at named anchors |
| Linked text | Text, number, Choice, or On/Off | Updates a label from parameter values |
| Nested inputs | Parent parameter | Drives a parameter on a specific nested block |

The dock guides you through **Parameter → Actions → Geometry → Test**. Select a
parameter, choose an action, and supply the geometry and reference points that
action needs. In **Actions**, use **Rotate…**, **Flip / mirror…**, **Position by
option…**, **Show / hide…**, or **Link text…**. Position destinations use block
coordinates in the drawing's units.

Connected actions offer consistent editing, target selection, duplication and
removal. Selecting an action highlights its objects and its pivot, axis or
attachment. Stretch target editing retains endpoint-level choices. Existing
option mappings are retained when editing another action setting.

Use **Add angle parameter** for a rotation angle. Its value is stored in degrees,
and its minimum and maximum can be left unrestricted or set in the parameter
editor. Pick the geometry first, start the action, then supply its pivot, axis,
or anchors. Missing or invalid settings keep the setup open with an explanation.

Named presets store useful combinations of values. Compatibility rules constrain
which options can be combined. Test the complete configuration before saving a
block, especially when several actions affect the same geometry.
Test values are temporary previews for the edited occurrence. Test mode hides
authoring controls and blocks source-geometry edits; navigation and measurements
remain available. **Return to authoring** restores the definition view. Change a
parameter's default or a placed instance's configuration to retain a value
outside Test mode.

Author mode displays the source definition geometry so targets and reference
points align even when a placed instance has different values or hidden parts.
Other occurrences keep their evaluated appearance. Test starts from the edited
instance's values. After returning to Author, **Keep the last test configuration**
offers explicit actions to retain those values as shared defaults or a preset.

The scope banner distinguishes the shared definition from a temporary preview.
**Save definition** keeps the definition changes in the current drawing; save the
drawing separately to write them to disk. Discarding definition changes restores
the definition and closes its authoring session.

The current nested-input shortcut maps to the child's first parameter. The
compatibility-rule shortcut starts from the first Choice/numeric pair; it is
not a general rule editor. Native scale and array actions are not implemented.

## Create a size that changes geometry

1. Create a block from selected geometry, or select an existing block reference.
   Use **Create a Dynamic Block…** to open its authoring controls.
2. Choose **Add size parameter…**, name it (for example, Width), and pick two
   reference points in the block. Choose the measurement direction and which
   side stays fixed.
3. Leave **Minimum** and **Maximum** unchecked for unrestricted values, or enable
   either limit. Alternatively, choose a list of available sizes. The measured
   size is the starting value; adding the parameter preserves the current shape.
4. Confirm, then use **Choose what this parameter changes**. **Stretch** moves
   selected endpoints; choose them individually or draw a selection window.
   **Move** shifts complete selected objects. Choose the first point, second
   point, or center according to the part of the size those objects follow.
5. Enter **Test mode**, try several values, and inspect the result. Return to
   authoring to adjust it, then **Save** the block. Save the drawing separately.

Select a parameter later to change its bounds or connected actions. Choice,
On/Off, and Text parameters are available for other behavior. A numeric offset
that starts at zero is available under **Advanced**.

## Build a library

Open the dockable **Blocks** panel. Under **Block library**, choose **New library…**,
pick a parent folder, and name the new folder. Use **Open library…** for an
existing folder. The selected folder and recent folders are remembered between
sessions. Favorites and personal descriptions are stored in your preferences;
exporting settings carries them between machines. Descriptions are not embedded
in the shared block asset.

Select a block in the drawing tree, or edit a block, and choose **Save … to
library**. Each `.mycadblock` asset includes its dynamic definition, nested
blocks, and referenced layers, linetypes, and text styles. Existing files are
kept; saving again creates another numbered file. Font files are not embedded.

Browse thumbnails, filter favorites, or search filenames and personal
descriptions. Choose **Insert**, move the preview to a snap point, and click to
place it. Press **R** to rotate by 90°, or set a precise angle in the library
panel. **Esc** cancels without changing the drawing. A simplified preview is
identified when the block exceeds the preview segment limit; the inserted block
retains its full geometry.

Conflicting definitions and styles receive new names so the existing drawing is
preserved. A single Undo removes the insertion and its imported dependencies.
**Refresh** picks up files added outside EntoCAD. Library coordinates use the
current drawing's units; insertion does not infer or convert units.

To reuse an AutoCAD library, open its DWG/DXF using **File → Open**, then save
the desired blocks into the native library folder. Inspect any import warnings
before relying on dynamic controls from that library.

## Exchange with AutoCAD

Save DWG/DXF to share the last evaluated appearance of EntoCAD dynamic blocks.
The companion retains EntoCAD parameters and actions; AutoCAD sees ordinary
blocks. Missing helpers leave the saved geometry usable. Keep the drawing and
helper together when moving an editable EntoCAD drawing between machines.
New helpers are named `.Drawing.dwg.mycad` or `.Drawing.dxf.mycad` and are hidden
in normal file-manager views. Older visible helper filenames remain discoverable.

EntoCAD imports a supported subset of AutoCAD dynamic behavior. Unsupported
controls and DWG object classes still limit round-trip fidelity. The current
writer does not preserve AutoCAD's native dynamic control graph; loss warnings
and a separate export destination protect the original drawing. Real AutoCAD
validation is still required for a company's library and AutoCAD versions.
Open **View → Drawing compatibility** (or **Compatibility** in the status bar)
for helper status, loaded behavior, warnings and links to named affected blocks.
The summary is built with the evaluated drawing; after incremental edits it is
labeled as the last evaluated summary and offers **Refresh summary**. Helper
verification describes the last open/save operation, not continuous monitoring
of external changes. Save As shows the selected DWG/DXF destination and retention
information in this dock before **Save this copy** writes it. Saving back to an
already established destination keeps the usual direct Save behavior.

See the [compatibility contract](autocad-compatibility.md) for the exact scope.

Helpers are bound to the saved drawing bytes. An external edit, including a
rewrite by AutoCAD, disables automatic restoration from that helper. EntoCAD
reports the mismatch and opens the drawing geometry without restoring stale
behavior over the external changes.

## Navigation shortcuts

Use **View → Zoom Selection** to frame selected objects, including blocks. The
command line accepts `ZS` for Zoom Selection and `ZE` for Zoom Extents, plus
`U`/`UNDO` and `REDO`. Navigation preserves the last drawing command for repeat.

## Edit object properties

Select one or more objects and open **Properties**. Common layer, color, linetype
and linetype-scale fields support multiple objects; **Mixed values** identifies
values that differ. Only changed fields are applied. Single-object controls also
include supported coordinates, line length, circle/arc radius and angles, and
block insertion, scale and rotation. Coordinates inside block edit are local to
the shared definition. Angles display degrees and lengths use drawing units.
For a dynamic block, **Edit parameters and actions…** opens the guided editor for
its existing definition. **Edit shared block definition…** opens geometry editing.

Use **Apply properties** to validate and commit all pending fields together as
one Undo step. Invalid fields are marked without partly changing the drawing.
**Discard changes** resets the draft; changing selection also clears pending
fields. Objects on locked layers are protected. Tilted block transforms remain
read-only. Blocks with attached attributes permit translation, rotation and
matching positive XY scale changes; other scale changes are rejected to avoid
distorting attribute text. Dynamic instance parameters remain separate from
shared-definition editing.
