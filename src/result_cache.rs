use std::fmt;
use std::{collections::BTreeMap, path::PathBuf, sync::{Arc, RwLock}, time::SystemTime};

struct CachedPage {
    when: SystemTime,
    html: String,
}

struct WrappedCache {
    cache: BTreeMap<PathBuf, CachedPage>,
    current_size: usize,
    max_size: usize,
}

#[derive(Clone)]
pub struct ResultCache {
    lock: Arc<RwLock<WrappedCache>>,
    signal_tx: tokio::sync::mpsc::Sender<()>,
}

impl ResultCache {
    pub fn new(max_size: usize) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let wrapped_cache = Arc::new(RwLock::new(WrappedCache {
            cache: BTreeMap::new(),
            current_size: 0,
            max_size,
        }));
        tokio::spawn(cache_compactor(rx, wrapped_cache.clone()));
        ResultCache {
            lock: wrapped_cache,
            signal_tx: tx,
        }
    }

    pub async fn add(&self, path: &std::path::Path, html: &str) {
        let needs_compact =
        {
            let Ok(mut lock) = self.lock.write() else {
                return;
            };
            let prev = lock.cache.insert(path.to_path_buf(), CachedPage {
                when: SystemTime::now(),
                html: html.to_string(),
            });
            if let Some(prev) = prev {
                lock.current_size -= prev.html.len();
            }
            lock.current_size += html.len();
            lock.current_size > lock.max_size
        };
        if needs_compact {
            if let Err(e) = self.signal_tx.send(()).await {
                tracing::warn!("Failed to send cache compact message: {e}");
            }
        }
    }

    pub fn get(&self, path: &std::path::Path) -> Option<String> {
        let Ok(lock) = self.lock.read() else {
            return None;
        };
        lock.cache.get(path).map(|res| res.html.clone())
    }

    pub fn clear(&self) {
        let Ok(mut lock) = self.lock.write() else {
            return;
        };
        lock.cache.clear();
        lock.current_size = 0;
    }
}

impl fmt::Debug for CachedPage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "({:?}-{:?})", self.when, &self.html[0..20])
    }
}

async fn cache_compactor(
    mut go_signal: tokio::sync::mpsc::Receiver<()>,
    cache: Arc<RwLock<WrappedCache>>,
) {
    while go_signal.recv().await.is_some() {
        tracing::debug!("Compacting HTML result cache");
        let Ok(mut lock) = cache.write() else {
            return;
        };
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
