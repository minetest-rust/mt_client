use super::{super::media::MediaMgr, AtlasSlice, CUBE};
use mt_net::NodeDef;
use std::collections::HashMap;

pub(super) fn create_atlas(
    nodes: &mut HashMap<u16, NodeDef>,
    media: &MediaMgr,
) -> (image::RgbaImage, Vec<AtlasSlice>) {
    let mut allocator = guillotiere::SimpleAtlasAllocator::new(guillotiere::size2(1, 1));
    let mut textures = Vec::new();

    let mut id_map = HashMap::new();

    for node in nodes.values_mut() {
        let tiles = std::iter::empty()
            .chain(node.tiles.iter_mut())
            .chain(node.overlay_tiles.iter_mut())
            .chain(node.special_tiles.iter_mut());

        for tile in tiles {
            tile.texture.custom = *id_map.entry(tile.texture.name.clone()).or_insert_with(|| {
                let img = media.texture_string(&tile.texture.name);

                let dimensions = img.dimensions();
                let size = guillotiere::size2(dimensions.0 as i32, dimensions.1 as i32);

                loop {
                    match allocator.allocate(size) {
                        None => {
                            let mut atlas_size = allocator.size();
                            atlas_size.width *= 2;
                            atlas_size.height *= 2;
                            allocator.grow(atlas_size);
                        }
                        Some(rect) => {
                            let id = textures.len();
                            textures.push((img, rect));
                            return id;
                        }
                    }
                }
            })
        }
    }

    let size = allocator.size();
    let mut atlas = image::RgbaImage::new(size.width as u32, size.height as u32);

    let slices = textures
        .into_iter()
        .map(|(img, rect)| {
            let w = size.width as f32;
            let h = size.height as f32;

            let x = (rect.min.x as f32 / w)..(rect.max.x as f32 / w);
            let y = (rect.min.y as f32 / h)..(rect.max.y as f32 / h);

            use image::GenericImage;
            atlas
                .copy_from(&img, rect.min.x as u32, rect.min.y as u32)
                .unwrap();

            use lerp::Lerp;
            use std::array::from_fn as array;

            let rect = [x, y];
            let cube_tex_coords =
                array(|f| array(|v| array(|i| rect[i].start.lerp(rect[i].end, CUBE[f][v].1[i]))));

            AtlasSlice { cube_tex_coords }
        })
        .collect();

    (atlas, slices)
}
