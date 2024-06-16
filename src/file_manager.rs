use std::{cmp::Ordering, ffi::OsStr, path::{Path, PathBuf}, time::Duration};
use async_watcher::{notify::{EventKind, FsEventWatcher, RecursiveMode}, AsyncDebouncer, DebouncedEvent};

use crate::{chimera_error::ChimeraError, document_scraper::Doclink};

type NotifyError = async_watcher::notify::Error;

pub struct FileManager {
    broadcast_tx: tokio::sync::broadcast::Sender<PathBuf>,
    debouncer: AsyncDebouncer<FsEventWatcher>,
}

impl FileManager {
    pub async fn new() -> Result<FileManager, ChimeraError> {
        let (broadcast_tx, _broadcast_rx) = tokio::sync::broadcast::channel(32);
        let (debouncer, file_events) =
            AsyncDebouncer::new_with_channel(Duration::from_secs(1), Some(Duration::from_secs(1))).await?;
        tokio::spawn(directory_watcher(broadcast_tx.clone(), file_events));
        Ok(FileManager{
            broadcast_tx,
            debouncer,
        })
    }

    pub async fn find_peers(&self, relative_path: &str, index_file: &str) -> Option<Vec<Doclink>> {
        let relative_path = std::path::PathBuf::from(relative_path);
        let Ok(abs_path) = relative_path.canonicalize() else {
            return None;
        };
        let Some(parent_path) = abs_path.parent() else {
            return None;
        };
        tracing::debug!("Find peers: {}", abs_path.display());

        let Some(original_file_name) = relative_path.file_name() else {
            return None;
        };
        let mut files = Vec::new();
        if let Ok(mut read_dir) = tokio::fs::read_dir(parent_path.as_os_str()).await {
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
        Some(files)
    }

    pub fn add_watch(&mut self, path: &Path) -> Result<(), ChimeraError> {
        self.debouncer.watcher().watch(path, RecursiveMode::Recursive)?;
        Ok(())
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<PathBuf> {
        self.broadcast_tx.subscribe()
    }
}

async fn directory_watcher(
    broadcast_tx: tokio::sync::broadcast::Sender<PathBuf>,
    mut file_events: tokio::sync::mpsc::Receiver<Result<Vec<DebouncedEvent>, Vec<NotifyError>>>,
) ->Result<(), ChimeraError> {
    while let Some(Ok(events)) = file_events.recv().await {
        for e in events {
            tracing::debug!("File change event {e:?}");
            match e.event.kind {
                EventKind::Create(f) => {
                    tracing::debug!("File change event: CREATE - {f:?}, {:?}", e.path);
                    broadcast_tx.send(e.path)?;
                },
                EventKind::Modify(f) => {
                    tracing::debug!("File change event: MODIFY - {f:?}, {:?}", e.event.paths);
                    for p in e.event.paths {
                        broadcast_tx.send(p)?;
                    }
                },
                EventKind::Remove(f) => {
                    tracing::debug!("File change event: REMOVE - {f:?}, {:?}", e.path);
                    broadcast_tx.send(e.path)?;
                },
                _ => {}
            };
        }
    }
    Ok(())
}
