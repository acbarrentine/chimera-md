use std::{cmp::Ordering, ffi::OsStr, path::{Path, PathBuf}, time::Duration};
use async_watcher::{notify::{EventKind, RecommendedWatcher, RecursiveMode}, AsyncDebouncer, DebouncedEvent};

use crate::{chimera_error::ChimeraError, document_scraper::Doclink};

type NotifyError = async_watcher::notify::Error;

pub struct FileManager {
    broadcast_tx: tokio::sync::broadcast::Sender<PathBuf>,
    debouncer: AsyncDebouncer<RecommendedWatcher>,
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

    pub async fn find_files_in_directory(&self, abs_path: &Path, skip: Option<&OsStr>) -> Option<Vec<Doclink>> {
        tracing::debug!("Find file in: {}", abs_path.display());
        let mut files = Vec::new();
        if let Ok(mut read_dir) = tokio::fs::read_dir(abs_path.as_os_str()).await {
            while let Ok(entry_opt) = read_dir.next_entry().await {
                if let Some(entry) = entry_opt {
                    let path = entry.path();
                    let file_name = entry.file_name();
                    if let Some(extension) = path.extension() {
                        tracing::debug!("Found {}", path.display());
                        if extension.eq_ignore_ascii_case(OsStr::new("md")) {
                            if let Some(skip) = skip {
                                if file_name.eq(skip) {
                                    continue;
                                }
                            }
                            let name_string = file_name.to_string_lossy().to_string();
                            tracing::debug!("Peer: {}", name_string);
                            files.push(Doclink {
                                anchor: urlencoding::encode(name_string.as_str()).into_owned(),
                                name: name_string,
                                level: 1,
                            });
                        }
                    }
                }
                else {
                    break;
                }
            }
        }
        if files.is_empty() {
            None
        }
        else {
            Some(files)
        }
    }

    pub async fn find_peers(&self, relative_path: &Path, index_file: &str) -> Option<Vec<Doclink>> {
        tracing::debug!("Finding peers of {}", relative_path.display());
        let Ok(abs_path) = relative_path.canonicalize() else {
            tracing::debug!("No canonical representation");
            return None;
        };
        let Some(parent_path) = abs_path.parent() else {
            tracing::debug!("No parent path");
            return None;
        };
        let Some(original_file_name) = relative_path.file_name() else {
            tracing::debug!("No root file");
            return None;
        };
        let mut files = self.find_files_in_directory(parent_path, Some(original_file_name)).await;
        if let Some(files) = files.as_mut() {
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
        }
        files
    }

    pub fn add_watch(&mut self, path: &Path) {
        if let Err(e) = self.debouncer.watcher().watch(path, RecursiveMode::Recursive) {
            tracing::warn!("Error reported adding a watch to {}: {e}", path.display());
        }
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
            tracing::info!("File change event {e:?}");
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
