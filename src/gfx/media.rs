use rand::Rng;
use std::collections::HashMap;

#[derive(rust_embed::RustEmbed)]
#[folder = "assets/textures"]
pub struct BaseFolder; // copied from github.com/minetest/minetest

pub struct MediaMgr {
    packs: Vec<HashMap<String, Vec<u8>>>,
    srv_idx: usize,
}

impl MediaMgr {
    pub fn new() -> Self {
        Self {
            packs: [
                BaseFolder::iter()
                    .map(|file| {
                        (
                            file.to_string(),
                            BaseFolder::get(&file).unwrap().data.into_owned(),
                        )
                    })
                    .collect(),
                HashMap::new(),
            ]
            .into(),
            srv_idx: 1,
        }
    }

    pub fn add_server_media(&mut self, files: HashMap<String, Vec<u8>>) {
        self.packs[self.srv_idx].extend(files.into_iter());
    }

    pub fn get(&self, file: &str) -> Option<&[u8]> {
        self.packs
            .iter()
            .rev()
            .find_map(|pack| pack.get(file))
            .map(Vec::as_slice)
    }

    pub fn rand_img() -> image::RgbaImage {
        let mut img = image::RgbImage::new(1, 1);
        rand::thread_rng().fill(&mut img.get_pixel_mut(0, 0).0);

        image::DynamicImage::from(img).to_rgba8()
    }

    pub fn texture(&self, texture: &str) -> image::RgbaImage {
        match match self.get(texture) {
            Some(payload) => image::load_from_memory(payload)
                .or_else(|_| image::load_from_memory_with_format(payload, image::ImageFormat::Tga))
                .map_err(|e| eprintln!("while loading {texture}: {e}"))
                .ok(),
            None => {
                eprintln!("unknown texture: {texture}");
                None
            }
        } {
            Some(v) => image::imageops::flip_vertical(&v),
            None => Self::rand_img(),
        }
    }

    pub fn texture_string(&self, texture: &str) -> image::RgbaImage {
        texture
            .split('^')
            .fold(None, |mut base, next| {
                if let Some(overlay) = match next {
                    "" => Some(self.texture("no_texture.png")),
                    texmod if matches!(texmod.chars().next(), Some('[')) => {
                        eprintln!("unknown texture modifier: {texmod}");
                        None
                    }
                    texture => Some(self.texture(texture)),
                } {
                    if let Some(base) = &mut base {
                        image::imageops::overlay(base, &overlay, 0, 0);
                    } else {
                        base = Some(overlay);
                    }
                }

                base
            })
            .unwrap_or_else(Self::rand_img)
    }
}
