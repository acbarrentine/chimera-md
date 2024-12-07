use core::ops::Range;
use std::{collections::BTreeMap, ffi::OsStr, path::PathBuf, sync::{Arc, RwLock}, time::SystemTime};
use serde::{Deserialize, Serialize};
use tantivy::{collector::TopDocs, directory::MmapDirectory, IndexReader};
use tantivy::query::QueryParser;
use tantivy::{schema::*, SnippetGenerator};
use tantivy::{Index, IndexWriter, ReloadPolicy};
use tokio::{io::AsyncWriteExt, sync::mpsc::{self, Receiver}};

use crate::chimera_error::ChimeraError;
use crate::file_manager::FileManager;

#[derive(Serialize)]
pub struct SearchResult {
    title: String,
    link: String,
    snippet: String,
}

type FileMapType = BTreeMap<PathBuf, SystemTime>;

#[derive(Default, Serialize, Deserialize)]
struct FileTimes {
    index_location: PathBuf,
    files: FileMapType,
}

pub struct FullTextIndex {
    index: Index,
    title_field: Field,
    link_field: Field,
    body_field: Field,
    index_writer: Arc<RwLock<IndexWriter>>,
    index_reader: IndexReader,
}

struct DocumentScanner {
    index_writer: Arc<RwLock<IndexWriter>>,
    file_times: FileTimes,
    work_queue: Receiver<PathBuf>,
    document_root: PathBuf,
    title: Field,
    link: Field,
    body: Field,
}

impl FullTextIndex {
    pub fn new(index_path: &std::path::Path) -> Result<Self, ChimeraError> {
        let text_field_indexing = TextFieldIndexing::default()
            .set_tokenizer("en_stem")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions);

        let text_options = TextOptions::default()
            .set_indexing_options(text_field_indexing)
            .set_stored();

        let mut schema_builder = Schema::builder();
        let title_field = schema_builder.add_text_field("title", STRING | STORED);
        let link_field = schema_builder.add_text_field("link", STRING | STORED);
        let body_field = schema_builder.add_text_field("body", text_options);
        let schema = schema_builder.build();

        let dir = MmapDirectory::open(index_path)?;
        let index = Index::open_or_create(dir, schema.clone())?;
        let index_writer = Arc::new(RwLock::new(index.writer(50_000_000)?));

        let index_reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        let fti = FullTextIndex {
            index,
            title_field,
            link_field,
            body_field,
            index_writer,
            index_reader,
        };
        Ok(fti)
    }
    
    pub async fn scan_directory(
        &self,
        root_directory: PathBuf,
        search_index_dir: PathBuf,
        file_manager: &FileManager
    ) -> Result<(), ChimeraError> {
        let file_times = FileTimes::try_load(search_index_dir).await;

        let (tx, rx) = mpsc::channel::<PathBuf>(32);
        let scanner = DocumentScanner {
            index_writer: self.index_writer.clone(),
            file_times,
            work_queue: rx,
            document_root: root_directory.to_path_buf(),
            title: self.title_field,
            link: self.link_field,
            body: self.body_field,
        };
        tokio::spawn(scanner.scan());

        let md_files = file_manager.get_markdown_files();
        for md in md_files {
            tx.send(md).await?;
        }

        let change_rx = file_manager.subscribe();
        tokio::spawn(listen_for_changes(change_rx, tx));

        Ok(())
    }

    pub fn search(&self, query_str: &str) -> Result<Vec<SearchResult>, ChimeraError> {
        let searcher = self.index_reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.body_field]);
        let query = query_parser.parse_query(query_str)?;
        let mut results = Vec::new();
        let snippet_generator = SnippetGenerator::create(&searcher, &query, self.body_field)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(10))?;
        for (_score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;
            let title = retrieved_doc.get_first(self.title_field);
            let anchor = retrieved_doc.get_first(self.link_field);
            tracing::debug!("Search result: {title:?} {anchor:?}");
            if let Some(OwnedValue::Str(title)) = title {
                if let Some(OwnedValue::Str(anchor)) = anchor {
                    let snippet = snippet_generator.snippet_from_doc(&retrieved_doc);
                    tracing::debug!("Snippet: {snippet:?}");
                    let snippet = self.highlight(snippet.fragment(), snippet.highlighted());
                    results.push(SearchResult {
                        title: title.clone(),
                        link: anchor.clone(),
                        snippet,
                    });
                }
            }
        }
        tracing::debug!("Result count: {}", results.len());
        Ok(results)
    }

    fn highlight(&self, snippet: &str, highlights: &[Range<usize>]) -> String {
        let prefix = "<span class=\"highlight\">";
        let suffix = "</span>";
        let per_highlight_len = prefix.len() + suffix.len();
        let mut result = String::with_capacity(snippet.len() + (highlights.len() * per_highlight_len));
        let mut start = 0_usize;
        let highlights = normalize_ranges(highlights);
        for blurb in highlights {
            result.push_str(tera::escape_html(&snippet[start..blurb.start]).as_str());
            result.push_str(prefix);
            result.push_str(&snippet[blurb.start..blurb.end]);
            result.push_str(suffix);
            start = blurb.end;
        }
        result.push_str(tera::escape_html(&snippet[start..]).as_str());
        result
    }
}

// Ngram tokenizer causes the snippet highlight ranges to overlap for longer search terms
// "table" => "tabl" + "able"
fn normalize_ranges(ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut results = Vec::with_capacity(ranges.len());
    let mut start = 0;
    let mut end = 0;
    ranges.iter().for_each(|r| {
        if r.start > end {
            if start != end {
                results.push(Range { start, end });
            }
            start = r.start;
            end = r.end;
        }
        else {
            end = r.end;
        }
    });
    if start != end {
        results.push(Range { start, end });
    }
    tracing::debug!("Normalized spans: {results:?}");
    results
}

async fn get_modtime(path: &std::path::Path) -> Option<SystemTime> {
    if let Ok(metadata) = tokio::fs::metadata(path).await {
        if let Ok(modtime) = metadata.modified() {
            return Some(modtime);
        }
    }
    None
}

impl DocumentScanner {
    async fn prune_deleted_documents(&mut self) -> Result<(), ChimeraError> {
        // look for deleted documents since we last ran
        let mut deleted = Vec::new();
        self.file_times.files.retain(|path, _time| {
            if !path.exists() {
                deleted.push(path.clone());
                false
            }
            else {
                true
            }
        });
        if !deleted.is_empty()
        {
            let mut index = self.index_writer.write()?;
            for del in deleted {
                if let Ok(relative_path) = del.strip_prefix(self.document_root.as_path()) {
                    let anchor_string = format!("/home/{}", relative_path.to_string_lossy());
                    tracing::debug!("Removing deleted document {} from full text index", del.display());
                    let doc_term = Term::from_field_text(self.link, &anchor_string);
                    index.delete_term(doc_term);
                }
            }
            index.commit()?;
        }
        Ok(())
    }

    async fn scan(mut self) -> Result<(), ChimeraError> {
        self.prune_deleted_documents().await?;

        let mut docs_since_last_commit = 0;
        while let Some(path) = self.work_queue.recv().await {
            let modtime = get_modtime(path.as_path()).await;
            if self.file_times.check_up_to_date(path.as_path(), modtime) {
                continue;
            }

            let mut doc = TantivyDocument::default();
            if let Ok(relative_path) = path.strip_prefix(self.document_root.as_path()) {
                let anchor_string = format!("/home/{}", relative_path.to_string_lossy());

                tracing::debug!("Removing {anchor_string} from full text index");
                let doc_term = Term::from_field_text(self.link, &anchor_string);
                {
                    let index = self.index_writer.write()?;
                    index.delete_term(doc_term);
                }

                if let Some(title_string) = path.file_name() {
                    let title_string = title_string.to_string_lossy();
                    if let Ok(body_text) = tokio::fs::read_to_string(path.as_path()).await {
                        tracing::debug!("Adding {} to full-text index", title_string);
                        doc.add_text(self.title, title_string);
                        doc.add_text(self.link, anchor_string);
                        doc.add_text(self.body, body_text);
                        {
                            let index = self.index_writer.write()?;
                            index.add_document(doc)?;
                        }
                    }
                    docs_since_last_commit += 1;
                }
            }

            // commit?
            if self.work_queue.is_empty() || docs_since_last_commit > 20 {
                self.file_times.save().await?;
                let mut index = self.index_writer.write()?;
                index.commit()?;
                docs_since_last_commit = 0;
            }
        }
        Ok(())
    }
}

async fn listen_for_changes(
    mut rx: tokio::sync::broadcast::Receiver<PathBuf>,
    tx: tokio::sync::mpsc::Sender<PathBuf>,
) {
    while let Ok(path) = rx.recv().await {
        tracing::debug!("FTI change event {}", path.display());
        if let Some(ext) = path.extension() {
            if ext == OsStr::new("md") {
                // forward to the DocumentScanner
                let _ = tx.send(path).await;
            }
        }
    }
}

impl FileTimes {
    async fn try_load(search_index_dir: PathBuf) -> FileTimes {
        let index_file = search_index_dir.join("ft.toml");
        let times = match tokio::fs::read_to_string(index_file.as_path()).await {
            Ok(f) => {
                toml::from_str(f.as_str()).unwrap_or_default()
            },
            Err(_) => {
                FileMapType::default()
            }
        };
        FileTimes {
            index_location: index_file,
            files: times,
        }
    }

    fn check_up_to_date(&mut self, path: &std::path::Path, current_modtime: Option<SystemTime>) -> bool {
        if current_modtime.is_none() {
            // No such file, remove from index, if it's there
            tracing::debug!("File not in ft.toml: {}", path.display());
            let _ = self.files.remove(path);
            return false;
        }
        let current_modtime = current_modtime.unwrap();
        let last_modtime = self.files.get(path);
        if last_modtime.is_some() && *last_modtime.unwrap() == current_modtime {
            tracing::debug!("Up-to-date in ft.toml: {}", path.display());
            return true;
        }
        tracing::debug!("Adding to ft.toml: {}", path.display());
        self.files.insert(path.to_path_buf(), current_modtime);
        false
    }

    async fn save(&self) -> Result<(), ChimeraError> {
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(self.index_location.as_path())
            .await?;
        match toml::to_string(&self.files) {
            Ok(toml) => {
                match tokio::fs::File::write_all(&mut file, toml.as_bytes()).await {
                    Ok(_) => {
                        tracing::debug!("Saved ft.toml");
                    },
                    Err(e) => {
                        tracing::warn!("Failure writing full text index file times: {e}");
                    }
                }
            },
            Err(e) => {
                tracing::warn!("Failure converting file times to toml: {e}");
            }
        }
        Ok(())
    }
}
