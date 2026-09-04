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
    line_pipeline: wgpu::RenderPipeline,
    fill_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
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
}

struct VertexChunk {
    buffer: wgpu::Buffer,
    capacity: u32,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuUpload {
    #[default]
    Full,
    Append {
        line_start: u32,
        fill_start: u32,
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
    if let GpuUpload::Append {
        line_start,
        fill_start,
    } = kind
    {
        let fits = new_line_count <= line_capacity && new_fill_count <= fill_capacity;
        let contiguous = line_start == uploaded_line_count && fill_start == uploaded_fill_count;
        let ordered = new_line_count >= line_start && new_fill_count >= fill_start;
        if fits && contiguous && ordered {
            return GpuUploadPlan::Append {
                line_start,
                fill_start,
            };
        }
    }
    if new_line_count <= line_capacity && new_fill_count <= fill_capacity {
        GpuUploadPlan::Full
    } else {
        GpuUploadPlan::GrowAndFull
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
                visibility: wgpu::ShaderStages::VERTEX,
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
        let vertex_layout = wgpu::VertexBufferLayout {
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
        let line_pipeline = make_pipeline(
            device,
            &pipeline_layout,
            &shader,
            target_format,
            &vertex_layout,
            wgpu::PrimitiveTopology::LineList,
            "mycad.lines",
        );
        let fill_pipeline = make_pipeline(
            device,
            &pipeline_layout,
            &shader,
            target_format,
            &vertex_layout,
            wgpu::PrimitiveTopology::TriangleList,
            "mycad.fill",
        );
        Self {
            line_pipeline,
            fill_pipeline,
            scene: UniformSlot::new(device, &bind_group_layout, "mycad.scene"),
            selection: UniformSlot::new(device, &bind_group_layout, "mycad.selection"),
            preview: UniformSlot::new(device, &bind_group_layout, "mycad.preview"),
            bind_group_layout,
            line_chunks: Vec::new(),
            fill_chunks: Vec::new(),
            line_count: 0,
            fill_count: 0,
            line_capacity: 0,
            fill_capacity: 0,
            max_line_vertices: max_vertices_for_buffer(device_max_buffer_size(device), 2),
            max_fill_vertices: max_vertices_for_buffer(device_max_buffer_size(device), 3),
            uploaded_generation: 0,
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
        let _ = &self.bind_group_layout;
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
        model: [[f32; 4]; 4],
    ) {
        let uniforms = Uniforms {
            view_proj: camera.view_proj_f32(origin, aspect),
            overlay_color,
            overlay_params: [overlay_mix, 0.0, 0.0, 0.0],
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

fn write_vertex_tail(
    queue: &wgpu::Queue,
    chunks: &[VertexChunk],
    start: u32,
    verts: &[GpuVertex],
) {
    write_vertex_range(queue, chunks, start, verts);
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

fn make_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    vertex_layout: &wgpu::VertexBufferLayout,
    topology: wgpu::PrimitiveTopology,
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
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
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
}

impl egui_wgpu::CallbackTrait for CadFrame {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(gpu) = resources.get_mut::<CadGpu>() else {
            return Vec::new();
        };
        gpu.upload(device, queue, &self.display, self.generation, self.upload);
        gpu.scene.write(
            queue,
            self.camera,
            self.origin,
            self.aspect,
            [0.0; 4],
            0.0,
            Transform2::identity_mat4(),
        );
        gpu.selection.write(
            queue,
            self.camera,
            self.origin,
            self.aspect,
            self.selection_color,
            1.0,
            Transform2::identity_mat4(),
        );
        gpu.preview.write(
            queue,
            self.camera,
            self.origin,
            self.aspect,
            self.preview_color,
            1.0,
            self.preview_model,
        );
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
        draw_scene(gpu, render_pass);
        draw_overlay(gpu, render_pass, &gpu.selection.bind_group, &self.selection);
        draw_overlay(gpu, render_pass, &gpu.preview.bind_group, &self.preview);
    }
}

fn draw_scene(gpu: &CadGpu, render_pass: &mut wgpu::RenderPass<'static>) {
    let has_fill = !gpu.fill_chunks.is_empty() && gpu.fill_count >= 3;
    let has_lines = !gpu.line_chunks.is_empty() && gpu.line_count >= 2;
    if !has_fill && !has_lines {
        return;
    }
    render_pass.set_bind_group(0, &gpu.scene.bind_group, &[]);
    if has_fill {
        render_pass.set_pipeline(&gpu.fill_pipeline);
        draw_chunks(render_pass, &gpu.fill_chunks, gpu.fill_count, 3);
    }
    if has_lines {
        render_pass.set_pipeline(&gpu.line_pipeline);
        draw_chunks(render_pass, &gpu.line_chunks, gpu.line_count, 2);
    }
}

fn draw_chunks(
    render_pass: &mut wgpu::RenderPass<'static>,
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
    render_pass: &mut wgpu::RenderPass<'static>,
    bind_group: &wgpu::BindGroup,
    overlay: &OverlayBatches,
) {
    if overlay.is_empty() {
        return;
    }
    render_pass.set_bind_group(0, bind_group, &[]);
    if !overlay.fills.is_empty() && !gpu.fill_chunks.is_empty() {
        render_pass.set_pipeline(&gpu.fill_pipeline);
        for range in &overlay.fills {
            draw_global_range(render_pass, &gpu.fill_chunks, range.start, range.end, 3);
        }
    }
    if !overlay.lines.is_empty() && !gpu.line_chunks.is_empty() {
        render_pass.set_pipeline(&gpu.line_pipeline);
        for range in &overlay.lines {
            draw_global_range(render_pass, &gpu.line_chunks, range.start, range.end, 2);
        }
    }
}

fn draw_global_range(
    render_pass: &mut wgpu::RenderPass<'static>,
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
        chunk_capacities, max_vertices_for_buffer, next_vertex_capacity, plan_gpu_upload,
        GpuUpload, GpuUploadPlan, DEFAULT_MAX_BUFFER_SIZE, VERTEX_STRIDE,
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
