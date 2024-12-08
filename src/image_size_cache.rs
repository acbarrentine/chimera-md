use std::{collections::HashMap, fs, path::PathBuf};
use serde::Deserialize;

#[derive (Deserialize, Debug)]
pub struct WidthAndHeight {
    pub width: u32,
    pub height: u32,
}

#[derive (Deserialize, Debug, Default)]
pub struct ImageSizeCache {
    map: HashMap<String, WidthAndHeight>,
}

impl ImageSizeCache {
    pub fn new(path_to_cache: PathBuf) -> Self {
        match fs::read_to_string(path_to_cache) {
            Ok(cached_file_data) => {
                match toml::from_str(&cached_file_data) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::error!("Error parsing image-sizes.toml: {e}");
                        ImageSizeCache::default()
                    },
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read image-sizes.toml: {e}");
                ImageSizeCache::default()
            },
        }
    }

    pub fn get_dimensions(&self, img: &str) -> Option<&WidthAndHeight> {
        self.map.get(img)
    }
}
