use super::{super::media::MediaMgr, AtlasSlice, CUBE};
use mt_net::{NodeDef, TileAnim, TileDef};
use rand::Rng;
use std::collections::HashMap;

pub(super) fn create_atlas(
    nodes: &mut HashMap<u16, NodeDef>,
    media: &MediaMgr,
) -> (image::RgbaImage, Vec<AtlasSlice>) {
    let mut rng = rand::thread_rng();
    let mut allocator = guillotiere::SimpleAtlasAllocator::new(guillotiere::size2(1, 1));
    let mut textures = Vec::new();

    for node in nodes.values_mut() {
        let tiles = std::iter::empty()
            .chain(node.tiles.iter_mut())
            .chain(node.overlay_tiles.iter_mut())
            .chain(node.special_tiles.iter_mut());

        let load_texture = |texture: &str| {
            let payload = media
                .get(texture)
                .ok_or_else(|| format!("texture not found: {texture}"))?;

            image::load_from_memory(payload)
                .or_else(|_| image::load_from_memory_with_format(payload, image::ImageFormat::Tga))
                .map_err(|e| format!("failed to load texture {texture}: {e}"))
                .map(|x| image::imageops::flip_vertical(&x))
        };

        let mut make_texture = |tile: &TileDef| {
            let string = &tile.texture.name;
            let mut tex = string
                .split('^')
                .map(|part| match load_texture(part) {
                    Ok(v) => v,
                    Err(e) => {
                        if !string.is_empty() && !string.contains('[') {
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
                .unwrap();
            match tile.animation {
                TileAnim::VerticalFrame {
                    n_frames: whatever, ..
                } => (|| {
                    if whatever.x == 0 || whatever.y == 0 {
                        eprintln!("invalid animation for tile {}", string);
                        return;
                    }
                    let tex_size = tex.dimensions();
                    let frame_height =
                        (tex_size.0 as f32 / whatever.x as f32 * whatever.y as f32) as u32;
                    tex =
                        image::imageops::crop(&mut tex, 0, 0, tex_size.0, frame_height).to_image();
                })(),
                _ => (),
            };
            tex
        };

        let mut id_map = HashMap::new();

        for tile in tiles {
            tile.texture.custom = *id_map.entry(tile.texture.name.clone()).or_insert_with(|| {
                let img = make_texture(&tile);

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
