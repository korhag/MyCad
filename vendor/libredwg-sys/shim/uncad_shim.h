#ifndef UNCAD_SHIM_H
#define UNCAD_SHIM_H

#ifdef __cplusplus
extern "C" {
#endif

/* Reads `dwg_path` (DWG or DXF, by extension) and writes it back out as DXF
 * text to `dxf_path`. Returns a LibreDWG error code (see DWG_ERR_* in
 * dwg.h); values >= DWG_ERR_CRITICAL mean the write did not happen.
 *
 * This exists because dwg_write_dxf(Bit_Chain*, Dwg_Data*) is declared only
 * in the private src/out_dxf.h (not a public installed header) and needs a
 * populated Bit_Chain (version/from_version/fh) to call safely -- Bit_Chain
 * is likewise a private, unstable-layout struct. Rather than bindgen the
 * private headers (fragile: no ABI stability guarantee), this narrow,
 * stable-signature shim is compiled as part of the same translation unit
 * set as the rest of vendored LibreDWG, where reaching into those private
 * headers is normal/expected. Rust only ever sees this one function.
 *
 * Recipe ported verbatim from the existing JS/embind binding's
 * dwg_write_dxf_wrapper (lib/libredwg-web/bindings/javascript/embind/binding_func.cpp).
 */
int uncad_write_dxf_file(const char *dwg_path, const char *dxf_path);

/* Writes an already-populated Dwg_Data (from a caller that already called
 * dwg_read_file/dxf_read_file itself, rather than a fresh path -- unlike
 * uncad_write_dxf_file above, which reads *and* writes) out as DXF text to
 * `dxf_path`. Same return-code convention as uncad_write_dxf_file, and the
 * same private-header reasoning for why this needs a shim instead of a
 * direct bindgen binding.
 */
int uncad_write_dxf(Dwg_Data *dwg, const char *dxf_path);

/* Reads ASCII DXF bytes already in memory. Same setup as dxf_read_file
 * (reset dwg, require a trailing newline, reject a DWG magic) without a
 * temporary file. Returns 0 on success, or a LibreDWG error >= DWG_ERR_CRITICAL.
 * `dwg` must be zeroed or a previous result freed by the caller. On failure
 * the caller frees `dwg`.
 */
int uncad_read_dxf_buffer(const char *buf, size_t len, Dwg_Data *dwg);

/* Returns the type-specific entity struct pointer (e.g. Dwg_Entity_LINE*,
 * as a void*) for `obj`, or NULL if `obj` isn't an entity or has no data.
 *
 * This exists because Dwg_Object/Dwg_Object_Entity are bound as opaque
 * blobs on the Rust side (see build.rs: their real definitions pull in a
 * ~90-type union that broke bindgen's struct codegen), so Rust cannot do
 * the C idiom `obj->tio.entity->tio.LINE` itself. The union's members are
 * all same-representation pointers (LibreDWG's own dynapi relies on this:
 * dwg_dynapi_entity_value() is handed a `void *entity` and a type-name
 * string, not a specifically-typed pointer), so reading through any single
 * fixed member name yields the correct address for every entity type --
 * this shim always reads `.tio.UNKNOWN_ENT`, which is defined for every
 * DWG version this project supports. Callers pass the result to
 * dwg_dynapi_entity_value()/dwg_dynapi_entity_field() together with the
 * entity's own dxfname (from dwg_object_get_dxfname), never dereferencing
 * it directly as a Rust struct.
 */
void *uncad_object_entity_ptr(Dwg_Object *obj);

/* Same idea for non-entity OBJECT types (LAYER, BLOCK_RECORD, ...): returns
 * the type-specific struct pointer via obj->tio.object->tio.UNKNOWN_OBJ.
 */
void *uncad_object_object_ptr(Dwg_Object *obj);

/* One leader-line from a MULTILEADER. `points` is the stored vertices.
 * The node's landing and dogleg are copied onto each line of that node.
 * `arrow_override` means `arrow_size` replaces the entity default. */
typedef struct uncad_multileader_line
{
  unsigned int num_points;
  double *points; /* x0,y0,z0, x1,y1,z1, ... -- length 3*num_points */
  int has_last_point;
  double last_point[3];
  int has_dogleg;
  double dogleg_vector[3];
  double dogleg_length;
  int arrow_override;
  double arrow_size;
} uncad_multileader_line_t;

/* kind: 0 none, 1 block, 2 text. Strings are malloc'd and may be NULL. */
typedef struct uncad_multileader_content
{
  int kind;
  char *text;
  double location[3];
  double height;
  double rotation;
  double width;
  int frame;
  char *block_name;
  double block_scale[3];
  double block_rotation;
} uncad_multileader_content_t;

/* One display wire from a REGION (or any 3DSOLID-shaped entity). */
typedef struct uncad_region_wire
{
  unsigned int num_points;
  double *points; /* x0,y0,z0, ... -- length 3*num_points */
} uncad_region_wire_t;

/* Flattens a MULTILEADER entity's ctx.leaders[].lines[] (every leader node
 * can own several lines/splines) into one array of polylines, skipping
 * type==0 ("invisible leader") lines. `entity` is the MULTILEADER's
 * type-specific struct pointer, i.e. what uncad_object_entity_ptr returned
 * for this object (Dwg_Entity_MULTILEADER*, passed as void* for the same
 * reason uncad_object_entity_ptr itself returns void*).
 *
 * This exists because MULTILEADER's leader geometry lives 3 struct levels
 * deep (entity.ctx.leaders[i].lines[j].points[k]) through a chain of nested
 * structs/arrays that dynapi's flat `dwg_dynapi_entity_value` can't reach
 * (it only exposes top-level entity fields by name) and that bindgen can't
 * safely bind directly on the Rust side (Dwg_MLEADER_AnnotContext and its
 * nested LEADER_Node/LEADER_Line types are exactly the kind of
 * pointer-heavy nested struct that already broke bindgen's codegen for
 * Dwg_HATCH_Path -- see build.rs). Doing the walk here, where dwg.h's real
 * struct layouts are visible, sidesteps both problems at once: Rust only
 * ever sees this one flat, stable-shape output type.
 *
 * Returns the number of lines and mallocs *out_lines to that length (each
 * line's own ->points also malloc'd separately); 0 with *out_lines
 * unset/NULL if entity is NULL or has no visible leader lines (not an
 * error -- MULTILEADER's leader-line data is legitimately absent for some
 * block/text-only content variants). Free with
 * uncad_multileader_free_lines.
 */
unsigned int uncad_multileader_get_lines(void *entity,
                                          uncad_multileader_line_t **out_lines);

void uncad_multileader_free_lines(uncad_multileader_line_t *lines,
                                   unsigned int num_lines);

/* Fills `out` from ctx content. Returns 0, or -1 when `entity` is NULL.
 * `dwg` resolves a block content handle to a name; it may be NULL. */
int uncad_multileader_get_content(Dwg_Data *dwg, void *entity,
                                   uncad_multileader_content_t *out);

void uncad_multileader_free_content(uncad_multileader_content_t *content);

/* Copies REGION wireframe wires. Returns 0 when the wire cache is empty.
 * Applies axis/translation/scale when a wire stores a transform. */
unsigned int uncad_region_get_wires(void *entity,
                                     uncad_region_wire_t **out_wires);

void uncad_region_free_wires(uncad_region_wire_t *wires, unsigned int num_wires);

#ifdef __cplusplus
}
#endif

#endif /* UNCAD_SHIM_H */
