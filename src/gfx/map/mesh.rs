use super::{LeavesMode, MapRenderSettings, MeshgenInfo, Vertex, CUBE, FACE_DIR};
use cgmath::{Deg, Matrix3, Point3, Vector3};
use mt_net::MapBlock;

#[derive(Clone)]
pub(super) struct MeshData {
    pub vertices: Vec<Vertex>,
    pub vertices_blend: Vec<Vertex>,
}

impl MeshData {
    pub fn new(cap: usize) -> Self {
        Self {
            vertices: Vec::with_capacity(cap),
            vertices_blend: Vec::with_capacity(cap),
        }
    }

    pub fn cap(&self) -> usize {
        std::cmp::max(self.vertices.capacity(), self.vertices_blend.capacity())
    }
}

pub(super) fn create_mesh(
    mkinfo: &MeshgenInfo,
    settings: &MapRenderSettings,
    _pos: Point3<i16>,
    block: &MapBlock,
    nbors: [Option<&MapBlock>; 6],
    buffer: &mut MeshData,
) {
    for (index, &content) in block.param_0.iter().enumerate() {
        let def = match &mkinfo.nodes[content as usize] {
            Some(x) => x,
            None => continue,
        };

        use mt_net::{DrawType, Param1Type};
        use std::array::from_fn as array;

        let mut tiles = &def.tiles;
        let mut draw_type = def.draw_type;

        match draw_type {
            DrawType::AllFacesOpt => {
                draw_type = match settings.leaves {
                    LeavesMode::Opaque => DrawType::Cube,
                    LeavesMode::Simple => {
                        tiles = &def.special_tiles;

                        DrawType::GlassLike
                    }
                    LeavesMode::Fancy => DrawType::AllFaces,
                };
            }
            DrawType::None => continue,
            _ => {}
        }

        let light = match def.param1_type {
            Param1Type::Light => block.param_1[index] as f32 / 15.0, // FIXME
            _ => 1.0,
        };

        let vertices = if def.alpha == mt_net::Alpha::Blend {
            &mut buffer.vertices_blend
        } else {
            &mut buffer.vertices
        };

        let pos: [i16; 3] = array(|i| ((index >> (4 * i)) & 0xf) as i16);

        if draw_type == DrawType::Plant {
            let f = 2;
            let face = &CUBE[f];

            let tile = &tiles[f];
            let texture = mkinfo.textures[tile.texture.custom].cube_tex_coords[f];

            let mut add_vertex = |mat: Matrix3<f32>, vertex: (usize, &([f32; 3], [f32; 2]))| {
                let point = Point3::new(pos[0] as f32, pos[1] as f32, pos[2] as f32)
                    + mat
                        * (Vector3::new(vertex.1 .0[0], vertex.1 .0[1], vertex.1 .0[2])
                            - Vector3::new(0.5, 0.0, 0.0));

                vertices.push(Vertex {
                    pos: [point.x, point.y, point.z],
                    tex_coords: texture[vertex.0],
                    light,
                });
            };

            let mut add_vertices = |mat| {
                face.iter().enumerate().for_each(|x| add_vertex(mat, x));
                if !tile.flags.contains(mt_net::TileFlag::BackfaceCull) {
                    face.iter()
                        .enumerate()
                        .rev()
                        .for_each(|x| add_vertex(mat, x));
                }
            };

            add_vertices(Matrix3::from_angle_y(Deg(45.0)));
            add_vertices(Matrix3::from_angle_y(Deg(135.0)));
        } else {
            for (f, face) in CUBE.iter().enumerate() {
                if draw_type == DrawType::Cube || draw_type == DrawType::Liquid {
                    let c = [1, 1, 0, 0, 2, 2][f];

                    let mut nblk = block;
                    let mut npos = pos;
                    npos[c] += FACE_DIR[f][c];

                    if !(0..16).contains(&npos[c]) {
                        nblk = match nbors[f].as_ref() {
                            Some(x) => x,
                            None => continue,
                        };

                        npos[c] = (npos[c] + 16) % 16;
                    }

                    let nidx = npos[0] | (npos[1] << 4) | (npos[2] << 8);
                    let ncontent = nblk.param_0[nidx as usize];

                    if let Some(ndef) = &mkinfo.nodes[ncontent as usize] {
                        if match draw_type {
                            DrawType::Cube => ndef.draw_type == DrawType::Cube,
                            DrawType::Liquid => {
                                ndef.draw_type == DrawType::Cube || ncontent == content
                            }
                            _ => false,
                        } {
                            continue;
                        }
                    }
                }

                let tile = &tiles[f];
                let texture = mkinfo.textures[tile.texture.custom].cube_tex_coords[f];

                let mut add_vertex = |vertex: (usize, &([f32; 3], [f32; 2]))| {
                    vertices.push(Vertex {
                        pos: array(|i| pos[i] as f32 + vertex.1 .0[i]),
                        tex_coords: texture[vertex.0],
                        light,
                    });
                };

                face.iter().enumerate().for_each(&mut add_vertex);
                if !tile.flags.contains(mt_net::TileFlag::BackfaceCull) {
                    face.iter().enumerate().rev().for_each(&mut add_vertex);
                }
            }
        }
    }
}
