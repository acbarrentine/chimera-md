use std::{borrow::Borrow, cmp::Ordering, collections::BTreeMap, ffi::OsStr, path::{Path, PathBuf}, time::Duration};
use async_watcher::{notify::{EventKind, RecommendedWatcher, RecursiveMode}, AsyncDebouncer, DebouncedEvent};

use crate::{chimera_error::ChimeraError, document_scraper::Doclink};

type NotifyError = async_watcher::notify::Error;

#[derive(Default, Debug)]
struct FolderInfo {
    folders: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

#[derive(Default, Debug)]
pub struct PeerInfo {
    pub files: Vec<Doclink>,
    pub folders: Vec<Doclink>,
}

pub struct FileManager {
    folders: BTreeMap<PathBuf, FolderInfo>,
    broadcast_tx: tokio::sync::broadcast::Sender<PathBuf>,
    debouncer: AsyncDebouncer<RecommendedWatcher>,
    document_root: PathBuf,
}

impl FileManager {
    pub async fn new(document_root: &Path) -> Result<FileManager, ChimeraError> {
        let (broadcast_tx, _broadcast_rx) = tokio::sync::broadcast::channel(32);
        let (debouncer, file_events) =
            AsyncDebouncer::new_with_channel(Duration::from_secs(1), Some(Duration::from_secs(1))).await?;
        tokio::spawn(directory_watcher(broadcast_tx.clone(), file_events));

        tracing::info!("Walk: {}", document_root.display());
        let mut folders = BTreeMap::new();
        for entry in walkdir::WalkDir::new(document_root).into_iter().flatten() {
            let p = entry.path();
            let parent = p.parent().map_or(PathBuf::from("/"), |p| p.to_path_buf());
            let parent_clone = parent.clone();
            let parent_folder = folders.entry(parent).or_insert(FolderInfo::default());
            if entry.file_type().is_file() {
                let fname = entry.file_name().to_string_lossy();
                if let Some((_stem, ext)) = fname.rsplit_once('.') {
                    if ext.eq_ignore_ascii_case("md") {
                        tracing::debug!("  Adding file {} to {}", p.display(), parent_clone.display());
                        parent_folder.files.push(p.to_path_buf());
                    }
                }
            }
            else {
                tracing::debug!("  Adding folder {} to {}", p.display(), parent_clone.display());
                parent_folder.folders.push(p.to_path_buf());
            }
        }

        Ok(FileManager{
            folders,
            broadcast_tx,
            debouncer,
            document_root: document_root.to_path_buf(),
        })
    }

    pub async fn find_files_in_directory(&self, abs_path: &Path, skip: Option<&OsStr>) -> PeerInfo {
        tracing::info!("Find files in: {}", abs_path.display());
        let mut files = Vec::new();
        let mut folders = Vec::new();
        if let Some(folder_info) = self.folders.get(abs_path) {
            for file in folder_info.files.iter() {
                if let Some(file_name) = file.file_name() {
                    if let Some(skip) = skip {
                        if file_name.eq(skip) {
                            continue;
                        }
                    }
                    let name_string = file_name.to_string_lossy().into_owned();
                    tracing::debug!("Peer: {}", name_string);
                    files.push(Doclink {
                        anchor: urlencoding::encode(name_string.as_str()).into_owned(),
                        name: name_string,
                        level: 1,
                    });
                }
            }

            for folder in folder_info.folders.iter() {
                if let Ok(relative_path) = folder.strip_prefix(self.document_root.as_path()) {
                    let name_string = relative_path.to_string_lossy();
                    folders.push(Doclink {
                        anchor: urlencoding::encode(name_string.borrow()).into_owned(),
                        name: name_string.into_owned(),
                        level: 1,
                    });
                }
            }
        }
        PeerInfo {
            files,
            folders
        }

        // let mut files = Vec::new();
        // if let Ok(mut read_dir) = tokio::fs::read_dir(abs_path.as_os_str()).await {
        //     while let Ok(entry_opt) = read_dir.next_entry().await {
        //         if let Some(entry) = entry_opt {
        //             let path = entry.path();
        //             let file_name = entry.file_name();
        //             if let Some(extension) = path.extension() {
        //                 tracing::debug!("Found {}", path.display());
        //                 if extension.eq_ignore_ascii_case(OsStr::new("md")) {
        //                     if let Some(skip) = skip {
        //                         if file_name.eq(skip) {
        //                             continue;
        //                         }
        //                     }
        //                     let name_string = file_name.to_string_lossy().into_owned();
        //                     tracing::debug!("Peer: {}", name_string);
        //                     files.push(Doclink {
        //                         anchor: urlencoding::encode(name_string.as_str()).into_owned(),
        //                         name: name_string,
        //                         level: 1,
        //                     });
        //                 }
        //             }
        //         }
        //         else {
        //             break;
        //         }
        //     }
        // }
        //files
    }

    pub async fn find_peers(&self, relative_path: &Path, index_file: &str) -> PeerInfo {
        tracing::info!("Finding peers of {}", relative_path.display());
        let Ok(abs_path) = relative_path.canonicalize() else {
            tracing::debug!("No canonical representation");
            return PeerInfo::default();
        };
        let Some(parent_path) = abs_path.parent() else {
            tracing::debug!("No parent path");
            return PeerInfo::default();
        };
        let Some(original_file_name) = relative_path.file_name() else {
            tracing::debug!("No root file");
            return PeerInfo::default();
        };
        let mut peers = self.find_files_in_directory(parent_path, Some(original_file_name)).await;
        peers.files.sort_unstable_by(|a, b| {
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
        peers.folders.sort_unstable_by(|a, b| {
            a.name.cmp(&b.name)
        });
        peers
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
