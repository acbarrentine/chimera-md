use std::{fs, path::PathBuf};
use indexmap::IndexMap;
use serde::Deserialize;

#[derive (Deserialize, Debug)]
pub struct WidthAndHeight {
    pub width: u32,
    pub height: u32,
}

#[derive (Deserialize, Debug, Default)]
pub struct ImageSizeCache {
    map: IndexMap<String, WidthAndHeight>,
}

impl ImageSizeCache {
    pub fn new(path_to_cache: PathBuf) -> Self {
        let cache = match fs::read_to_string(path_to_cache.as_path()) {
            Ok(cached_file_data) => {
                match toml::from_str(&cached_file_data) {
                    Ok(m) => {
                        m
                    },
                    Err(e) => {
                        tracing::error!("Error parsing image-sizes.toml: {e}");
                        ImageSizeCache::default()
                    },
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read {}: {e}", path_to_cache.display());
                ImageSizeCache::default()
            },
        };
        tracing::debug!("Found {} images in the cache", cache.map.len());
        cache
    }

    pub fn get_dimensions(&self, img: &str) -> Option<&WidthAndHeight> {
        self.map.get(img)
    }
}
