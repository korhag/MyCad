//! wgpu CAD viewport renderer, kept separate from application chrome.

use std::sync::Arc;

use cad_core::{Point2, Transform2};
use cad_viewport::Camera2;
use egui::PaintCallbackInfo;
use egui_wgpu::wgpu;

use crate::tessellate::{DisplayList, GpuVertex, OverlayBatches};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    overlay_color: [f32; 4],
    overlay_params: [f32; 4],
    model: [[f32; 4]; 4],
}

const UNIFORM_SIZE: u64 = std::mem::size_of::<Uniforms>() as u64;
const _: () = assert!(UNIFORM_SIZE == 160);

struct UniformSlot {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

// ------------------------------------------------------------
// Type: CadGpu
// Purpose: Persistent wgpu resources stored in egui callback_resources.
// ------------------------------------------------------------
pub struct CadGpu {
    shader: wgpu::ShaderModule,
    pipelines: Vec<ScenePipelines>,
    pipeline_layout: wgpu::PipelineLayout,
    scene: UniformSlot,
    selection: UniformSlot,
    preview: UniformSlot,
    line_chunks: Vec<VertexChunk>,
    fill_chunks: Vec<VertexChunk>,
    line_count: u32,
    fill_count: u32,
    line_capacity: u32,
    fill_capacity: u32,
    max_line_vertices: u32,
    max_fill_vertices: u32,
    uploaded_generation: u64,
    target_format: wgpu::TextureFormat,
    max_texture_px: u32,
    blit_pipeline: wgpu::RenderPipeline,
    blit_layout: wgpu::BindGroupLayout,
    blit_sampler: wgpu::Sampler,
    scene_picture: Option<CachedPicture>,
    overlay_picture: Option<CachedPicture>,
    scene_key: Option<SceneKey>,
    overlay_key: Option<OverlayKey>,
    overlay_visible: bool,
}

// ------------------------------------------------------------
// Type: ScenePipelines
// Purpose: Line and fill pipelines for one sample count. Both the
//          moving (1x) and still (MSAA) sets stay cached, so starting
//          a pan never compiles a shader.
// ------------------------------------------------------------
struct ScenePipelines {
    samples: u32,
    line: wgpu::RenderPipeline,
    fill: wgpu::RenderPipeline,
    line_premul: wgpu::RenderPipeline,
    fill_premul: wgpu::RenderPipeline,
}

struct VertexChunk {
    buffer: wgpu::Buffer,
    capacity: u32,
}

/// Offscreen pictures grow in steps of this many pixels, so dragging a
/// dock split reuses the texture instead of reallocating every frame.
pub const PICTURE_SLACK_PX: u32 = 256;

// ------------------------------------------------------------
// Type: CachedPicture
// Purpose: One offscreen raster. The texture may be larger than the
//          viewport; `used` is the region drawn and blitted.
// ------------------------------------------------------------
struct CachedPicture {
    texture_px: [u32; 2],
    used: [u32; 2],
    msaa_samples: u32,
    color_view: wgpu::TextureView,
    msaa_view: Option<wgpu::TextureView>,
    bind_group: wgpu::BindGroup,
    scale: wgpu::Buffer,
    _color: wgpu::Texture,
    _msaa: Option<wgpu::Texture>,
}

impl CachedPicture {
    fn new(
        device: &wgpu::Device,
        blit_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        format: wgpu::TextureFormat,
        texture_px: [u32; 2],
        samples: u32,
        label: &str,
    ) -> Self {
        let size = wgpu::Extent3d {
            width: texture_px[0],
            height: texture_px[1],
            depth_or_array_layers: 1,
        };
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let msaa = (samples > 1).then(|| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        });
        let msaa_view = msaa
            .as_ref()
            .map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()));
        let scale = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: BLIT_UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: blit_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: scale.as_entire_binding(),
                },
            ],
        });
        Self {
            texture_px,
            used: [0, 0],
            msaa_samples: samples,
            color_view,
            msaa_view,
            bind_group,
            scale,
            _color: color,
            _msaa: msaa,
        }
    }

    /// Point the blit at the drawn region when the viewport size changes.
    fn set_used(&mut self, queue: &wgpu::Queue, used: [u32; 2]) {
        if self.used == used {
            return;
        }
        self.used = used;
        let scale = [
            used[0] as f32 / self.texture_px[0].max(1) as f32,
            used[1] as f32 / self.texture_px[1].max(1) as f32,
            0.0,
            0.0,
        ];
        queue.write_buffer(&self.scale, 0, bytemuck::cast_slice(&scale));
    }
}

const BLIT_UNIFORM_SIZE: u64 = 16;

// ------------------------------------------------------------
// Function: picture_texture_size
// Purpose: Pure resize rule. Returns None when `current` can hold
//          `need` without wasting more than 4x the area; otherwise
//          the next size, rounded up to PICTURE_SLACK_PX steps and
//          clamped to the device limit.
// ------------------------------------------------------------
pub fn picture_texture_size(
    current: Option<[u32; 2]>,
    need: [u32; 2],
    max_px: u32,
) -> Option<[u32; 2]> {
    let max_px = max_px.max(1);
    let need = [need[0].clamp(1, max_px), need[1].clamp(1, max_px)];
    let round = |value: u32| {
        value
            .div_ceil(PICTURE_SLACK_PX)
            .saturating_mul(PICTURE_SLACK_PX)
            .min(max_px)
    };
    let target = [round(need[0]), round(need[1])];
    if let Some(current) = current {
        let fits = need[0] <= current[0] && need[1] <= current[1];
        let area = u64::from(current[0]) * u64::from(current[1]);
        let target_area = u64::from(target[0]) * u64::from(target[1]);
        if fits && area <= target_area.saturating_mul(4) {
            return None;
        }
    }
    Some(target)
}

/// An empty overlay skips both its render pass and its blit.
fn overlay_has_content(selection: &OverlayBatches, preview: &OverlayBatches) -> bool {
    !selection.is_empty() || !preview.is_empty()
}

/// The scene renders at one sample while the view moves, then settles
/// to the chosen MSAA.
pub fn motion_samples(interactive: bool, chosen: u32) -> u32 {
    if interactive {
        1
    } else {
        sanitize_viewport_samples(chosen)
    }
}

// ------------------------------------------------------------
// Function: viewport_pixel_size
// Purpose: Physical size of a viewport rect, rounded the same way
//          egui rounds the paint callback viewport, so the cached
//          picture maps 1:1 onto the screen.
// ------------------------------------------------------------
pub fn viewport_pixel_size(min: [f32; 2], max: [f32; 2], pixels_per_point: f32) -> [u32; 2] {
    let edge = |value: f32| (value * pixels_per_point).round();
    [
        (edge(max[0]) - edge(min[0])).max(0.0) as u32,
        (edge(max[1]) - edge(min[1])).max(0.0) as u32,
    ]
}

const MIN_VERTEX_CAPACITY: u32 = 1024;
const VERTEX_STRIDE: u64 = std::mem::size_of::<GpuVertex>() as u64;
const DEFAULT_MAX_BUFFER_SIZE: u64 = 256 * 1024 * 1024;

fn device_max_buffer_size(device: &wgpu::Device) -> u64 {
    match device.limits().max_buffer_size {
        0 => DEFAULT_MAX_BUFFER_SIZE,
        size => size,
    }
}

// ------------------------------------------------------------
// Type: GpuUpload
// Purpose: Tells the viewport whether the CPU display list was
//          rebuilt or only gained a vertex tail.
// ------------------------------------------------------------
/// How many rewritten spans an edit may upload before the whole buffer
/// is cheaper to send.
pub const MAX_PATCH_RANGES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuUpload {
    #[default]
    Full,
    Append {
        line_start: u32,
        fill_start: u32,
    },
    /// Rewritten spans plus any appended tail. More than
    /// [`MAX_PATCH_RANGES`] in either buffer falls back to [`Self::Full`].
    Patch {
        lines: [UploadRange; MAX_PATCH_RANGES],
        line_count: u8,
        fills: [UploadRange; MAX_PATCH_RANGES],
        fill_count: u8,
    },
}

// ------------------------------------------------------------
// Type: GpuUploadPlan
// Purpose: Pure upload decision used by CadGpu and unit tests.
// ------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuUploadPlan {
    Skip,
    Append { line_start: u32, fill_start: u32 },
    Patch {
        lines: [UploadRange; MAX_PATCH_RANGES],
        line_count: u8,
        fills: [UploadRange; MAX_PATCH_RANGES],
        fill_count: u8,
    },
    Full,
    GrowAndFull,
}

pub fn plan_gpu_upload(
    uploaded_generation: u64,
    generation: u64,
    uploaded_line_count: u32,
    uploaded_fill_count: u32,
    line_capacity: u32,
    fill_capacity: u32,
    new_line_count: u32,
    new_fill_count: u32,
    kind: GpuUpload,
) -> GpuUploadPlan {
    if generation == uploaded_generation {
        return GpuUploadPlan::Skip;
    }
    let fits = new_line_count <= line_capacity && new_fill_count <= fill_capacity;
    match kind {
        GpuUpload::Append {
            line_start,
            fill_start,
        } => {
            let contiguous = line_start == uploaded_line_count && fill_start == uploaded_fill_count;
            let ordered = new_line_count >= line_start && new_fill_count >= fill_start;
            if fits && contiguous && ordered {
                return GpuUploadPlan::Append {
                    line_start,
                    fill_start,
                };
            }
        }
        GpuUpload::Patch {
            lines,
            line_count,
            fills,
            fill_count,
        } => {
            if fits
                && ranges_fit(&lines, line_count, new_line_count)
                && ranges_fit(&fills, fill_count, new_fill_count)
            {
                return GpuUploadPlan::Patch {
                    lines,
                    line_count,
                    fills,
                    fill_count,
                };
            }
        }
        GpuUpload::Full => {}
    }
    if fits {
        GpuUploadPlan::Full
    } else {
        GpuUploadPlan::GrowAndFull
    }
}

fn ranges_fit(ranges: &[UploadRange; MAX_PATCH_RANGES], count: u8, vertices: u32) -> bool {
    let count = (count as usize).min(MAX_PATCH_RANGES);
    ranges[..count]
        .iter()
        .all(|range| range.start <= range.end && range.end <= vertices)
}

/// Merge rewritten spans into one upload. Too many spans send the whole buffer.
pub fn upload_for_ranges(
    lines: &mut Vec<std::ops::Range<u32>>,
    fills: &mut Vec<std::ops::Range<u32>>,
) -> GpuUpload {
    crate::tessellate::merge_vertex_ranges(lines);
    crate::tessellate::merge_vertex_ranges(fills);
    lines.retain(|range| range.end > range.start);
    fills.retain(|range| range.end > range.start);
    if lines.is_empty() && fills.is_empty() {
        return GpuUpload::Full;
    }
    if lines.len() > MAX_PATCH_RANGES || fills.len() > MAX_PATCH_RANGES {
        return GpuUpload::Full;
    }
    let mut line_ranges = [UploadRange { start: 0, end: 0 }; MAX_PATCH_RANGES];
    let mut fill_ranges = [UploadRange { start: 0, end: 0 }; MAX_PATCH_RANGES];
    for (index, range) in lines.iter().enumerate() {
        line_ranges[index] = UploadRange {
            start: range.start,
            end: range.end,
        };
    }
    for (index, range) in fills.iter().enumerate() {
        fill_ranges[index] = UploadRange {
            start: range.start,
            end: range.end,
        };
    }
    GpuUpload::Patch {
        lines: line_ranges,
        line_count: lines.len() as u8,
        fills: fill_ranges,
        fill_count: fills.len() as u8,
    }
}

fn max_vertices_for_buffer(max_buffer_size: u64, align: u32) -> u32 {
    let align = align.max(1);
    let n = (max_buffer_size / VERTEX_STRIDE).min(u64::from(u32::MAX)) as u32;
    (n / align) * align
}

fn next_vertex_capacity(needed: u32, max_per_buffer: u32) -> u32 {
    if needed == 0 {
        return 0;
    }
    let max_per_buffer = max_per_buffer.max(1);
    let capped = needed.min(max_per_buffer);
    let grown = capped.max(MIN_VERTEX_CAPACITY).next_power_of_two();
    grown.min(max_per_buffer).max(capped)
}

fn chunk_capacities(needed: u32, max_per_buffer: u32) -> Vec<u32> {
    if needed == 0 {
        return Vec::new();
    }
    let max_per_buffer = max_per_buffer.max(1);
    let mut remaining = needed;
    let mut caps = Vec::new();
    while remaining > 0 {
        let take = remaining.min(max_per_buffer);
        caps.push(next_vertex_capacity(take, max_per_buffer));
        remaining -= take;
    }
    caps
}

impl CadGpu {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mycad.viewport"),
            source: wgpu::ShaderSource::Wgsl(include_str!("line.wgsl").into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mycad.viewport.bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mycad.viewport.pll"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipelines = [1, crate::VIEWPORT_MSAA_SAMPLES]
            .into_iter()
            .map(|samples| {
                make_scene_pipelines(device, &pipeline_layout, &shader, target_format, samples)
            })
            .collect();
        let (blit_pipeline, blit_layout, blit_sampler) = make_blit(device, target_format);
        Self {
            shader,
            pipelines,
            scene: UniformSlot::new(device, &bind_group_layout, "mycad.scene"),
            selection: UniformSlot::new(device, &bind_group_layout, "mycad.selection"),
            preview: UniformSlot::new(device, &bind_group_layout, "mycad.preview"),
            pipeline_layout,
            line_chunks: Vec::new(),
            fill_chunks: Vec::new(),
            line_count: 0,
            fill_count: 0,
            line_capacity: 0,
            fill_capacity: 0,
            max_line_vertices: max_vertices_for_buffer(device_max_buffer_size(device), 2),
            max_fill_vertices: max_vertices_for_buffer(device_max_buffer_size(device), 3),
            uploaded_generation: 0,
            target_format,
            max_texture_px: device.limits().max_texture_dimension_2d,
            blit_pipeline,
            blit_layout,
            blit_sampler,
            scene_picture: None,
            overlay_picture: None,
            scene_key: None,
            overlay_key: None,
            overlay_visible: false,
        }
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        list: &DisplayList,
        generation: u64,
        kind: GpuUpload,
    ) {
        let new_line_count = list.line_vertices.len() as u32;
        let new_fill_count = list.triangle_vertices.len() as u32;
        let plan = plan_gpu_upload(
            self.uploaded_generation,
            generation,
            self.line_count,
            self.fill_count,
            self.line_capacity,
            self.fill_capacity,
            new_line_count,
            new_fill_count,
            kind,
        );
        match plan {
            GpuUploadPlan::Skip => {}
            GpuUploadPlan::Append {
                line_start,
                fill_start,
            } => {
                write_vertex_tail(queue, &self.line_chunks, line_start, &list.line_vertices);
                write_vertex_tail(
                    queue,
                    &self.fill_chunks,
                    fill_start,
                    &list.triangle_vertices,
                );
                self.line_count = new_line_count;
                self.fill_count = new_fill_count;
                self.uploaded_generation = generation;
            }
            GpuUploadPlan::Patch {
                lines,
                line_count,
                fills,
                fill_count,
            } => {
                write_upload_ranges(
                    queue,
                    &self.line_chunks,
                    &lines,
                    line_count,
                    &list.line_vertices,
                );
                write_upload_ranges(
                    queue,
                    &self.fill_chunks,
                    &fills,
                    fill_count,
                    &list.triangle_vertices,
                );
                self.line_count = new_line_count;
                self.fill_count = new_fill_count;
                self.uploaded_generation = generation;
            }
            GpuUploadPlan::Full | GpuUploadPlan::GrowAndFull => {
                if matches!(plan, GpuUploadPlan::GrowAndFull) {
                    self.grow_buffers(device, new_line_count, new_fill_count);
                }
                write_vertex_range(queue, &self.line_chunks, 0, &list.line_vertices);
                write_vertex_range(queue, &self.fill_chunks, 0, &list.triangle_vertices);
                if new_line_count == 0 {
                    self.line_chunks.clear();
                    self.line_capacity = 0;
                }
                if new_fill_count == 0 {
                    self.fill_chunks.clear();
                    self.fill_capacity = 0;
                }
                self.line_count = new_line_count;
                self.fill_count = new_fill_count;
                self.uploaded_generation = generation;
            }
        }
    }

    // --------------------------------------------------------
    // Method: refresh_pictures
    // Purpose: Redraw the cached scene and overlay only when their
    //          keys change. Each uniform is written just before the
    //          picture that reads it is drawn.
    // --------------------------------------------------------
    fn refresh_pictures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &CadFrame,
    ) {
        let used = frame.viewport_px;
        if used[0] == 0 || used[1] == 0 {
            return;
        }
        let chosen = sanitize_viewport_samples(frame.samples);
        let samples = motion_samples(frame.interactive, chosen);
        self.ensure_pipelines(device, chosen);
        self.ensure_pipelines(device, samples);
        let used = self.ensure_picture(device, queue, used, chosen, true);
        let scene_key = SceneKey::capture(frame, used[0], used[1], samples);
        if scene_needs_redraw(self.scene_key, scene_key) {
            self.scene.write(
                queue,
                frame.camera,
                frame.origin,
                frame.aspect,
                [0.0; 4],
                0.0,
                0.0,
                Transform2::identity_mat4(),
            );
            self.render_cached(
                encoder,
                true,
                samples,
                frame.clear_color,
                |gpu, pipelines, pass| draw_scene(gpu, pipelines, pass),
            );
            self.scene_key = Some(scene_key);
        }

        self.overlay_visible = overlay_has_content(&frame.selection, &frame.preview);
        if !self.overlay_visible {
            self.overlay_key = None;
            return;
        }
        self.ensure_picture(device, queue, used, chosen, false);
        let overlay_key = OverlayKey {
            scene: scene_key,
            fingerprint: overlay_fingerprint(frame),
        };
        if self.overlay_key == Some(overlay_key) {
            return;
        }
        self.selection.write(
            queue,
            frame.camera,
            frame.origin,
            frame.aspect,
            frame.selection_color,
            1.0,
            1.0,
            Transform2::identity_mat4(),
        );
        self.preview.write(
            queue,
            frame.camera,
            frame.origin,
            frame.aspect,
            frame.preview_color,
            1.0,
            1.0,
            frame.preview_model,
        );
        self.render_cached(
            encoder,
            false,
            samples,
            [0.0, 0.0, 0.0, 0.0],
            |gpu, pipelines, pass| {
                draw_overlay(gpu, pipelines, pass, &gpu.selection.bind_group, &frame.selection);
                draw_overlay(gpu, pipelines, pass, &gpu.preview.bind_group, &frame.preview);
            },
        );
        self.overlay_key = Some(overlay_key);
    }

    fn ensure_pipelines(&mut self, device: &wgpu::Device, samples: u32) {
        if self.pipelines_for(samples).is_some() {
            return;
        }
        let set = make_scene_pipelines(
            device,
            &self.pipeline_layout,
            &self.shader,
            self.target_format,
            samples,
        );
        self.pipelines.push(set);
    }

    fn pipelines_for(&self, samples: u32) -> Option<&ScenePipelines> {
        self.pipelines.iter().find(|set| set.samples == samples)
    }

    // --------------------------------------------------------
    // Method: ensure_picture
    // Purpose: Keep one offscreen texture big enough for `used`,
    //          growing in PICTURE_SLACK_PX steps. Returns the drawn
    //          size, clamped to the texture.
    // --------------------------------------------------------
    fn ensure_picture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        used: [u32; 2],
        msaa_samples: u32,
        scene: bool,
    ) -> [u32; 2] {
        let current = if scene {
            self.scene_picture.as_ref()
        } else {
            self.overlay_picture.as_ref()
        }
        .filter(|picture| picture.msaa_samples == msaa_samples)
        .map(|picture| picture.texture_px);
        let fresh = picture_texture_size(current, used, self.max_texture_px).map(|texture_px| {
            CachedPicture::new(
                device,
                &self.blit_layout,
                &self.blit_sampler,
                self.target_format,
                texture_px,
                msaa_samples,
                if scene { "mycad.scene" } else { "mycad.overlay" },
            )
        });
        let created = fresh.is_some();
        let slot = if scene {
            &mut self.scene_picture
        } else {
            &mut self.overlay_picture
        };
        if let Some(picture) = fresh {
            *slot = Some(picture);
        }
        let mut drawn = used;
        if let Some(picture) = slot.as_mut() {
            drawn = [
                used[0].min(picture.texture_px[0]),
                used[1].min(picture.texture_px[1]),
            ];
            picture.set_used(queue, drawn);
        }
        if created {
            if scene {
                self.scene_key = None;
            } else {
                self.overlay_key = None;
            }
        }
        drawn
    }

    fn render_cached(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        scene: bool,
        samples: u32,
        clear: [f32; 4],
        draw: impl FnOnce(&CadGpu, &ScenePipelines, &mut wgpu::RenderPass<'_>),
    ) {
        let Some(picture) = (if scene {
            self.scene_picture.as_ref()
        } else {
            self.overlay_picture.as_ref()
        }) else {
            return;
        };
        let Some(pipelines) = self.pipelines_for(samples) else {
            return;
        };
        let multisampled = samples > 1;
        if multisampled && picture.msaa_samples != samples {
            return;
        }
        let msaa_view = picture.msaa_view.as_ref().filter(|_| multisampled);
        let (view, resolve_target, store) = match msaa_view {
            Some(msaa) => (msaa, Some(&picture.color_view), wgpu::StoreOp::Discard),
            None => (&picture.color_view, None, wgpu::StoreOp::Store),
        };
        let label = if scene {
            "mycad.scene.cache"
        } else {
            "mycad.overlay.cache"
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: clear[0] as f64,
                        g: clear[1] as f64,
                        b: clear[2] as f64,
                        a: clear[3] as f64,
                    }),
                    store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_viewport(
            0.0,
            0.0,
            picture.used[0] as f32,
            picture.used[1] as f32,
            0.0,
            1.0,
        );
        draw(self, pipelines, &mut pass);
    }

    fn grow_buffers(&mut self, device: &wgpu::Device, line_count: u32, fill_count: u32) {
        let line_caps = chunk_capacities(line_count, self.max_line_vertices);
        let fill_caps = chunk_capacities(fill_count, self.max_fill_vertices);
        let line_capacity = line_caps.iter().copied().sum();
        let fill_capacity = fill_caps.iter().copied().sum();
        if line_capacity > self.line_capacity {
            self.line_chunks = create_vertex_chunks(device, &line_caps, "mycad.linevb");
            self.line_capacity = line_capacity;
        }
        if fill_capacity > self.fill_capacity {
            self.fill_chunks = create_vertex_chunks(device, &fill_caps, "mycad.fillvb");
            self.fill_capacity = fill_capacity;
        }
    }
}

impl UniformSlot {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, label: &str) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        Self { buffer, bind_group }
    }

    fn write(
        &self,
        queue: &wgpu::Queue,
        camera: Camera2,
        origin: Point2,
        aspect: f64,
        overlay_color: [f32; 4],
        overlay_mix: f32,
        premul: f32,
        model: [[f32; 4]; 4],
    ) {
        let uniforms = Uniforms {
            view_proj: camera.view_proj_f32(origin, aspect),
            overlay_color,
            overlay_params: [overlay_mix, premul, 0.0, 0.0],
            model,
        };
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&uniforms));
    }
}

fn create_vertex_chunks(device: &wgpu::Device, caps: &[u32], label: &str) -> Vec<VertexChunk> {
    caps.iter()
        .enumerate()
        .filter_map(|(index, &capacity)| {
            let name = if index == 0 {
                label.to_string()
            } else {
                format!("{label}.{index}")
            };
            Some(VertexChunk {
                buffer: create_vertex_buffer(device, capacity, &name)?,
                capacity,
            })
        })
        .collect()
}

fn create_vertex_buffer(device: &wgpu::Device, capacity: u32, label: &str) -> Option<wgpu::Buffer> {
    if capacity == 0 {
        return None;
    }
    Some(device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: VERTEX_STRIDE * u64::from(capacity),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    }))
}

fn write_vertex_tail(queue: &wgpu::Queue, chunks: &[VertexChunk], start: u32, verts: &[GpuVertex]) {
    write_vertex_range(queue, chunks, start, verts);
}

fn write_upload_ranges(
    queue: &wgpu::Queue,
    chunks: &[VertexChunk],
    ranges: &[UploadRange; MAX_PATCH_RANGES],
    count: u8,
    verts: &[GpuVertex],
) {
    let count = (count as usize).min(MAX_PATCH_RANGES);
    for range in &ranges[..count] {
        if range.end <= range.start || range.start as usize >= verts.len() {
            continue;
        }
        let end = (range.end as usize).min(verts.len());
        write_vertex_range(queue, chunks, range.start, &verts[..end]);
    }
}

fn write_vertex_range(
    queue: &wgpu::Queue,
    chunks: &[VertexChunk],
    start: u32,
    verts: &[GpuVertex],
) {
    let mut offset = start as usize;
    if offset >= verts.len() {
        return;
    }
    let mut remaining = &verts[offset..];
    for chunk in chunks {
        let cap = chunk.capacity as usize;
        if offset >= cap {
            offset -= cap;
            continue;
        }
        let take = remaining.len().min(cap - offset);
        queue.write_buffer(
            &chunk.buffer,
            VERTEX_STRIDE * offset as u64,
            bytemuck::cast_slice(&remaining[..take]),
        );
        remaining = &remaining[take..];
        offset = 0;
        if remaining.is_empty() {
            return;
        }
    }
}

const PREMUL_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
};

fn make_scene_pipelines(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    samples: u32,
) -> ScenePipelines {
    let vertex_layout = &wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GpuVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: 8,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x4,
            },
        ],
    };
    let line = make_pipeline(
        device,
        layout,
        shader,
        format,
        vertex_layout,
        wgpu::PrimitiveTopology::LineList,
        samples,
        Some(wgpu::BlendState::ALPHA_BLENDING),
        "mycad.lines",
    );
    let fill = make_pipeline(
        device,
        layout,
        shader,
        format,
        vertex_layout,
        wgpu::PrimitiveTopology::TriangleList,
        samples,
        Some(wgpu::BlendState::ALPHA_BLENDING),
        "mycad.fill",
    );
    let line_premul = make_pipeline(
        device,
        layout,
        shader,
        format,
        vertex_layout,
        wgpu::PrimitiveTopology::LineList,
        samples,
        Some(PREMUL_BLEND),
        "mycad.lines.premul",
    );
    let fill_premul = make_pipeline(
        device,
        layout,
        shader,
        format,
        vertex_layout,
        wgpu::PrimitiveTopology::TriangleList,
        samples,
        Some(PREMUL_BLEND),
        "mycad.fill.premul",
    );
    ScenePipelines {
        samples,
        line,
        fill,
        line_premul,
        fill_premul,
    }
}

fn make_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    vertex_layout: &wgpu::VertexBufferLayout,
    topology: wgpu::PrimitiveTopology,
    samples: u32,
    blend: Option<wgpu::BlendState>,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: std::slice::from_ref(vertex_layout),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        multiview: None,
        cache: None,
    })
}

const BLIT_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let p = pos[vi];
    var out: VsOut;
    out.pos = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5) * scale.xy;
    return out;
}

@group(0) @binding(0)
var tex: texture_2d<f32>;
@group(0) @binding(1)
var samp: sampler;
@group(0) @binding(2)
var<uniform> scale: vec4<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
"#;

fn make_blit(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("mycad.blit.bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(BLIT_UNIFORM_SIZE),
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("mycad.blit.pll"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("mycad.blit"),
        source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mycad.blit"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(PREMUL_BLEND),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: 1,
            ..Default::default()
        },
        multiview: None,
        cache: None,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("mycad.blit.sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    (pipeline, layout, sampler)
}

/// Paint callback that also writes the camera matrix during prepare.
pub struct CadFrame {
    pub camera: Camera2,
    pub origin: Point2,
    pub generation: u64,
    pub upload: GpuUpload,
    pub display: Arc<DisplayList>,
    pub aspect: f64,
    pub selection: OverlayBatches,
    pub selection_color: [f32; 4],
    pub preview: OverlayBatches,
    pub preview_color: [f32; 4],
    pub preview_model: [[f32; 4]; 4],
    /// Viewport size in physical pixels. The static scene is cached at this size.
    pub viewport_px: [u32; 2],
    /// 1, 2, or 4. Applied to the offscreen target, not the egui surface.
    pub samples: u32,
    /// Opaque color behind the drawing. Matches the viewport panel fill.
    pub clear_color: [f32; 4],
    /// True while the camera moves. The picture renders at 1 sample until it settles.
    pub interactive: bool,
}

/// Identifies a cached raster of the static drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneKey {
    pub generation: u64,
    pub samples: u32,
    pub width: u32,
    pub height: u32,
    pub center_x: u64,
    pub center_y: u64,
    pub view_height: u64,
    pub origin_x: u64,
    pub origin_y: u64,
    pub aspect: u64,
    pub clear: [u32; 4],
}

impl SceneKey {
    pub fn capture(frame: &CadFrame, width: u32, height: u32, samples: u32) -> Self {
        Self {
            generation: frame.generation,
            samples,
            width,
            height,
            center_x: frame.camera.center.x.to_bits(),
            center_y: frame.camera.center.y.to_bits(),
            view_height: frame.camera.view_height.to_bits(),
            origin_x: frame.origin.x.to_bits(),
            origin_y: frame.origin.y.to_bits(),
            aspect: frame.aspect.to_bits(),
            clear: frame.clear_color.map(f32::to_bits),
        }
    }
}

/// True when the cached scene texture cannot be reused.
pub fn scene_needs_redraw(previous: Option<SceneKey>, next: SceneKey) -> bool {
    previous != Some(next)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OverlayKey {
    scene: SceneKey,
    fingerprint: u64,
}

fn overlay_fingerprint(frame: &CadFrame) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    hash = mix_color(hash, frame.selection_color);
    hash = mix_color(hash, frame.preview_color);
    for row in frame.preview_model {
        for value in row {
            hash = mix_u64(hash, value.to_bits() as u64);
        }
    }
    hash = mix_ranges(hash, &frame.selection.lines);
    hash = mix_ranges(hash, &frame.selection.fills);
    hash = mix_ranges(hash, &frame.preview.lines);
    mix_ranges(hash, &frame.preview.fills)
}

fn mix_u64(hash: u64, value: u64) -> u64 {
    hash.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(value)
}

fn mix_color(hash: u64, color: [f32; 4]) -> u64 {
    color.into_iter().fold(hash, |hash, channel| {
        mix_u64(hash, channel.to_bits() as u64)
    })
}

fn mix_ranges(mut hash: u64, ranges: &[std::ops::Range<u32>]) -> u64 {
    hash = mix_u64(hash, ranges.len() as u64);
    for range in ranges {
        hash = mix_u64(hash, u64::from(range.start));
        hash = mix_u64(hash, u64::from(range.end));
    }
    hash
}

pub fn sanitize_viewport_samples(samples: u32) -> u32 {
    match samples {
        1 => 1,
        2 => 2,
        _ => 4,
    }
}

impl egui_wgpu::CallbackTrait for CadFrame {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(gpu) = resources.get_mut::<CadGpu>() else {
            return Vec::new();
        };
        gpu.upload(device, queue, &self.display, self.generation, self.upload);
        gpu.refresh_pictures(device, queue, encoder, self);
        Vec::new()
    }

    fn paint(
        &self,
        info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(gpu) = resources.get::<CadGpu>() else {
            return;
        };
        let vp = info.viewport_in_pixels();
        if vp.width_px == 0 || vp.height_px == 0 {
            return;
        }
        let Some(scene) = gpu.scene_picture.as_ref() else {
            return;
        };
        render_pass.set_pipeline(&gpu.blit_pipeline);
        render_pass.set_bind_group(0, &scene.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
        if !gpu.overlay_visible {
            return;
        }
        if let Some(overlay) = gpu.overlay_picture.as_ref() {
            render_pass.set_bind_group(0, &overlay.bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }
    }
}

fn draw_scene(gpu: &CadGpu, pipelines: &ScenePipelines, render_pass: &mut wgpu::RenderPass<'_>) {
    let has_fill = !gpu.fill_chunks.is_empty() && gpu.fill_count >= 3;
    let has_lines = !gpu.line_chunks.is_empty() && gpu.line_count >= 2;
    if !has_fill && !has_lines {
        return;
    }
    render_pass.set_bind_group(0, &gpu.scene.bind_group, &[]);
    if has_fill {
        render_pass.set_pipeline(&pipelines.fill);
        draw_chunks(render_pass, &gpu.fill_chunks, gpu.fill_count, 3);
    }
    if has_lines {
        render_pass.set_pipeline(&pipelines.line);
        draw_chunks(render_pass, &gpu.line_chunks, gpu.line_count, 2);
    }
}

fn draw_chunks(
    render_pass: &mut wgpu::RenderPass<'_>,
    chunks: &[VertexChunk],
    count: u32,
    min_verts: u32,
) {
    let mut remaining = count;
    for chunk in chunks {
        if remaining == 0 {
            break;
        }
        let n = remaining.min(chunk.capacity);
        if n >= min_verts {
            render_pass.set_vertex_buffer(0, chunk.buffer.slice(..));
            render_pass.draw(0..n, 0..1);
        }
        remaining -= n;
    }
}

fn draw_overlay(
    gpu: &CadGpu,
    pipelines: &ScenePipelines,
    render_pass: &mut wgpu::RenderPass<'_>,
    bind_group: &wgpu::BindGroup,
    overlay: &OverlayBatches,
) {
    if overlay.is_empty() {
        return;
    }
    render_pass.set_bind_group(0, bind_group, &[]);
    if !overlay.fills.is_empty() && !gpu.fill_chunks.is_empty() {
        render_pass.set_pipeline(&pipelines.fill_premul);
        for range in &overlay.fills {
            draw_global_range(render_pass, &gpu.fill_chunks, range.start, range.end, 3);
        }
    }
    if !overlay.lines.is_empty() && !gpu.line_chunks.is_empty() {
        render_pass.set_pipeline(&pipelines.line_premul);
        for range in &overlay.lines {
            draw_global_range(render_pass, &gpu.line_chunks, range.start, range.end, 2);
        }
    }
}

fn draw_global_range(
    render_pass: &mut wgpu::RenderPass<'_>,
    chunks: &[VertexChunk],
    start: u32,
    end: u32,
    min_verts: u32,
) {
    let mut base = 0u32;
    for chunk in chunks {
        let chunk_end = base.saturating_add(chunk.capacity);
        let lo = start.max(base);
        let hi = end.min(chunk_end);
        if hi.saturating_sub(lo) >= min_verts {
            render_pass.set_vertex_buffer(0, chunk.buffer.slice(..));
            render_pass.draw(lo - base..hi - base, 0..1);
        }
        base = chunk_end;
        if base >= end {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        chunk_capacities, max_vertices_for_buffer, motion_samples, next_vertex_capacity,
        picture_texture_size, plan_gpu_upload, scene_needs_redraw, viewport_pixel_size, GpuUpload,
        GpuUploadPlan, SceneKey, UploadRange, DEFAULT_MAX_BUFFER_SIZE, MAX_PATCH_RANGES,
        VERTEX_STRIDE,
    };
    use cad_core::Transform2;

    #[test]
    fn overlay_uniforms_are_160_bytes() {
        assert_eq!(super::UNIFORM_SIZE, 160);
        assert_eq!(std::mem::size_of::<super::Uniforms>(), 160);
    }

    #[test]
    fn large_coordinate_preview_model_matches_local_origin_transform() {
        let origin = cad_core::Point2::new(1.0e8, -5.0e7);
        let world = Transform2::translate(12.0, 8.0);
        let local = world.to_local_origin(origin);
        let stored = cad_core::Point2::new(4.0, 1.0);
        let preview = local.apply(stored);
        assert!((preview.x - 16.0).abs() < 1e-6);
        assert!((preview.y - 9.0).abs() < 1e-6);
        let model = local.to_mat4();
        assert!((model[3][0] - 12.0).abs() < 1e-4);
        assert!((model[3][1] - 8.0).abs() < 1e-4);
    }

    #[test]
    fn an_unchanged_scene_key_skips_the_redraw() {
        let key = SceneKey {
            generation: 3,
            samples: 4,
            width: 800,
            height: 600,
            center_x: 0.0f64.to_bits(),
            center_y: 0.0f64.to_bits(),
            view_height: 100.0f64.to_bits(),
            origin_x: 0.0f64.to_bits(),
            origin_y: 0.0f64.to_bits(),
            aspect: 1.0f64.to_bits(),
            clear: [0.0f32, 0.0, 0.0, 1.0].map(f32::to_bits),
        };
        assert!(!scene_needs_redraw(Some(key), key));
        assert!(scene_needs_redraw(None, key));
        let mut moved = key;
        moved.center_x = 1.0f64.to_bits();
        assert!(scene_needs_redraw(Some(key), moved));
        let mut edited = key;
        edited.generation = 4;
        assert!(scene_needs_redraw(Some(key), edited));
        let mut recolored = key;
        recolored.clear = [0.1f32, 0.1, 0.1, 1.0].map(f32::to_bits);
        assert!(scene_needs_redraw(Some(key), recolored));
        let mut settled = key;
        settled.samples = 1;
        assert!(scene_needs_redraw(Some(key), settled));
    }

    #[test]
    fn viewport_pixels_match_egui_edge_rounding() {
        assert_eq!(
            viewport_pixel_size([0.5, 0.0], [13.0, 10.0], 1.0),
            [12, 10]
        );
        assert_eq!(
            viewport_pixel_size([100.3, 40.0], [900.3, 640.0], 1.25),
            [1000, 750]
        );
        assert_eq!(viewport_pixel_size([10.0, 10.0], [5.0, 5.0], 2.0), [0, 0]);
    }

    #[test]
    fn picture_textures_grow_in_slack_steps_and_are_reused() {
        let max = 8192;
        assert_eq!(
            picture_texture_size(None, [1000, 700], max),
            Some([1024, 768])
        );
        assert_eq!(
            picture_texture_size(Some([1024, 768]), [1010, 700], max),
            None
        );
        assert_eq!(
            picture_texture_size(Some([1024, 768]), [1030, 700], max),
            Some([1280, 768])
        );
        assert_eq!(picture_texture_size(None, [9000, 100], max), Some([8192, 256]));
        assert_eq!(
            picture_texture_size(Some([4096, 4096]), [300, 300], max),
            Some([512, 512])
        );
        assert_eq!(picture_texture_size(Some([256, 256]), [10, 10], max), None);
    }

    #[test]
    fn an_empty_overlay_is_skipped() {
        let empty = crate::OverlayBatches::default();
        assert!(!super::overlay_has_content(&empty, &empty));
        let selected = crate::OverlayBatches {
            lines: vec![0..2],
            fills: Vec::new(),
        };
        assert!(super::overlay_has_content(&selected, &empty));
        assert!(super::overlay_has_content(&empty, &selected));
    }

    #[test]
    fn moving_views_render_at_one_sample() {
        assert_eq!(motion_samples(true, 4), 1);
        assert_eq!(motion_samples(false, 4), 4);
        assert_eq!(motion_samples(false, 2), 2);
        assert_eq!(motion_samples(false, 7), 4);
    }

    #[test]
    fn matching_generation_skips_upload() {
        assert_eq!(
            plan_gpu_upload(4, 4, 10, 0, 16, 0, 12, 0, GpuUpload::Full),
            GpuUploadPlan::Skip
        );
    }

    #[test]
    fn append_hint_writes_only_the_new_tail_when_it_fits() {
        assert_eq!(
            plan_gpu_upload(
                1,
                2,
                100,
                0,
                1024,
                0,
                102,
                0,
                GpuUpload::Append {
                    line_start: 100,
                    fill_start: 0
                }
            ),
            GpuUploadPlan::Append {
                line_start: 100,
                fill_start: 0
            }
        );
    }

    #[test]
    fn append_grows_when_capacity_is_exhausted() {
        assert_eq!(
            plan_gpu_upload(
                1,
                2,
                100,
                0,
                100,
                0,
                102,
                0,
                GpuUpload::Append {
                    line_start: 100,
                    fill_start: 0
                }
            ),
            GpuUploadPlan::GrowAndFull
        );
    }

    #[test]
    fn missed_append_falls_back_to_a_full_write() {
        assert_eq!(
            plan_gpu_upload(
                1,
                3,
                100,
                0,
                1024,
                0,
                104,
                0,
                GpuUpload::Append {
                    line_start: 102,
                    fill_start: 0
                }
            ),
            GpuUploadPlan::Full
        );
    }

    #[test]
    fn a_patch_writes_only_the_rewritten_spans() {
        let mut lines = [UploadRange { start: 0, end: 0 }; MAX_PATCH_RANGES];
        lines[0] = UploadRange { start: 4, end: 8 };
        lines[1] = UploadRange {
            start: 40,
            end: 44,
        };
        assert_eq!(
            plan_gpu_upload(
                1,
                2,
                40,
                0,
                1024,
                0,
                44,
                0,
                GpuUpload::Patch {
                    lines,
                    line_count: 2,
                    fills: [UploadRange { start: 0, end: 0 }; MAX_PATCH_RANGES],
                    fill_count: 0,
                }
            ),
            GpuUploadPlan::Patch {
                lines,
                line_count: 2,
                fills: [UploadRange { start: 0, end: 0 }; MAX_PATCH_RANGES],
                fill_count: 0,
            }
        );
    }

    #[test]
    fn a_patch_past_the_buffer_grows() {
        let mut lines = [UploadRange { start: 0, end: 0 }; MAX_PATCH_RANGES];
        lines[0] = UploadRange {
            start: 90,
            end: 110,
        };
        assert_eq!(
            plan_gpu_upload(
                1,
                2,
                100,
                0,
                100,
                0,
                110,
                0,
                GpuUpload::Patch {
                    lines,
                    line_count: 1,
                    fills: [UploadRange { start: 0, end: 0 }; MAX_PATCH_RANGES],
                    fill_count: 0,
                }
            ),
            GpuUploadPlan::GrowAndFull
        );
    }

    #[test]
    fn full_rebuild_reuses_capacity_when_it_fits() {
        assert_eq!(
            plan_gpu_upload(8, 9, 50, 6, 1024, 64, 40, 3, GpuUpload::Full),
            GpuUploadPlan::Full
        );
    }

    #[test]
    fn capacity_does_not_round_up_past_the_gpu_buffer_limit() {
        let max_verts = max_vertices_for_buffer(DEFAULT_MAX_BUFFER_SIZE, 2);
        let just_over_power_of_two = (1u32 << 23) + 2;
        let capacity = next_vertex_capacity(just_over_power_of_two, max_verts);
        assert!(capacity >= just_over_power_of_two);
        assert!(u64::from(capacity) * VERTEX_STRIDE <= DEFAULT_MAX_BUFFER_SIZE);
        assert!(
            u64::from(capacity.next_power_of_two()) * VERTEX_STRIDE > DEFAULT_MAX_BUFFER_SIZE,
            "this case is the one that used to allocate 384 MiB"
        );
    }

    #[test]
    fn oversized_meshes_split_into_gpu_sized_chunks() {
        let max_verts = max_vertices_for_buffer(DEFAULT_MAX_BUFFER_SIZE, 2);
        let needed = max_verts.saturating_mul(2).saturating_add(4);
        let caps = chunk_capacities(needed, max_verts);
        assert!(caps.len() >= 3);
        assert!(caps.iter().all(|&cap| cap <= max_verts));
        assert!(caps
            .iter()
            .all(|&cap| u64::from(cap) * VERTEX_STRIDE <= DEFAULT_MAX_BUFFER_SIZE));
        assert!(caps.iter().copied().sum::<u32>() >= needed);
    }

    #[test]
    fn small_uploads_still_use_power_of_two_capacity() {
        assert_eq!(
            next_vertex_capacity(2000, max_vertices_for_buffer(DEFAULT_MAX_BUFFER_SIZE, 2)),
            2048
        );
    }
}
