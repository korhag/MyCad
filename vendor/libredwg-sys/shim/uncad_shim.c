#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "dwg.h"
#include "dwg_api.h"
#include "bits.h"
#include "out_dxf.h"
#include "uncad_shim.h"

extern int dwg_read_dxf (Bit_Chain *dat, Dwg_Data *dwg);

int
uncad_write_dxf_file (const char *dwg_path, const char *dxf_path)
{
  if (strcmp (dwg_path, dxf_path) == 0)
    return DWG_ERR_IOERROR;

  Dwg_Data dwg;
  Bit_Chain dat;
  memset (&dwg, 0, sizeof (dwg));
  memset (&dat, 0, sizeof (dat));

  int error = dwg_read_file (dwg_path, &dwg);
  if (error >= DWG_ERR_CRITICAL)
    {
      dwg_free (&dwg);
      return error;
    }

  dat.version = dwg.header.version;
  dat.from_version = dwg.header.from_version;
  dat.fh = fopen (dxf_path, "wb");
  if (!dat.fh)
    {
      dwg_free (&dwg);
      return DWG_ERR_IOERROR;
    }

  error = dwg_write_dxf (&dat, &dwg);
  fclose (dat.fh);
  dwg_free (&dwg);
  return error;
}

int
uncad_write_dxf (Dwg_Data *dwg, const char *dxf_path)
{
  Bit_Chain dat;
  memset (&dat, 0, sizeof (dat));

  dat.version = dwg->header.version;
  dat.from_version = dwg->header.from_version;
  dat.fh = fopen (dxf_path, "wb");
  if (!dat.fh)
    return DWG_ERR_IOERROR;

  int error = dwg_write_dxf (&dat, dwg);
  fclose (dat.fh);
  return error;
}

int
uncad_read_dxf_buffer (const char *buf, size_t len, Dwg_Data *dwg)
{
  Bit_Chain dat;
  Dwg_Version_Type version;
  unsigned int loglevel;
  size_t size;
  int add_nl;
  int error;

  if (!buf || !dwg)
    return DWG_ERR_INVALIDDWG;

  loglevel = dwg->opts & DWG_OPTS_LOGLEVEL;
  version = dwg->header.version;
  memset (dwg, 0, sizeof (Dwg_Data));
  dwg->opts = loglevel | DWG_OPTS_INDXF;
  dwg->header.version = version;

  memset (&dat, 0, sizeof (Bit_Chain));
  add_nl = (len == 0 || (unsigned char)buf[len - 1] != '\n');
  size = len + (add_nl ? 1 : 0);
  dat.chain = (unsigned char *)calloc (1, size + 1);
  if (!dat.chain)
    return DWG_ERR_OUTOFMEM;
  if (len)
    memcpy (dat.chain, buf, len);
  if (add_nl)
    dat.chain[len] = '\n';
  dat.chain[size] = '\0';
  dat.size = size;
  dat.byte = 0;
  dat.bit = 0;
  dat.from_version = dwg->header.from_version;
  dat.version = dwg->header.version;
  dat.opts = dwg->opts;

  if (size < 31)
    {
      free (dat.chain);
      return DWG_ERR_IOERROR;
    }
  if (!memcmp (dat.chain, "AC10", 4) || !memcmp (dat.chain, "AC1.", 4)
      || !memcmp (dat.chain, "AC2.10", 4) || !memcmp (dat.chain, "MC0.0", 4))
    {
      free (dat.chain);
      return DWG_ERR_INVALIDDWG;
    }

  error = dwg_read_dxf (&dat, dwg);
  free (dat.chain);
  if (error >= DWG_ERR_CRITICAL)
    return error;
  return 0;
}

void *
uncad_object_entity_ptr (Dwg_Object *obj)
{
  /* dwg_object_to_entity() dereferences obj->supertype with no null check
     of its own -- every current Rust call site already null-checks obj
     before calling this, but this shim's own contract shouldn't rely on
     callers upholding that silently; fail closed instead of crashing. */
  if (!obj)
    return NULL;
  int error = 0;
  Dwg_Object_Entity *ent = dwg_object_to_entity (obj, &error);
  if (!ent || error)
    return NULL;
  return (void *)ent->tio.UNKNOWN_ENT;
}

void *
uncad_object_object_ptr (Dwg_Object *obj)
{
  /* Same reasoning as uncad_object_entity_ptr above. */
  if (!obj)
    return NULL;
  int error = 0;
  Dwg_Object_Object *o = dwg_object_to_object (obj, &error);
  if (!o || error)
    return NULL;
  return (void *)o->tio.UNKNOWN_OBJ;
}

static int
leader_line_visible (const Dwg_Entity_MULTILEADER *mleader,
                     const Dwg_LEADER_Line *line)
{
  /* type 0 is "invisible" from R2010. Earlier files leave type unset. */
  if (mleader->class_version >= 2 && line->type == 0)
    return 0;
  if (line->num_points == 0 || line->num_points > 100000 || !line->points)
    return 0;
  return 1;
}

unsigned int
uncad_multileader_get_lines (void *entity, uncad_multileader_line_t **out_lines)
{
  *out_lines = NULL;
  if (!entity)
    return 0;
  Dwg_Entity_MULTILEADER *mleader = (Dwg_Entity_MULTILEADER *)entity;
  Dwg_MLEADER_AnnotContext *ctx = &mleader->ctx;
  if (ctx->num_leaders > 5000 || !ctx->leaders)
    return 0;

  /* First pass: count visible lines so out_lines can be sized exactly. */
  unsigned int total_lines = 0;
  for (BITCODE_BL i = 0; i < ctx->num_leaders; i++)
    {
      Dwg_LEADER_Node *node = &ctx->leaders[i];
      if (node->num_lines > 5000 || !node->lines)
        continue;
      for (BITCODE_BL j = 0; j < node->num_lines; j++)
        {
          if (leader_line_visible (mleader, &node->lines[j]))
            total_lines++;
        }
    }
  if (total_lines == 0)
    return 0;

  uncad_multileader_line_t *lines
      = calloc (total_lines, sizeof (uncad_multileader_line_t));
  if (!lines)
    return 0;

  unsigned int k = 0;
  for (BITCODE_BL i = 0; i < ctx->num_leaders; i++)
    {
      Dwg_LEADER_Node *node = &ctx->leaders[i];
      if (node->num_lines > 5000 || !node->lines)
        continue;
      for (BITCODE_BL j = 0; j < node->num_lines; j++)
        {
          Dwg_LEADER_Line *line = &node->lines[j];
          if (!leader_line_visible (mleader, line))
            continue;
          double *pts = malloc (sizeof (double) * 3 * line->num_points);
          if (!pts)
            continue;
          for (BITCODE_BL p = 0; p < line->num_points; p++)
            {
              pts[p * 3 + 0] = line->points[p].x;
              pts[p * 3 + 1] = line->points[p].y;
              pts[p * 3 + 2] = line->points[p].z;
            }
          lines[k].num_points = line->num_points;
          lines[k].points = pts;
          if (node->has_lastleaderlinepoint)
            {
              lines[k].has_last_point = 1;
              lines[k].last_point[0] = node->lastleaderlinepoint.x;
              lines[k].last_point[1] = node->lastleaderlinepoint.y;
              lines[k].last_point[2] = node->lastleaderlinepoint.z;
            }
          if (node->has_dogleg)
            {
              lines[k].has_dogleg = 1;
              lines[k].dogleg_vector[0] = node->dogleg_vector.x;
              lines[k].dogleg_vector[1] = node->dogleg_vector.y;
              lines[k].dogleg_vector[2] = node->dogleg_vector.z;
              lines[k].dogleg_length = node->dogleg_length;
            }
          if ((line->flags & 16) != 0)
            {
              lines[k].arrow_override = 1;
              lines[k].arrow_size = line->arrow_size;
            }
          k++;
        }
    }

  *out_lines = lines;
  return k;
}

void
uncad_multileader_free_lines (uncad_multileader_line_t *lines, unsigned int num_lines)
{
  if (!lines)
    return;
  for (unsigned int i = 0; i < num_lines; i++)
    free (lines[i].points);
  free (lines);
}

static char *
uncad_dup_text (const char *text)
{
  if (!text || !text[0])
    return NULL;
  return strdup (text);
}

static void
copy_point (double out[3], BITCODE_3BD point)
{
  out[0] = point.x;
  out[1] = point.y;
  out[2] = point.z;
}

int
uncad_multileader_get_content (Dwg_Data *dwg, void *entity,
                               uncad_multileader_content_t *out)
{
  if (!out)
    return -1;
  memset (out, 0, sizeof *out);
  out->block_scale[0] = 1.0;
  out->block_scale[1] = 1.0;
  out->block_scale[2] = 1.0;
  if (!entity)
    return -1;

  Dwg_Entity_MULTILEADER *mleader = (Dwg_Entity_MULTILEADER *)entity;
  Dwg_MLEADER_AnnotContext *ctx = &mleader->ctx;
  out->frame = mleader->has_text_frame ? 1 : 0;

  int want_text = ctx->has_content_txt;
  int want_block = ctx->has_content_blk;
  if (want_text && want_block)
    {
      if (mleader->style_content == 1)
        want_text = 0;
      else
        want_block = 0;
    }

  if (want_text)
    {
      out->kind = 2;
      out->text = uncad_dup_text (ctx->content.txt.default_text);
      copy_point (out->location, ctx->content.txt.location);
      out->height = ctx->content.txt.height;
      if (out->height == 0.0)
        out->height = ctx->text_height;
      out->rotation = ctx->content.txt.rotation;
      out->width = ctx->content.txt.width;
      return 0;
    }

  if (want_block)
    {
      out->kind = 1;
      copy_point (out->location, ctx->content.blk.location);
      out->block_rotation = ctx->content.blk.rotation;
      out->block_scale[0] = ctx->content.blk.scale.x;
      out->block_scale[1] = ctx->content.blk.scale.y;
      out->block_scale[2] = ctx->content.blk.scale.z;
      if (dwg && ctx->content.blk.block_table)
        {
          int alloced = 0;
          char *name = dwg_dynapi_handle_name (dwg, ctx->content.blk.block_table,
                                               &alloced);
          if (name)
            {
              out->block_name = uncad_dup_text (name);
              if (alloced)
                free (name);
            }
        }
      return 0;
    }
  return 0;
}

void
uncad_multileader_free_content (uncad_multileader_content_t *content)
{
  if (!content)
    return;
  free (content->text);
  free (content->block_name);
  content->text = NULL;
  content->block_name = NULL;
}

static void
apply_wire_point (const Dwg_3DSOLID_wire *wire, BITCODE_3BD point, double *out)
{
  if (!wire->transform_present)
    {
      out[0] = point.x;
      out[1] = point.y;
      out[2] = point.z;
      return;
    }
  double sx = wire->scale.x;
  double sy = wire->scale.y;
  double sz = wire->scale.z;
  out[0] = wire->translation.x + wire->axis_x.x * point.x * sx
           + wire->axis_y.x * point.y * sy + wire->axis_z.x * point.z * sz;
  out[1] = wire->translation.y + wire->axis_x.y * point.x * sx
           + wire->axis_y.y * point.y * sy + wire->axis_z.y * point.z * sz;
  out[2] = wire->translation.z + wire->axis_x.z * point.x * sx
           + wire->axis_y.z * point.y * sy + wire->axis_z.z * point.z * sz;
}

unsigned int
uncad_region_get_wires (void *entity, uncad_region_wire_t **out_wires)
{
  if (out_wires)
    *out_wires = NULL;
  if (!entity || !out_wires)
    return 0;
  Dwg_Entity__3DSOLID *solid = (Dwg_Entity__3DSOLID *)entity;
  if (!solid->wireframe_data_present || !solid->isoline_present
      || solid->num_wires == 0 || solid->num_wires > 10000 || !solid->wires)
    return 0;

  uncad_region_wire_t *wires
      = calloc (solid->num_wires, sizeof (uncad_region_wire_t));
  if (!wires)
    return 0;

  unsigned int kept = 0;
  for (BITCODE_BL i = 0; i < solid->num_wires; i++)
    {
      Dwg_3DSOLID_wire *wire = &solid->wires[i];
      if (wire->num_points < 2 || wire->num_points > 100000 || !wire->points)
        continue;
      double *pts = malloc (sizeof (double) * 3 * wire->num_points);
      if (!pts)
        continue;
      for (BITCODE_BL p = 0; p < wire->num_points; p++)
        apply_wire_point (wire, wire->points[p], pts + (size_t)p * 3);
      wires[kept].num_points = wire->num_points;
      wires[kept].points = pts;
      kept++;
    }
  if (kept == 0)
    {
      free (wires);
      return 0;
    }
  *out_wires = wires;
  return kept;
}

void
uncad_region_free_wires (uncad_region_wire_t *wires, unsigned int num_wires)
{
  if (!wires)
    return;
  for (unsigned int i = 0; i < num_wires; i++)
    free (wires[i].points);
  free (wires);
}
