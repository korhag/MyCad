struct Uniforms {
    view_proj: mat4x4<f32>,
    overlay_color: vec4<f32>,
    overlay_params: vec4<f32>,
    model: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) depth: f32,
};

struct WeightInput {
    @location(0) position: vec2<f32>,
    @location(1) other: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) depth: f32,
    @location(4) side: f32,
    @location(5) half_px: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = uniforms.view_proj * uniforms.model * vec4<f32>(in.position, 0.0, 1.0);
    out.clip_position.z = in.depth * out.clip_position.w;
    out.color = mix(in.color, uniforms.overlay_color, uniforms.overlay_params.x);
    return out;
}

@vertex
fn vs_weight(in: WeightInput) -> VertexOutput {
    var out: VertexOutput;
    let model_a = uniforms.model * vec4<f32>(in.position, 0.0, 1.0);
    let model_b = uniforms.model * vec4<f32>(in.other, 0.0, 1.0);
    var clip = uniforms.view_proj * model_a;
    let clip_b = uniforms.view_proj * model_b;
    let width = max(uniforms.overlay_params.w, 1.0);
    let height = max(uniforms.overlay_params.z, 1.0);
    let px = vec2<f32>(width, height) * 0.5;
    let pix_a = (clip.xy / max(clip.w, 1e-6)) * px;
    let pix_b = (clip_b.xy / max(clip_b.w, 1e-6)) * px;
    var dir = pix_b - pix_a;
    let len = length(dir);
    if (len > 1e-4) {
        dir = dir / len;
    } else {
        dir = vec2<f32>(1.0, 0.0);
    }
    let perp = vec2<f32>(-dir.y, dir.x);
    let shift = (perp * in.side - dir) * in.half_px;
    let ndc = shift * vec2<f32>(2.0 / width, 2.0 / height);
    clip.x = clip.x + ndc.x * clip.w;
    clip.y = clip.y + ndc.y * clip.w;
    clip.z = in.depth * clip.w;
    out.clip_position = clip;
    out.color = mix(in.color, uniforms.overlay_color, uniforms.overlay_params.x);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    if (in.color.a <= 0.0) {
        discard;
    }
    let straight = in.color;
    let premul = vec4<f32>(straight.rgb * straight.a, straight.a);
    return mix(straight, premul, uniforms.overlay_params.y);
}
