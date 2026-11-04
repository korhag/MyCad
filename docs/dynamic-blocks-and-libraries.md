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

Select geometry and use **Rotate…**, **Flip / mirror…**, **Position by option…**,
**Show / hide…**, or **Link text…** in the dock. Position destinations use block
coordinates in the drawing's units.

Use **Add angle parameter** for a rotation angle. Its value is stored in degrees,
and its minimum and maximum can be left unrestricted or set in the parameter
editor. Pick the geometry first, start the action, then supply its pivot, axis,
or anchors. Missing or invalid settings keep the setup open with an explanation.

Named presets store useful combinations of values. Compatibility rules constrain
which options can be combined. Test the complete configuration before saving a
block, especially when several actions affect the same geometry.
Test values are temporary previews for the edited occurrence. Change a parameter's
default or a placed instance's configuration to retain a value outside Test mode.

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
existing folder. The selected folder is remembered for the current session.

Select a block in the drawing tree, or edit a block, and choose **Save … to
library**. Each `.mycadblock` asset includes its dynamic definition, nested
blocks, and referenced layers, linetypes, and text styles. Existing files are
kept; saving again creates another numbered file. Font files are not embedded.

Search by filename and choose **Insert**. The block appears at the center of the
current view, selected and ready to place with Move. Conflicting definitions and
styles receive new names so the existing drawing is preserved. A single Undo
removes the insertion and its imported dependencies. **Refresh** picks up files
added outside EntoCAD. Library coordinates use the current drawing's units;
insertion does not infer or convert units.

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
See the [compatibility contract](autocad-compatibility.md) for the exact scope.

Helpers are bound to the saved drawing bytes. An external edit, including a
rewrite by AutoCAD, disables automatic restoration from that helper. EntoCAD
reports the mismatch and opens the drawing geometry without restoring stale
behavior over the external changes.

## Navigation shortcuts

Use **View → Zoom Selection** to frame selected objects, including blocks. The
command line accepts `ZS` for Zoom Selection and `ZE` for Zoom Extents, plus
`U`/`UNDO` and `REDO`. Navigation preserves the last drawing command for repeat.
