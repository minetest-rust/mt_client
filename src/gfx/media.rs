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
}
