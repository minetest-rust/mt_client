mod atlas;
mod mesh;

use super::{media::MediaMgr, state::State, util::MatrixUniform};
use atlas::create_atlas;
use cgmath::{prelude::*, Matrix4, Point3, Vector3};
use collision::{prelude::*, Aabb3, Relation};
use mesh::{create_mesh, MeshData};
use mt_net::{MapBlock, NodeDef};
use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::{Entry, HashMap},
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex, RwLock},
    time::Instant,
};
use wgpu::util::DeviceExt;

#[derive(Serialize, Deserialize, PartialEq, Eq, Copy, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LeavesMode {
    Opaque,
    Simple,
    Fancy,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MapRenderSettings {
    pub leaves: LeavesMode,
    pub opaque_liquids: bool,
}

impl Default for MapRenderSettings {
    fn default() -> Self {
        Self {
            leaves: LeavesMode::Fancy,
            opaque_liquids: false,
        }
    }
}

struct AtlasSlice {
    cube_tex_coords: [[[f32; 2]; 6]; 6],
}

// data shared with meshgen threads
struct MeshgenInfo {
    // i optimized the shit out of these
    textures: Vec<AtlasSlice>,
    nodes: [Option<Box<NodeDef>>; u16::MAX as usize + 1],
}

type MeshQueue = HashMap<Point3<i16>, MeshData>;

// to avoid excessive block mesh rebuilds, only build a mesh once all 6 neighbors are present
// or a timeout of 100ms has elapsed
struct DeferredBlock {
    count: u8,
    mask: [bool; 6],
    time: Instant,
}

pub struct MapRender {
    pipeline: wgpu::RenderPipeline,
    atlas: wgpu::BindGroup,
    model: wgpu::BindGroupLayout,
    blocks: Arc<RwLock<HashMap<Point3<i16>, Arc<MapBlock>>>>,
    blocks_defer: HashMap<Point3<i16>, DeferredBlock>,
    block_models: HashMap<Point3<i16>, BlockModel>,
    meshgen_info: Arc<MeshgenInfo>,
    meshgen_threads: Vec<std::thread::JoinHandle<()>>,
    meshgen_channel: crossbeam_channel::Sender<Point3<i16>>,
    queue_consume: MeshQueue,
    queue_produce: Arc<Mutex<MeshQueue>>,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    pos: [f32; 3],
    tex_coords: [f32; 2],
    light: f32,
}

impl Vertex {
    const ATTRIBS: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32];

    fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

struct BlockMesh {
    vertex_buffer: wgpu::Buffer,
    num_vertices: u32,
}

impl BlockMesh {
    fn new(state: &State, vertices: &[Vertex]) -> Option<Self> {
        if vertices.is_empty() {
            return None;
        }

        Some(BlockMesh {
            vertex_buffer: state
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mapblock.vertex_buffer"),
                    contents: bytemuck::cast_slice(vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
            num_vertices: vertices.len() as u32,
        })
    }

    fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, transform: &'a MatrixUniform) {
        pass.set_bind_group(2, &transform.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..self.num_vertices, 0..1);
    }
}

struct BlockModel {
    mesh: Option<BlockMesh>,
    mesh_blend: Option<BlockMesh>,
    transform: MatrixUniform,
}

fn block_float_pos(pos: Point3<i16>) -> Point3<f32> {
    pos.cast::<f32>().unwrap() * 16.0
}

impl MapRender {
    pub fn render<'a>(&'a self, state: &'a State, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.atlas, &[]);
        pass.set_bind_group(1, &state.camera_uniform.bind_group, &[]);

        struct BlendEntry<'a> {
            dist: f32,
            index: usize,
            mesh: &'a BlockMesh,
            transform: &'a MatrixUniform,
        }

        let mut blend = Vec::new();

        for (index, (&pos, model)) in self.block_models.iter().enumerate() {
            if model.mesh.is_none() && model.mesh_blend.is_none() {
                continue;
            }

            let fpos = block_float_pos(pos);
            let one = Vector3::new(1.0, 1.0, 1.0);
            let aabb = Aabb3::new(fpos - one * 0.5, fpos + one * 15.5).transform(&state.view);

            if state.frustum.contains(&aabb) == Relation::Out {
                continue;
            }

            if let Some(mesh) = &model.mesh {
                mesh.render(pass, &model.transform);
            }

            if let Some(mesh) = &model.mesh_blend {
                blend.push(BlendEntry {
                    index,
                    dist: (state.view * (fpos + one * 8.5).to_homogeneous())
                        .truncate()
                        .magnitude(),
                    mesh,
                    transform: &model.transform,
                });
            }
        }

        blend.sort_unstable_by(|a, b| {
            a.dist
                .partial_cmp(&b.dist)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.index.cmp(&b.index))
        });

        for entry in blend {
            entry.mesh.render(pass, entry.transform);
        }
    }

    pub fn update(&mut self, state: &mut State) {
        for (pos, _) in self
            .blocks_defer
            .drain_filter(|_, v| v.time.elapsed().as_millis() > 100)
        {
            self.meshgen_channel.send(pos).ok();
        }

        std::mem::swap(
            self.queue_produce.lock().unwrap().deref_mut(),
            &mut self.queue_consume,
        );

        for (pos, data) in self.queue_consume.drain() {
            self.block_models.insert(
                pos,
                BlockModel {
                    mesh: BlockMesh::new(state, &data.vertices),
                    mesh_blend: BlockMesh::new(state, &data.vertices_blend),
                    transform: MatrixUniform::new(
                        &state.device,
                        &self.model,
                        Matrix4::from_translation(block_float_pos(pos).to_vec()),
                        "mapblock",
                        false,
                    ),
                },
            );
        }
    }

    pub fn add_block(&mut self, pos: Point3<i16>, block: Box<MapBlock>) {
        self.blocks.write().unwrap().insert(pos, Arc::new(*block));

        let blocks = self.blocks.read().unwrap();

        let mut count = 6;
        let mut mask = [false; 6];

        for (f, off) in FACE_DIR.iter().enumerate() {
            let npos = pos + Vector3::from(*off);

            if let Entry::Occupied(mut nbor) = self.blocks_defer.entry(npos) {
                let inner = nbor.get_mut();

                let rf = f ^ 1;

                if !inner.mask[rf] {
                    inner.mask[rf] = true;
                    inner.count -= 1;

                    if inner.count == 0 {
                        self.meshgen_channel.send(npos).ok();
                        nbor.remove();
                    }
                }
            } else if blocks.get(&npos).is_some() {
                self.meshgen_channel.send(npos).ok();
            } else {
                continue;
            }

            mask[f] = true;
            count -= 1;
        }

        if count == 0 {
            self.meshgen_channel.send(pos).ok();
        } else {
            match self.blocks_defer.entry(pos) {
                Entry::Occupied(mut x) => {
                    let x = x.get_mut();
                    x.mask = mask;
                    x.count = count;
                }
                Entry::Vacant(x) => {
                    x.insert(DeferredBlock {
                        mask,
                        count,
                        time: Instant::now(),
                    });
                }
            }
        }
    }

    pub fn new(state: &mut State, media: &MediaMgr, mut nodes: HashMap<u16, NodeDef>) -> Self {
        let (atlas_img, atlas_slices) = create_atlas(&mut nodes, media);

        let atlas_size = wgpu::Extent3d {
            width: atlas_img.width(),
            height: atlas_img.height(),
            depth_or_array_layers: 1,
        };

        let atlas_texture = state.device.create_texture(&wgpu::TextureDescriptor {
            size: atlas_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            label: Some("tile_atlas"),
            view_formats: &[],
        });

        state.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &atlas_img,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: std::num::NonZeroU32::new(4 * atlas_img.width()),
                rows_per_image: std::num::NonZeroU32::new(atlas_img.height()),
            },
            atlas_size,
        );

        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let atlas_sampler = state.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            // "We've got you surrounded, stop using Nearest filter"
            // - "I hate bilinear filtering I hate bilinear filtering I hate bilinear filtering"
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let atlas_bind_group_layout =
            state
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                multisampled: false,
                                view_dimension: wgpu::TextureViewDimension::D2,
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                    label: Some("atlas.bind_group_layout"),
                });

        let atlas_bind_group = state.device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &atlas_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
            ],
            label: Some("atlas.bind_group"),
        });

        let model_bind_group_layout = MatrixUniform::layout(&state.device, "mapblock");

        let shader = state
            .device
            .create_shader_module(wgpu::include_wgsl!("../../assets/shaders/map.wgsl"));

        let pipeline_layout =
            state
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[
                        &atlas_bind_group_layout,
                        &model_bind_group_layout,
                        &state.camera_bind_group_layout,
                    ],
                    push_constant_ranges: &[],
                });

        let pipeline = state
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: "vs_main",
                    buffers: &[Vertex::desc()],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: "fs_main",
                    targets: &[Some(wgpu::ColorTargetState {
                        format: state.config.format,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::SrcAlpha,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent::OVER,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: 1,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview: None,
            });

        let meshgen_queue = Arc::new(Mutex::new(HashMap::new()));
        let meshgen_info = Arc::new(MeshgenInfo {
            nodes: std::array::from_fn(|i| nodes.get(&(i as u16)).cloned().map(Box::new)),
            textures: atlas_slices,
        });
        let mut meshgen_threads = Vec::new();
        let (meshgen_tx, meshgen_rx) = crossbeam_channel::unbounded();

        let blocks = Arc::new(RwLock::new(HashMap::<Point3<i16>, Arc<MapBlock>>::new()));

        // TODO: make this configurable
        for _ in 0..2 {
            let input = meshgen_rx.clone();
            let output = meshgen_queue.clone();
            let info = meshgen_info.clone();
            let config = Default::default();
            let blocks = blocks.clone();

            meshgen_threads.push(std::thread::spawn(move || {
                let mut buffer_cap = 0;
                let info = info.deref();

                while let Ok(pos) = input.recv() {
                    let mut data = MeshData::new(buffer_cap);

                    let blocks = blocks.read().unwrap();

                    let block = match blocks.get(&pos) {
                        Some(x) => x.clone(),
                        None => continue,
                    };

                    let nbors: [_; 6] = std::array::from_fn(|i| {
                        blocks.get(&(pos + Vector3::from(FACE_DIR[i]))).cloned()
                    });

                    drop(blocks);

                    create_mesh(
                        info,
                        &config,
                        pos,
                        block.deref(),
                        std::array::from_fn(|i| nbors[i].as_deref()),
                        &mut data,
                    );

                    drop(block);
                    drop(nbors);

                    buffer_cap = data.cap();
                    output.lock().unwrap().insert(pos, data);
                }
            }));
        }

        Self {
            pipeline,
            atlas: atlas_bind_group,
            model: model_bind_group_layout,
            blocks,
            blocks_defer: HashMap::new(),
            block_models: HashMap::new(),
            meshgen_info,
            meshgen_threads,
            meshgen_channel: meshgen_tx,
            queue_consume: HashMap::new(), // store this to keep capacity/allocations around
            queue_produce: meshgen_queue,
        }
    }
}

#[rustfmt::skip]
const CUBE: [[([f32; 3], [f32; 2]); 6]; 6] = [
	[
		([-0.5,  0.5, -0.5], [ 0.0,  1.0]),
		([ 0.5,  0.5,  0.5], [ 1.0,  0.0]),
		([ 0.5,  0.5, -0.5], [ 1.0,  1.0]),
		([ 0.5,  0.5,  0.5], [ 1.0,  0.0]),
		([-0.5,  0.5, -0.5], [ 0.0,  1.0]),
		([-0.5,  0.5,  0.5], [ 0.0,  0.0]),
	],
	[
		([-0.5, -0.5, -0.5], [ 0.0,  1.0]),
		([ 0.5, -0.5, -0.5], [ 1.0,  1.0]),
		([ 0.5, -0.5,  0.5], [ 1.0,  0.0]),
		([ 0.5, -0.5,  0.5], [ 1.0,  0.0]),
		([-0.5, -0.5,  0.5], [ 0.0,  0.0]),
		([-0.5, -0.5, -0.5], [ 0.0,  1.0]),
	],
	[
		([ 0.5,  0.5,  0.5], [ 1.0,  1.0]),
		([ 0.5, -0.5, -0.5], [ 0.0,  0.0]),
		([ 0.5,  0.5, -0.5], [ 0.0,  1.0]),
		([ 0.5, -0.5, -0.5], [ 0.0,  0.0]),
		([ 0.5,  0.5,  0.5], [ 1.0,  1.0]),
		([ 0.5, -0.5,  0.5], [ 1.0,  0.0]),
	],
	[
		([-0.5,  0.5,  0.5], [ 1.0,  1.0]),
		([-0.5,  0.5, -0.5], [ 0.0,  1.0]),
		([-0.5, -0.5, -0.5], [ 0.0,  0.0]),
		([-0.5, -0.5, -0.5], [ 0.0,  0.0]),
		([-0.5, -0.5,  0.5], [ 1.0,  0.0]),
		([-0.5,  0.5,  0.5], [ 1.0,  1.0]),
	],
	[
		([-0.5, -0.5,  0.5], [ 0.0,  0.0]),
		([ 0.5, -0.5,  0.5], [ 1.0,  0.0]),
		([ 0.5,  0.5,  0.5], [ 1.0,  1.0]),
		([ 0.5,  0.5,  0.5], [ 1.0,  1.0]),
		([-0.5,  0.5,  0.5], [ 0.0,  1.0]),
		([-0.5, -0.5,  0.5], [ 0.0,  0.0]),
	],
	[
		([-0.5, -0.5, -0.5], [ 0.0,  0.0]),
		([ 0.5,  0.5, -0.5], [ 1.0,  1.0]),
		([ 0.5, -0.5, -0.5], [ 1.0,  0.0]),
		([ 0.5,  0.5, -0.5], [ 1.0,  1.0]),
		([-0.5, -0.5, -0.5], [ 0.0,  0.0]),
		([-0.5,  0.5, -0.5], [ 0.0,  1.0]),
	],
];

#[rustfmt::skip]
const FACE_DIR: [[i16; 3]; 6] = [
	[ 0,  1,  0],
	[ 0, -1,  0],
	[ 1,  0,  0],
	[-1,  0,  0],
	[ 0,  0,  1],
	[ 0,  0, -1],
];
