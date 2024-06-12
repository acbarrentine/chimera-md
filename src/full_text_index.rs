use tantivy::{collector::TopDocs, IndexReader};
use tantivy::query::{FuzzyTermQuery, Query, QueryParser};
use tantivy::{schema::*, Searcher, TantivyError};
use tantivy::{Index, IndexWriter, ReloadPolicy};
use tempfile::TempDir;

use crate::chimera_error::ChimeraError;
use crate::document_scraper::Doclink;
use crate::Config;

/*
 * Todo
 * * Add documents to work queue, add to index on background thread
 * * Fuzzy search
 * * Watch documents for changes
 */

pub struct FullTextIndex {
    #[allow(dead_code)]     // It's not actually dead...
    index_path: TempDir,    // I need to keep the TempDir alive for the life of the index 
    index: Index,
    title: Field,
    anchor: Field,
    body: Field,
    index_reader: IndexReader,
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry.file_name()
         .to_str()
         .map(|s| s.starts_with('.'))
         .unwrap_or(false)
}

impl FullTextIndex {
    pub async fn new(config: &Config) -> Result<Self, ChimeraError>{
        let index_path = TempDir::new()?;
        let mut schema_builder = Schema::builder();
        schema_builder.add_text_field("title", TEXT | STORED);
        schema_builder.add_text_field("anchor", TEXT | STORED);
        schema_builder.add_text_field("body", TEXT);
        let schema = schema_builder.build();
        let index = Index::create_in_dir(&index_path, schema.clone())?;
        let mut index_writer: IndexWriter = index.writer(50_000_000)?;
        let title = schema.get_field("title").unwrap();
        let anchor = schema.get_field("anchor").unwrap();
        let body = schema.get_field("body").unwrap();
    
        for entry in walkdir::WalkDir::new(config.document_root.as_str())
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| !is_hidden(e))
            .flatten() {
            if entry.file_type().is_file() {
                let path = entry.path();
                if let Some(ext) = path.extension() {
                    if ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("html") {
                        let mut doc = TantivyDocument::default();
                        if let Ok(relative_path) = entry.path().strip_prefix(config.document_root.as_str()) {
                            let anchor_string = relative_path.to_string_lossy();
                            let title_string = entry.file_name().to_string_lossy();
                            tracing::info!("Adding {title_string} to full-text index");
                            doc.add_text(title, title_string);
                            doc.add_text(anchor, anchor_string);
                            doc.add_text(body, tokio::fs::read_to_string(path).await?);
                            index_writer.add_document(doc)?;
                        }
                        
                    }
                }
            }
        }
        index_writer.commit()?;
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
            index_reader,
        };
        Ok(fti)
    }

    fn execute_query(&self, searcher: &Searcher, query: &dyn Query) -> Result<Vec<Doclink>, TantivyError> {
        let mut results = Vec::new();
        let top_docs = searcher.search(query, &TopDocs::with_limit(10))?;
        for (_score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;
            let title = retrieved_doc.get_first(self.title);
            let anchor = retrieved_doc.get_first(self.anchor);
            tracing::debug!("Search result: {title:?} {anchor:?}");
            if let Some(OwnedValue::Str(title)) = title {
                if let Some(OwnedValue::Str(anchor)) = anchor {
                    results.push(Doclink {
                        anchor: anchor.clone(),
                        name: title.clone(),
                    });
                }
            }
        }
        Ok(results)
    }

    pub async fn search(&self, query_str: &str) -> Result<Vec<Doclink>, ChimeraError> {
        let searcher = self.index_reader.searcher();

        // probably want to soft fail on the query error here
        let query_parser = QueryParser::for_index(&self.index, vec![self.title, self.body]);
        let query = query_parser.parse_query(query_str)?;
        let mut results = self.execute_query(&searcher, &query)?;
        tracing::debug!("Initial query failed. Trying fuzzy search for {query_str}");
        if results.is_empty() {
            let term = Term::from_field_text(self.body, query_str);
            let query = FuzzyTermQuery::new(term, 2, true);
            results = match self.execute_query(&searcher, &query) {
                Ok(results) => results,
                Err(e) => {
                    tracing::warn!("Fuzzy search error: {e}");
                    Vec::new()
                }
            }
        }
        tracing::debug!("Result count: {}", results.len());
        Ok(results)
    }
}
