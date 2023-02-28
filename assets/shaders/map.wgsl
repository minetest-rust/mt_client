// Vertex shader

struct VertexInput {
	@location(0) pos: vec3<f32>,
	@location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
	@builtin(position) pos: vec4<f32>,
	@location(0) tex_coords: vec2<f32>,
}

@group(1) @binding(0) var<uniform> view_proj: mat4x4<f32>;
@group(2) @binding(0) var<uniform> model: mat4x4<f32>;

@vertex
fn vs_main(
	in: VertexInput,
) -> VertexOutput {
	var out: VertexOutput;
	out.pos = view_proj * model * vec4<f32>(in.pos, 1.0);
	out.tex_coords = in.tex_coords;
	return out;
}

// Fragment shader

@group(0) @binding(0) var atlas_texture: texture_2d<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
	return textureSample(atlas_texture, atlas_sampler, in.tex_coords);
}
