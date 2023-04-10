use super::{media::MediaMgr, state::State, util::MatrixUniform};
use cgmath::{prelude::*, Matrix4, Point3, Vector3};
use mt_net::{MapBlock, NodeDef};
use rand::Rng;
use std::{collections::HashMap, ops::Range};
use wgpu::util::DeviceExt;

pub struct MapRender {
    pipeline: wgpu::RenderPipeline,
    textures: HashMap<String, [Range<f32>; 2]>,
    nodes: HashMap<u16, NodeDef>,
    atlas: wgpu::BindGroup,
    model: wgpu::BindGroupLayout,
    blocks: HashMap<[i16; 3], BlockMesh>,
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
    model: MatrixUniform,
}

impl MapRender {
    pub fn render<'a>(&'a self, state: &'a State, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.atlas, &[]);
        pass.set_bind_group(1, &state.camera_uniform.bind_group, &[]);

        for mesh in self.blocks.values() {
            pass.set_bind_group(2, &mesh.model.bind_group, &[]);
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.draw(0..mesh.num_vertices, 0..1);
        }
    }

    pub fn add_block(&mut self, state: &mut State, pos: Point3<i16>, block: Box<MapBlock>) {
        let mut vertices = Vec::with_capacity(10000);
        for (index, content) in block.param_0.iter().enumerate() {
            let def = match self.nodes.get(content) {
                Some(x) => x,
                None => continue,
            };

            use lerp::Lerp;
            use mt_net::{DrawType, Param1Type};
            use std::array::from_fn as array;

            match def.draw_type {
                DrawType::Cube | DrawType::AllFaces | DrawType::AllFacesOpt => {
                    let light = match def.param1_type {
                        Param1Type::Light => {
                            println!("{}", block.param_1[index]);

                            block.param_1[index] as f32 / 15.0
                        }
                        _ => 1.0,
                    };

                    let pos: [i16; 3] = array(|i| ((index >> (4 * i)) & 0xf) as i16);
                    for (f, face) in CUBE.iter().enumerate() {
                        let dir = FACE_DIR[f];
                        let npos: [i16; 3] = array(|i| dir[i] + pos[i]);
                        if npos.iter().all(|x| (0..16).contains(x)) {
                            let nindex = npos[0] | (npos[1] << 4) | (npos[2] << 8);

                            if let Some(ndef) = self.nodes.get(&block.param_0[nindex as usize]) {
                                if ndef.draw_type == DrawType::Cube {
                                    continue;
                                }
                            }
                        }

                        let tile = &def.tiles[f];
                        let rect = self.textures.get(&tile.texture).unwrap();

                        for vertex in face.iter() {
                            /*println!(
                                "{:?} {:?} {:?} {:?}",
                                (vertex.1[0], vertex.1[1]),
                                (rect[0].start, rect[1].start),
                                (rect[0].end, rect[1].end),
                                (
                                    vertex.1[0].lerp(rect[0].start, rect[0].end),
                                    vertex.1[1].lerp(rect[1].start, rect[1].end)
                                )
                            );*/
                            vertices.push(Vertex {
                                pos: array(|i| pos[i] as f32 - 8.5 + vertex.0[i]),
                                tex_coords: array(|i| rect[i].start.lerp(rect[i].end, vertex.1[i])),
                                light,
                            })
                        }
                    }
                }
                DrawType::None => {}
                _ => {
                    // TODO
                }
            }
        }

        self.blocks.insert(
            pos.into(),
            BlockMesh {
                vertex_buffer: state
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("mapblock.vertex_buffer"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                num_vertices: vertices.len() as u32,
                model: MatrixUniform::new(
                    &state.device,
                    &self.model,
                    Matrix4::from_translation(
                        pos.cast::<f32>().unwrap().to_vec() * 16.0 + Vector3::new(8.5, 8.5, 8.5),
                    ),
                    "mapblock",
                    false,
                ),
            },
        );
    }

    pub fn new(state: &mut State, media: &MediaMgr, nodes: HashMap<u16, NodeDef>) -> Self {
        let mut rng = rand::thread_rng();
        let mut atlas_map = HashMap::new();
        let mut atlas_alloc = guillotiere::SimpleAtlasAllocator::new(guillotiere::size2(1, 1));

        for node in nodes.values() {
            let tiles = node
                .tiles
                .iter()
                .chain(node.overlay_tiles.iter())
                .chain(node.special_tiles.iter());

            let load_texture = |texture: &str| {
                let payload = media
                    .get(texture)
                    .ok_or_else(|| format!("texture not found: {texture}"))?;

                image::load_from_memory(payload)
                    .or_else(|_| {
                        image::load_from_memory_with_format(payload, image::ImageFormat::Tga)
                    })
                    .map_err(|e| format!("failed to load texture {texture}: {e}"))
                    .map(|x| image::imageops::flip_vertical(&x))
            };

            let mut make_texture = |texture: &str| {
                texture
                    .split('^')
                    .map(|part| match load_texture(part) {
                        Ok(v) => v,
                        Err(e) => {
                            if !texture.is_empty() && !texture.contains('[') {
                                eprintln!("{e}");
                            }

                            let mut img = image::RgbImage::new(1, 1);
                            rng.fill(&mut img.get_pixel_mut(0, 0).0);

                            image::DynamicImage::from(img).to_rgba8()
                        }
                    })
                    .reduce(|mut base, top| {
                        image::imageops::overlay(&mut base, &top, 0, 0);
                        base
                    })
                    .unwrap()
            };

            for tile in tiles {
                atlas_map.entry(tile.texture.clone()).or_insert_with(|| {
                    let img = make_texture(&tile.texture);

                    let dimensions = img.dimensions();
                    let size = guillotiere::size2(dimensions.0 as i32, dimensions.1 as i32);

                    loop {
                        match atlas_alloc.allocate(size) {
                            None => {
                                let mut atlas_size = atlas_alloc.size();
                                atlas_size.width *= 2;
                                atlas_size.height *= 2;
                                atlas_alloc.grow(atlas_size);
                            }
                            Some(v) => return (img, v),
                        }
                    }
                });
            }
        }

        let atlas_size = atlas_alloc.size();
        let mut atlas = image::RgbaImage::new(atlas_size.width as u32, atlas_size.height as u32);

        let textures = atlas_map
            .into_iter()
            .map(|(name, (img, rect))| {
                let w = atlas_size.width as f32;
                let h = atlas_size.height as f32;

                let x = (rect.min.x as f32 / w)..(rect.max.x as f32 / w);
                let y = (rect.min.y as f32 / h)..(rect.max.y as f32 / h);

                use image::GenericImage;
                atlas
                    .copy_from(&img, rect.min.x as u32, rect.min.y as u32)
                    .unwrap();

                (name, [x, y])
            })
            .collect();

        let size = wgpu::Extent3d {
            width: atlas_size.width as u32,
            height: atlas_size.height as u32,
            depth_or_array_layers: 1,
        };

        let atlas_texture = state.device.create_texture(&wgpu::TextureDescriptor {
            size,
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
            &atlas,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: std::num::NonZeroU32::new(4 * atlas_size.width as u32),
                rows_per_image: std::num::NonZeroU32::new(atlas_size.height as u32),
            },
            size,
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
                        blend: Some(wgpu::BlendState::REPLACE),
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

        Self {
            pipeline,
            nodes,
            textures,
            atlas: atlas_bind_group,
            model: model_bind_group_layout,
            blocks: HashMap::new(),
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
