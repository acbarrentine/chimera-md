use std::{cmp::Ordering, collections::BTreeMap, ffi::OsStr, path::{Path, PathBuf}, sync::Arc, time::Duration};
use async_watcher::{notify::{event::ModifyKind, EventKind, FsEventWatcher, RecursiveMode}, AsyncDebouncer, DebouncedEvent};
use tokio::sync::RwLock;

use crate::{chimera_error::ChimeraError, document_scraper::Doclink};

type NotifyError = async_watcher::notify::Error;

type DirectoryCache = Arc<RwLock<BTreeMap<PathBuf, Vec<Doclink>>>>;

pub struct FileManager {
    cache: DirectoryCache,
    broadcast_tx: tokio::sync::broadcast::Sender<FileEvent>,
    debouncer: AsyncDebouncer<FsEventWatcher>,
}

#[derive(Clone, Debug)]
pub enum EventType {
    Add,
    Change,
    Remove,
}

#[derive(Clone, Debug)]
pub struct FileEvent {
    pub path: PathBuf,
    pub kind: EventType,
}

impl FileManager {
    pub async fn new() -> Result<FileManager, ChimeraError> {
        let (broadcast_tx, _broadcast_rx) = tokio::sync::broadcast::channel(32);
        let (debouncer, file_events) =
            AsyncDebouncer::new_with_channel(Duration::from_secs(1), Some(Duration::from_secs(1))).await?;
        let cache = Arc::new(RwLock::new(BTreeMap::new()));
        tokio::spawn(directory_watcher(broadcast_tx.clone(), file_events, cache.clone()));
        Ok(FileManager{
            cache,
            broadcast_tx,
            debouncer,
        })
    }

    pub async fn find_peers(&self, relative_path: &str, index_file: &str) -> Option<Vec<Doclink>> {
        let relative_path = std::path::PathBuf::from(relative_path);
        let Ok(abs_path) = relative_path.canonicalize() else {
            return None;
        };
        let Some(relative_parent_path) = abs_path.parent() else {
            return None;
        };
        tracing::debug!("Find peers: {}", abs_path.display());
        {
            let cache = self.cache.read().await;
            if let Some(files) = cache.get(abs_path.as_path()) {
                return Some(files.clone())
            }
        }
        
        let Some(original_file_name) = relative_path.file_name() else {
            return None;
        };
        let mut files = Vec::new();
        if let Ok(mut read_dir) = tokio::fs::read_dir(relative_parent_path.as_os_str()).await {
            while let Ok(entry_opt) = read_dir.next_entry().await {
                if let Some(entry) = entry_opt {
                    let path = entry.path();
                    let file_name = entry.file_name();
                    if let Some(extension) = path.extension() {
                        if extension.eq_ignore_ascii_case(OsStr::new("md")) && file_name.ne(original_file_name) {
                            let name_string = file_name.to_string_lossy().to_string();
                            tracing::debug!("Peer: {}", name_string);
                            files.push(Doclink {
                                anchor: urlencoding::encode(name_string.as_str()).into_owned(),
                                name: name_string,
                            });
                        }
                    }
                }
                else {
                    break;
                }
            }
        }
        files.sort_unstable_by(|a, b| {
            if a.name.eq_ignore_ascii_case(index_file) {
                Ordering::Less
            }
            else if b.name.eq_ignore_ascii_case(index_file) {
                Ordering::Greater
            }
            else {
                a.name.cmp(&b.name)
            }
        });
        {
            let mut cache = self.cache.write().await;
            cache.insert(relative_parent_path.to_path_buf(), files.clone());
        }
        Some(files)
    }

    pub fn add_watch(&mut self, path: &Path) -> Result<(), ChimeraError> {
        self.debouncer.watcher().watch(path, RecursiveMode::Recursive)?;
        Ok(())
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<FileEvent> {
        self.broadcast_tx.subscribe()
    }
}

async fn remove_cached_directory(path: &Path, cache: &DirectoryCache) {
    if let Ok(abs_path) = path.canonicalize() {
        if let Some(parent_path) = abs_path.parent() {
            {
                let mut cache_lock = cache.write().await;
                cache_lock.remove(parent_path);
            }
        }
    }
}

async fn directory_watcher(
    broadcast_tx: tokio::sync::broadcast::Sender<FileEvent>,
    mut file_events: tokio::sync::mpsc::Receiver<Result<Vec<DebouncedEvent>, Vec<NotifyError>>>,
    cache: DirectoryCache,
) ->Result<(), ChimeraError> {
    while let Some(Ok(events)) = file_events.recv().await {
        for e in events {
            tracing::debug!("File change event {e:?}");
            match e.event.kind {
                EventKind::Create(f) => {
                    tracing::debug!("File change event: CREATE - {f:?}, {:?}", e.path);
                    remove_cached_directory(e.path.as_path(), &cache).await;
                    broadcast_tx.send(FileEvent{
                        path: e.path,
                        kind: EventType::Add,
                    })?;
                },
                EventKind::Modify(f) => {
                    tracing::debug!("File change event: MODIFY - {f:?}, {:?}", e.event.paths);
                    for p in e.event.paths {
                        match f {
                            ModifyKind::Name(_) |
                            ModifyKind::Other |
                            ModifyKind::Any => {
                                remove_cached_directory(p.as_path(), &cache).await;
                            },
                            _ => {}
                        };
                        broadcast_tx.send(FileEvent{
                            path: p,
                            kind: EventType::Change,
                        })?;
                    }
                },
                EventKind::Remove(f) => {
                    tracing::debug!("File change event: REMOVE - {f:?}, {:?}", e.path);
                    remove_cached_directory(e.path.as_path(), &cache).await;
                    broadcast_tx.send(FileEvent{
                        path: e.path,
                        kind: EventType::Remove,
                    })?;
                },
                _ => {}
            };
        }
    }
    Ok(())
}
