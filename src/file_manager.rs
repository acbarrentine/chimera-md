use std::{borrow::Borrow, collections::HashMap, ffi::OsStr, path::{Path, PathBuf}, time::Duration};
use async_watcher::{notify::{EventKind, RecommendedWatcher, RecursiveMode}, AsyncDebouncer, DebouncedEvent};
use serde::Serialize;

use crate::{chimera_error::ChimeraError, document_scraper::ExternalLink};

type NotifyError = async_watcher::notify::Error;

#[derive(Default, Debug, Serialize)]
pub struct PeerInfo {
    pub folders: Vec<ExternalLink>,
    pub files: Vec<ExternalLink>,
}

pub struct FileManager {
    broadcast_tx: tokio::sync::broadcast::Sender<PathBuf>,
    debouncer: AsyncDebouncer<RecommendedWatcher>,
    document_root: PathBuf,
    index_file: String,
}

impl FileManager {
    pub async fn new(document_root: &Path, index_file: &str) -> Result<Self, ChimeraError> {
        let (broadcast_tx, _broadcast_rx) = tokio::sync::broadcast::channel(32);
        let (debouncer, file_events) =
            AsyncDebouncer::new_with_channel(Duration::from_secs(1), Some(Duration::from_secs(1))).await?;
        tokio::spawn(directory_watcher(broadcast_tx.clone(), file_events));

        let file_manager = FileManager{
            broadcast_tx,
            debouncer,
            document_root: document_root.to_path_buf(),
            index_file: index_file.to_string(),
        };
        Ok(file_manager)
    }

    pub fn get_markdown_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for entry in walkdir::WalkDir::new(self.document_root.as_path()).into_iter().flatten() {
            let p = entry.path();
            if entry.file_type().is_file() {
                let fname = entry.file_name().to_string_lossy();
                if let Some((_stem, ext)) = fname.rsplit_once('.') {
                    if ext.eq_ignore_ascii_case("md") {
                        files.push(p.to_owned());
                    }
                }
            }
        }
        files
    }

    pub fn find_files_in_directory(&self, abs_path: &Path, skip: Option<&OsStr>) -> Option<PeerInfo> {
        tracing::debug!("Find files in: {}", abs_path.display());
        let mut folder_set: HashMap<PathBuf, Vec<(String, String)>> = HashMap::new();
        let mut files = Vec::new();
        for entry in walkdir::WalkDir::new(abs_path).max_depth(2).into_iter().flatten() {
            let parent = entry.path().parent().map_or(PathBuf::from("/"), |p| p.to_path_buf());
            if entry.file_type().is_file() {
                let fname = entry.file_name();
                let fname_str = fname.to_string_lossy();
                if let Some((stem, ext)) = fname_str.rsplit_once('.') {
                    if ext.eq_ignore_ascii_case("md") {
                        let direct_child = parent.as_os_str().len() == abs_path.as_os_str().len();
                        if direct_child {
                            if let Some(skip) = skip {
                                if fname.eq(skip) {
                                    continue;
                                }
                            }
                            files.push(ExternalLink::new(
                                urlencoding::encode(fname_str.borrow()).into_owned(), 
                                stem.to_string())
                            );
                        }
                        else if let Ok(parent) = parent.strip_prefix(abs_path) {
                            let folder = folder_set.entry(parent.to_owned()).or_default();
                            folder.push((fname_str.to_string(), stem.to_string()));
                        }
                    }
                }
            }
        }

        if files.is_empty() && folder_set.is_empty() {
            return None;
        }
        let mut folders = Vec::with_capacity(folder_set.len());
        for (path, sub_files) in folder_set {
            let path_str = path.to_string_lossy();
            if sub_files.len() == 1 {
                let (fname, stem) = &sub_files[0];
                let url = format!("{}/{}", urlencoding::encode(path_str.borrow()), urlencoding::encode(fname.as_str()));
                let name = format!("{}/{}", path_str, stem);
                files.push(ExternalLink::new(url, name));
            }
            else {
                folders.push(ExternalLink::new(
                    format!("{}/", urlencoding::encode(path_str.borrow())),
                    path_str.into_owned()
                ));
            }
        }
        let mut peers = PeerInfo {
            files,
            folders
        };
        peers.sort();
        Some(peers)
    }

    pub fn find_peers(&self, relative_path: &Path) -> Option<PeerInfo> {
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
        let original_file_name = match original_file_name.eq(self.index_file.as_str()) {
            false => Some(original_file_name),
            true => None,
        };
        self.find_files_in_directory(parent_path, original_file_name)
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

impl PeerInfo {
    fn sort(&mut self) {
        self.files.sort_unstable_by(|a, b| {
            a.name.cmp(&b.name)
        });
        self.folders.sort_unstable_by(|a, b| {
            a.name.cmp(&b.name)
        });
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
