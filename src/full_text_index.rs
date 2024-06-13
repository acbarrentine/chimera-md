use core::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use serde::Serialize;
use tantivy::{collector::TopDocs, IndexReader};
use tantivy::query::QueryParser;
use tantivy::{schema::*, SnippetGenerator};
use tantivy::{Index, IndexWriter, ReloadPolicy};
use tantivy::tokenizer::NgramTokenizer;
use tempfile::TempDir;
use tokio::sync::mpsc::{self, Receiver, Sender};

use crate::chimera_error::ChimeraError;

/*
 * Todo:
 * Watch documents for changes
 */

#[derive(Serialize)]
 pub struct SearchResult {
    title: String,
    link: String,
    snippet: String,
}

pub struct FullTextIndex {
    #[allow(dead_code)]     // It's not actually dead...
    index_path: TempDir,    // I need to keep the TempDir alive for the life of the index 

    index: Index,
    title_field: Field,
    link_field: Field,
    body_field: Field,
    index_writer: Arc<RwLock<IndexWriter>>,
    index_reader: IndexReader,
    scan_queue: Option<Sender<PathBuf>>,
}

struct DocumentScanner {
    index_writer: Arc<RwLock<IndexWriter>>,
    work_queue: Receiver<PathBuf>,
    document_root: String,
    title: Field,
    link: Field,
    body: Field,
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry.file_name()
         .to_str()
         .map(|s| s.starts_with('.'))
         .unwrap_or(false)
}

impl FullTextIndex {
    pub fn new() -> Result<Self, ChimeraError> {
        let index_path = TempDir::new()?;

        let text_field_indexing = TextFieldIndexing::default()
            .set_tokenizer("ngram4")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions);

        let text_options = TextOptions::default()
            .set_indexing_options(text_field_indexing)
            .set_stored();

        let mut schema_builder = Schema::builder();
        let title_field = schema_builder.add_text_field("title", STRING | STORED);
        let link_field = schema_builder.add_text_field("link", STRING | STORED);
        let body_field = schema_builder.add_text_field("body", text_options);
        let schema = schema_builder.build();

        let index = Index::create_in_dir(&index_path, schema.clone())?;
        index.tokenizers().register("ngram4", NgramTokenizer::new(4, 4, false).unwrap());
        let index_writer = Arc::new(RwLock::new(index.writer(50_000_000)?));

        let index_reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        let fti = FullTextIndex {
            index_path,
            index,
            title_field,
            link_field,
            body_field,
            index_writer,
            index_reader,
            scan_queue: None,
        };
        Ok(fti)
    }
    
    pub async fn scan_directory(&mut self, root_directory: &str) -> Result<(), ChimeraError> {
        let (tx, rx) = mpsc::channel::<PathBuf>(32);
        let scanner = DocumentScanner {
            index_writer: self.index_writer.clone(),
            work_queue: rx,
            document_root: root_directory.to_string(),
            title: self.title_field,
            link: self.link_field,
            body: self.body_field,
        };
        tokio::spawn(scanner.scan());

        for entry in walkdir::WalkDir::new(root_directory)
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| !is_hidden(e))
            .flatten() {
            if entry.file_type().is_file() {
                let path = entry.path();
                if let Some(ext) = path.extension() {
                    if ext.eq_ignore_ascii_case("md") {
                        tx.send(entry.path().to_owned()).await?;
                    }
                }
            }
        }
        self.scan_queue = Some(tx);

        Ok(())
    }

    pub async fn search(&self, query_str: &str) -> Result<Vec<SearchResult>, ChimeraError> {
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
                    //tracing::info!("Snippet: {snippet:?}");
                    let snippet = highlight(snippet.fragment(), snippet.highlighted());
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

    pub async fn rescan_document(&self, path: &Path) {
        if let Some(sender) = &self.scan_queue {
            let _ = sender.send(path.to_owned()).await;
        }
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

fn highlight(snippet: &str, highlights: &[Range<usize>]) -> String {
    let prefix = "<span class=\"highlight\">";
    let suffix = "</span>";
    let highlight_len = prefix.len() + suffix.len();
    let mut result = String::with_capacity(snippet.len() + (highlights.len() * highlight_len));
    let mut start = 0_usize;
    let highlights = normalize_ranges(highlights);
    for blurb in highlights {
        result.push_str(&snippet[start..blurb.start]);
        result.push_str(prefix);
        result.push_str(&snippet[blurb.start..blurb.end]);
        result.push_str(suffix);
        start = blurb.end;
    }
    result.push_str(&snippet[start..]);
    result
}

fn strip_html(body: String) -> String {
    let body = html2text::from_read(body.as_bytes(), body.len());
    body
}

impl DocumentScanner {
    async fn scan(mut self) -> Result<(), ChimeraError> {
        let mut docs_since_last_commit = 0;
        while let Some(path) = self.work_queue.recv().await {
            let mut doc = TantivyDocument::default();
            if let Ok(relative_path) = path.strip_prefix(self.document_root.as_str()) {
                let anchor_string = relative_path.to_string_lossy();

                let doc_term = Term::from_field_text(self.link, &anchor_string);
                {
                    tracing::debug!("Removing {anchor_string} from full text index");
                    let index = self.index_writer.write()?;
                    index.delete_term(doc_term);
                }

                if let Some(title_string) = path.file_name() {
                    let title_string = title_string.to_string_lossy();
                    if let Ok(body_text) = tokio::fs::read_to_string(path.as_path()).await {
                        let body_text = strip_html(body_text);

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
                let mut index = self.index_writer.write()?;
                index.commit()?;
                docs_since_last_commit = 0;
            }
        }
        Ok(())
    }
}
