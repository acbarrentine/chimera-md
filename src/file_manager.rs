use std::{cmp::Ordering, collections::BTreeMap, ffi::{OsStr, OsString}, path::{Path, PathBuf}, time::Duration};
use async_watcher::{notify::{EventKind, FsEventWatcher, RecursiveMode}, AsyncDebouncer, DebouncedEvent};
use tokio::sync::RwLock;

use crate::{chimera_error::ChimeraError, document_scraper::Doclink};

type NotifyError = async_watcher::notify::Error;

pub struct FileManager {
    cache: RwLock<BTreeMap<OsString, Vec<Doclink>>>,
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

        tokio::spawn(directory_watcher(broadcast_tx.clone(), file_events));
        // todo: listen for changes myself
    
        Ok(FileManager{
            cache: RwLock::new(BTreeMap::new()),
            broadcast_tx,
            debouncer,
        })
    }

    pub async fn find_peers(&self, relative_path: &str, index_file: &str) -> Vec<Doclink> {
        let relative_path = std::path::PathBuf::from(relative_path);
        let mut relative_parent_path = match relative_path.parent() {
            Some(relative_parent_path) => relative_parent_path.to_path_buf(),
            None => return Vec::new()
        };
        let osstr = relative_parent_path.as_mut_os_string();
        if osstr.is_empty() {
            osstr.push(".");
        }
        tracing::debug!("Relative path: {}", osstr.to_string_lossy());

        {
            let cache = self.cache.read().await;
            if let Some(files) = cache.get(osstr.as_os_str()) {
                return files.clone()
            }
        }
        
        let Some(original_file_name) = relative_path.file_name() else {
            tracing::debug!("No filename found for {}", relative_path.display());
            return Vec::new()
        };

        let mut files = Vec::new();
        if let Ok(mut read_dir) = tokio::fs::read_dir(osstr.as_os_str()).await {
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
            cache.insert(osstr.clone(), files.clone());
        }
        files
    }

    pub fn add_watch(&mut self, path: &Path) -> Result<(), ChimeraError> {
        self.debouncer.watcher().watch(path, RecursiveMode::Recursive)?;
        Ok(())
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<FileEvent> {
        self.broadcast_tx.subscribe()
    }
}

async fn directory_watcher(
    broadcast_tx: tokio::sync::broadcast::Sender<FileEvent>,
    mut file_events: tokio::sync::mpsc::Receiver<Result<Vec<DebouncedEvent>, Vec<NotifyError>>>
) ->Result<(), ChimeraError> {
    while let Some(Ok(events)) = file_events.recv().await {
        for e in events {
            tracing::debug!("File change event {e:?}");
            match e.event.kind {
                EventKind::Create(f) => {
                    tracing::debug!("File change event: CREATE - {f:?}, {:?}", e.path);
                    broadcast_tx.send(FileEvent{
                        path: e.path,
                        kind: EventType::Add,
                    })?;
                },
                EventKind::Modify(f) => {
                    tracing::debug!("File change event: MODIFY - {f:?}, {:?}", e.event.paths);
                    for p in e.event.paths {
                        broadcast_tx.send(FileEvent{
                            path: p,
                            kind: EventType::Change,
                        })?;
                    }
                },
                EventKind::Remove(f) => {
                    tracing::debug!("File change event: REMOVE - {f:?}, {:?}", e.path);
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
