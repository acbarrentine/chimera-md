use std::{ffi::OsStr, fs, path::PathBuf, sync::{Arc, RwLock}};
use indexmap::IndexMap;
use serde::Deserialize;

use crate::file_manager::FileManager;

#[derive (Deserialize, Debug, Clone)]
pub struct WidthAndHeight {
    pub width: u32,
    pub height: u32,
}

struct ImageSizeCacheInternal {
    path: PathBuf,
    map: IndexMap<String, WidthAndHeight>,
}

#[derive (Clone)]
pub struct ImageSizeCache {
    lock: Arc<RwLock<ImageSizeCacheInternal>>,
}

impl ImageSizeCacheInternal {
    fn new(path: PathBuf) -> Self {
        ImageSizeCacheInternal {
            path,
            map: IndexMap::new(),
        }
    }

    fn load(&mut self) {
        self.map = match fs::read_to_string(self.path.as_path()) {
            Ok(cached_file_data) => {
                match toml::from_str(&cached_file_data) {
                    Ok(m) => {
                        m
                    },
                    Err(e) => {
                        tracing::error!("Error parsing image-sizes.toml: {e}");
                        IndexMap::new()
                    },
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read {}: {e}", self.path.display());
                IndexMap::new()
            },
        };
        tracing::info!("Image cache loaded with {} images", self.map.len());
    }
}

impl ImageSizeCache {
    pub fn new(path_to_cache: PathBuf) -> Self {
        let mut cache = ImageSizeCacheInternal::new(path_to_cache);
        cache.load();
        tracing::debug!("Found {} images in the cache", cache.map.len());
        ImageSizeCache {
            lock: Arc::new(RwLock::new(cache))
        }
    }
    
    fn load(&mut self) {
        let Ok(mut lock) = self.lock.write() else {
            return;
        };
        lock.load();
    }

    pub fn listen_for_changes(&self, file_manager: &FileManager) {
        let rx: tokio::sync::broadcast::Receiver<PathBuf> = file_manager.subscribe();
        tokio::spawn(listen_for_changes(rx, self.clone()));
    }

    pub fn get_dimensions(&self, img: &str) -> Option<WidthAndHeight> {
        let Ok(lock) = self.lock.read() else {
            return None;
        };
        lock.map.get(img).cloned()
    }
}

async fn listen_for_changes(
    mut rx: tokio::sync::broadcast::Receiver<PathBuf>,
    mut cache: ImageSizeCache,
) {
    while let Ok(path) = rx.recv().await {
        if let Some(ext) = path.extension() {
            tracing::info!("Image size cache change event {}", path.display());
            if ext == OsStr::new("toml") {
                cache.load();
            }
        }
    }
}
