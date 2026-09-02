//! wgpu CAD viewport renderer, kept separate from application chrome.

use std::sync::Arc;

use cad_viewport::Camera2;
use egui::PaintCallbackInfo;
use egui_wgpu::wgpu;
use egui_wgpu::wgpu::util::DeviceExt;

use crate::tessellate::{DisplayList, GpuVertex};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
}

// ------------------------------------------------------------
// Type: CadGpu
// Purpose: Persistent wgpu resources stored in egui callback_resources.
// ------------------------------------------------------------
pub struct CadGpu {
    line_pipeline: wgpu::RenderPipeline,
    fill_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    line_buffer: Option<wgpu::Buffer>,
    fill_buffer: Option<wgpu::Buffer>,
    line_count: u32,
    fill_count: u32,
    uploaded_generation: u64,
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
                    min_binding_size: None,
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
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mycad.uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mycad.bg"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        Self {
            line_pipeline,
            fill_pipeline,
            bind_group_layout,
            uniform_buffer,
            bind_group,
            line_buffer: None,
            fill_buffer: None,
            line_count: 0,
            fill_count: 0,
            uploaded_generation: 0,
        }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, list: &DisplayList, generation: u64) {
        if generation != self.uploaded_generation {
            self.line_buffer = upload_vertices(device, &list.line_vertices, "mycad.linevb");
            self.fill_buffer = upload_vertices(device, &list.triangle_vertices, "mycad.fillvb");
            self.line_count = list.line_vertices.len() as u32;
            self.fill_count = list.triangle_vertices.len() as u32;
            self.uploaded_generation = generation;
        }
        let _ = queue;
        let _ = &self.bind_group_layout;
    }
}

fn upload_vertices(device: &wgpu::Device, verts: &[GpuVertex], label: &str) -> Option<wgpu::Buffer> {
    if verts.is_empty() {
        return None;
    }
    Some(
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(verts),
            usage: wgpu::BufferUsages::VERTEX,
        }),
    )
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

use cad_core::Point2;

impl CadGpu {
    pub fn write_camera(&self, queue: &wgpu::Queue, camera: Camera2, origin: Point2, aspect: f64) {
        let uniforms = Uniforms {
            view_proj: camera.view_proj_f32(origin, aspect),
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }
}

/// Paint callback that also writes the camera matrix during prepare.
pub struct CadFrame {
    pub camera: Camera2,
    pub origin: Point2,
    pub generation: u64,
    pub display: Arc<DisplayList>,
    pub aspect: f64,
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
        gpu.upload(device, queue, &self.display, self.generation);
        gpu.write_camera(queue, self.camera, self.origin, self.aspect);
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
        render_pass.set_bind_group(0, &gpu.bind_group, &[]);
        if let Some(buf) = gpu.fill_buffer.as_ref() {
            if gpu.fill_count >= 3 {
                render_pass.set_pipeline(&gpu.fill_pipeline);
                render_pass.set_vertex_buffer(0, buf.slice(..));
                render_pass.draw(0..gpu.fill_count, 0..1);
            }
        }
        if let Some(buf) = gpu.line_buffer.as_ref() {
            if gpu.line_count >= 2 {
                render_pass.set_pipeline(&gpu.line_pipeline);
                render_pass.set_vertex_buffer(0, buf.slice(..));
                render_pass.draw(0..gpu.line_count, 0..1);
            }
        }
    }
}
