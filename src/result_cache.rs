use std::ffi::OsStr;
use std::fmt;
use std::{path::PathBuf, sync::{Arc, RwLock}, time::SystemTime};
use indexmap::IndexMap;

#[cfg(test)]
use crate::chimera_error::ChimeraError;
use crate::file_manager::FileManager;

struct CachedPage {
    when: SystemTime,
    modtime: SystemTime,
    html: String,
}

struct WrappedCache {
    cache: IndexMap<PathBuf, CachedPage>,
    current_size: usize,
    max_size: usize,
}

enum CacheAction {
    Compact,
    Clean
}

#[derive(Clone)]
pub struct ResultCache {
    lock: Arc<RwLock<WrappedCache>>,
    signal_tx: tokio::sync::mpsc::Sender<CacheAction>,
}

async fn get_modtime(path: &std::path::Path) -> SystemTime {
    if let Ok(metadata) = tokio::fs::metadata(path).await {
        if let Ok(modtime) = metadata.modified() {
            return modtime;
        }
    }
    SystemTime::UNIX_EPOCH
}

impl ResultCache {
    pub fn new(max_size: usize) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        let wrapped_cache = Arc::new(RwLock::new(WrappedCache {
            cache: IndexMap::new(),
            current_size: 0,
            max_size,
        }));
        tokio::spawn(cache_compactor(rx, wrapped_cache.clone()));
        ResultCache {
            lock: wrapped_cache,
            signal_tx: tx,
        }
    }

    pub fn listen_for_changes(&self, file_manager: &FileManager) {
        let rx = file_manager.subscribe();
        tokio::spawn(listen_for_changes(rx, self.clone()));
    }

    pub async fn add(&self, path: &std::path::Path, html: &str) {
        let needs_compact =
        {
            let modtime = get_modtime(path).await;
            let Ok(mut lock) = self.lock.write() else {
                tracing::warn!("Result cache lock poisoned error");
                return;
            };
            let page = CachedPage {
                when: SystemTime::now(),
                modtime,
                html: html.to_string(),
            };
            let size = page.html.len();
            let prev = lock.cache.insert(path.to_path_buf(), page);
            if let Some(prev) = prev {
                lock.current_size -= prev.html.len();
            }
            lock.current_size += size;
            lock.current_size > lock.max_size
        };
        if needs_compact {
            if let Err(e) = self.signal_tx.send(CacheAction::Compact).await {
                tracing::warn!("Failed to send cache compact message: {e}");
            }
        }
    }

    pub async fn get(&self, path: &std::path::Path) -> Option<String> {
        let modtime = get_modtime(path).await;
        let mut needs_clean = false;
        {
            let Ok(lock) = self.lock.read() else {
                return None;
            };
            if let Some(res) = lock.cache.get(path) {
                if res.modtime == modtime {
                    return Some(res.html.clone())
                }
                else{
                    needs_clean = true;
                }
            }
        }
        if needs_clean {
            if let Err(e) = self.signal_tx.send(CacheAction::Clean).await {
                tracing::warn!("Failed to send cache clean message: {e}");
            }
        }
        None
    }

    #[cfg(test)]
    pub fn get_size(&self) -> Result<usize, ChimeraError> {
        let lock = self.lock.read()?;
        Ok(lock.current_size)
    }

    pub fn clear(&self) {
        let Ok(mut lock) = self.lock.write() else {
            return;
        };
        lock.cache.clear();
    }
}

impl fmt::Debug for CachedPage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "({:?}-{:?})", self.when, &self.html[0..20])
    }
}

async fn cache_compactor(
    mut go_signal: tokio::sync::mpsc::Receiver<CacheAction>,
    cache: Arc<RwLock<WrappedCache>>,
) {
    while let Some(signal) = go_signal.recv().await {
        match signal {
            CacheAction::Compact => {
                tracing::debug!("Compacting HTML result cache");
                let Ok(mut lock) = cache.write() else {
                    return;
                };
                let target_trim_size  = lock.current_size - lock.max_size;
                let mut prune_size = 0;
                let mut split_index = 0;
                for (i, v) in lock.cache.values().enumerate() {
                    prune_size += v.html.len();
                    if prune_size > target_trim_size {
                        split_index = i;
                        break;
                    }
                }
                lock.cache = lock.cache.split_off(split_index);
                lock.current_size -= prune_size;
                tracing::debug!("New cache size: {} kb", lock.current_size as f64 / 1024.0);
            },
            CacheAction::Clean => {
                tracing::debug!("Compacting HTML result cache");
                let Ok(mut lock) = cache.write() else {
                    return;
                };
                lock.cache.clear();
            },
        }
    }
}

async fn listen_for_changes(
    mut rx: tokio::sync::broadcast::Receiver<PathBuf>,
    cache: ResultCache,
) {
    while let Ok(path) = rx.recv().await {
        tracing::debug!("RC change event {}", path.display());
        if let Some(ext) = path.extension() {
            if ext == OsStr::new("md") || ext == OsStr::new("html") {
                cache.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn test_compact() {
        let cache = ResultCache::new(450);
        cache.add(PathBuf::from("a").as_path(), "a".repeat(100).as_str()).await;
        assert_eq!(cache.get_size(), Ok(100));
        cache.add(PathBuf::from("a").as_path(), "a".repeat(100).as_str()).await;
        assert_eq!(cache.get_size(), Ok(100));
        cache.add(PathBuf::from("b").as_path(), "b".repeat(100).as_str()).await;
        assert_eq!(cache.get_size(), Ok(200));
        cache.add(PathBuf::from("c").as_path(), "c".repeat(100).as_str()).await;
        assert_eq!(cache.get_size(), Ok(300));
        cache.add(PathBuf::from("d").as_path(), "d".repeat(100).as_str()).await;
        assert_eq!(cache.get_size(), Ok(400));
        cache.add(PathBuf::from("e").as_path(), "e".repeat(100).as_str()).await;
        assert_eq!(cache.get_size(), Ok(500));
        // wait a bit for the compaction to occur
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        assert_eq!(cache.get_size(), Ok(400));
    }
}
