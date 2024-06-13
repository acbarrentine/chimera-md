use core::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::usize;
use serde::Serialize;
use tantivy::{collector::TopDocs, IndexReader};
use tantivy::query::QueryParser;
use tantivy::{schema::*, SnippetGenerator};
use tantivy::{Index, IndexWriter, ReloadPolicy};
use tantivy::tokenizer::NgramTokenizer;
use tempfile::TempDir;
use tokio::sync::mpsc::{self, Receiver};

use crate::chimera_error::ChimeraError;

/*
 * Todo:
 * Watch documents for changes
 * Strip HTML from documents before indexing
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
    title: Field,
    anchor: Field,
    body: Field,
    index_writer: Arc<RwLock<IndexWriter>>,
    index_reader: IndexReader,
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
        schema_builder.add_text_field("title", TEXT | STORED);
        schema_builder.add_text_field("anchor", TEXT | STORED);
        schema_builder.add_text_field("body", text_options);
        let schema = schema_builder.build();

        let index = Index::create_in_dir(&index_path, schema.clone())?;
        index.tokenizers().register("ngram4", NgramTokenizer::new(4, 4, false).unwrap());
        let title = schema.get_field("title").unwrap();
        let anchor = schema.get_field("anchor").unwrap();
        let body = schema.get_field("body").unwrap();
        let index_writer = Arc::new(RwLock::new(index.writer(50_000_000)?));

        // index_writer.commit()?;
        let index_reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        let fti = FullTextIndex {
            index_path,
            index,
            title,
            anchor,
            body,
            index_writer,
            index_reader,
        };
        Ok(fti)
    }
    
    pub async fn scan_directory(&self, root_directory: &str) -> Result<(), ChimeraError> {
        let (tx, rx) = mpsc::channel::<PathBuf>(32);
        let scanner = DocumentScanner {
            index_writer: self.index_writer.clone(),
            work_queue: rx,
            document_root: root_directory.to_string(),
            title: self.title,
            link: self.anchor,
            body: self.body,
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
        Ok(())
    }

    pub async fn search(&self, query_str: &str) -> Result<Vec<SearchResult>, ChimeraError> {
        let searcher = self.index_reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.title, self.body]);
        let query = query_parser.parse_query(query_str)?;
        let mut results = Vec::new();
        let snippet_generator = SnippetGenerator::create(&searcher, &query, self.body)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(10))?;
        for (_score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;
            let title = retrieved_doc.get_first(self.title);
            let anchor = retrieved_doc.get_first(self.anchor);
            tracing::debug!("Search result: {title:?} {anchor:?}");
            if let Some(OwnedValue::Str(title)) = title {
                if let Some(OwnedValue::Str(anchor)) = anchor {
                    let snippet = snippet_generator.snippet_from_doc(&retrieved_doc);
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
}

// Ngram tokenizer causes the snippet highlight ranges to overlap for longer search terms
// "table" => "tabl" + "able"
fn normalize_ranges(ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut results = Vec::with_capacity(ranges.len());
    let mut start = 0;
    let mut end = 0;
    for r in ranges {
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
    }
    if start != end {
        results.push(Range { start, end });
    }
    tracing::debug!("Normalized spans: {results:?}");
    results
}

fn highlight(snippet: &str, highlights: &[Range<usize>]) -> String {
    tracing::debug!("Highlight {snippet}");
    tracing::debug!("Spans: {highlights:?}");
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

impl DocumentScanner {
    async fn scan(mut self) -> Result<(), ChimeraError> {
        while let Some(path) = self.work_queue.recv().await {
            let mut doc = TantivyDocument::default();
            if let Ok(relative_path) = path.strip_prefix(self.document_root.as_str()) {
                let anchor_string = relative_path.to_string_lossy();
                if let Some(title_string) = path.file_name() {
                    let title_string = title_string.to_string_lossy();
                    tracing::info!("Adding {} to full-text index", title_string);
                    doc.add_text(self.title, title_string);
                    doc.add_text(self.link, anchor_string);
                    doc.add_text(self.body, tokio::fs::read_to_string(path).await?);
                    {
                        let index = self.index_writer.read()?;
                        index.add_document(doc)?;
                    }
                }
            }

            // commit?
            if self.work_queue.is_empty() {
                let mut index = self.index_writer.write()?;
                index.commit()?;
            }
        }
        Ok(())
    }
}
