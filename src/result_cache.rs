use std::ffi::OsStr;
use std::fmt;
use std::path::Path;
use std::{collections::BTreeMap, path::PathBuf, sync::{Arc, RwLock}, time::SystemTime};

use crate::document_scraper::DocumentScraper;
use crate::file_manager::FileManager;

struct CachedPage {
    when: SystemTime,
    html: String,
    scraper: DocumentScraper,
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
    root: PathBuf,
}

impl ResultCache {
    pub fn new(file_manager: &FileManager, max_size: usize, document_root: &Path) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let wrapped_cache = Arc::new(RwLock::new(WrappedCache {
            cache: BTreeMap::new(),
            current_size: 0,
            max_size,
        }));
        tokio::spawn(cache_compactor(rx, wrapped_cache.clone()));
        let cache = ResultCache {
            lock: wrapped_cache,
            signal_tx: tx,
            root: document_root.to_path_buf(),
        };

        let rx = file_manager.subscribe();
        tokio::spawn(listen_for_changes(rx, cache.clone()));

        cache
    }

    pub async fn add(&self, path: &std::path::Path, html: &str, scraper: DocumentScraper) {
        let needs_compact =
        {
            let Ok(mut lock) = self.lock.write() else {
                tracing::warn!("Result cache lock poisoned error");
                return;
            };
            let page = CachedPage {
                when: SystemTime::now(),
                html: html.to_string(),
                scraper,
            };
            let size = page.get_size();
            let prev = lock.cache.insert(path.to_path_buf(), page);
            if let Some(prev) = prev {
                lock.current_size -= prev.get_size();
            }
            lock.current_size += size;
            lock.current_size > lock.max_size
        };
        if needs_compact {
            if let Err(e) = self.signal_tx.send(()).await {
                tracing::warn!("Failed to send cache compact message: {e}");
            }
        }
    }

    pub fn get(&self, path: &std::path::Path) -> Option<(String, DocumentScraper)> {
        let Ok(lock) = self.lock.read() else {
            return None;
        };
        lock.cache.get(path).map(|res| (res.html.clone(), res.scraper.clone()))
    }

    pub fn remove(&self, path: &std::path::Path) {
        let Ok(mut lock) = self.lock.write() else {
            return;
        };
        if let Some(prev) = lock.cache.remove(path) {
            lock.current_size -= prev.get_size();
        }
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

async fn listen_for_changes(
    mut rx: tokio::sync::broadcast::Receiver<PathBuf>,
    cache: ResultCache,
) {
    while let Ok(path) = rx.recv().await {
        tracing::debug!("RC change event {}", path.display());
        if let Some(ext) = path.extension() {
            if ext == OsStr::new("md") {
                if let Ok(relative_path) = path.strip_prefix(cache.root.as_path()) {
                    tracing::info!("Discarding cached HTML result for {}", relative_path.display());
                    cache.remove(relative_path);
                }
            }
        }
    }
}

impl CachedPage {
    fn get_size(&self) -> usize {
        self.html.len() + self.scraper.get_size()
    }
}