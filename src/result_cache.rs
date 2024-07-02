use std::fmt;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::SystemTime};
use tokio::sync::RwLock;

struct CachedPage {
    when: SystemTime,
    html: String,
}

struct WrappedCache {
    cache: BTreeMap<PathBuf, CachedPage>,
    compact_tx: tokio::sync::mpsc::Sender<()>,
    current_size: usize,
    max_size: usize,
}

#[derive(Clone)]
pub struct ResultCache {
    lock: Arc<RwLock<WrappedCache>>,
}

impl ResultCache {
    pub fn new(max_size: usize) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let wrapped_cache = Arc::new(RwLock::new(WrappedCache {
            cache: BTreeMap::new(),
            compact_tx: tx,
            current_size: 0,
            max_size,
        }));
        tokio::spawn(cache_watcher(rx, wrapped_cache.clone()));
        ResultCache {
            lock: wrapped_cache
        }
    }

    pub async fn add(&self, path: &std::path::Path, html: &str) {
        let mut lock = self.lock.write().await;
        let prev = lock.cache.insert(path.to_path_buf(), CachedPage {
            when: SystemTime::now(),
            html: html.to_string(),
        });
        if let Some(prev) = prev {
            lock.current_size -= prev.html.len();
        }
        lock.current_size += html.len();
        tracing::info!("Current cache size: {} kb", lock.current_size as f64 / 1024.0);
        if lock.current_size > lock.max_size {
            if let Err(e) = lock.compact_tx.send(()).await {
                tracing::warn!("Failed to send cache compact message: {e}");
            }
        }
    }

    pub async fn get(&self, path: &std::path::Path) -> Option<String> {
        let lock = self.lock.read().await;
        lock.cache.get(path).map(|res| res.html.clone())
    }

    pub async fn clear(&self) {
        let mut lock = self.lock.write().await;
        lock.cache.clear();
        lock.current_size = 0;
    }
}

impl fmt::Debug for CachedPage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "({:?}-{:?})", self.when, &self.html[0..20])
    }
}

async fn cache_watcher(
    mut cache_compactor: tokio::sync::mpsc::Receiver<()>,
    cache: Arc<RwLock<WrappedCache>>,
) {
    while cache_compactor.recv().await.is_some() {
        tracing::debug!("Compacting HTML result cache");
        let mut lock = cache.write().await;
        let mut v: Vec<(PathBuf, SystemTime, usize)> = lock.cache.iter().map(|(path, page)| {
            (path.clone(), page.when, path.as_os_str().len() + page.html.len())
        }).collect();
        v.sort_unstable_by(|a, b| {
            b.1.cmp(&a.1)
        });
        while lock.current_size > lock.max_size {
            match v.pop() {
                Some((path, _, page_size)) => {
                    tracing::debug!("Retiring {} from the HTML cache, size {} kb", path.display(), page_size as f64 / 1024.0);
                    lock.cache.remove(path.as_path());
                    lock.current_size = lock.current_size.saturating_sub(page_size);
                },
                None => {
                    break;
                }
            }
        }
        tracing::debug!("New cache size: {} kb", lock.current_size as f64 / 1024.0);
    }
}
