use tantivy::{collector::TopDocs, IndexReader};
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, IndexWriter, ReloadPolicy};
use tempfile::TempDir;

use crate::chimera_error::ChimeraError;
use crate::document_scraper::Doclink;
use crate::Config;

pub struct FullTextIndex {
    index_path: TempDir,
    index: Index,
    schema: Schema,
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
    
        // Todo
        // I obviously need to be reading these on a background thread
        // And because the index is persistent, I'll also need to make
        // a directory watcher and refresh them if something changes
        // (requires deleting and re-adding the doc)
    
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
            schema,
            title,
            anchor,
            body,
            index_reader,
        };
        Ok(fti)
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Doclink>, ChimeraError> {
        let mut results = Vec::new();
        let searcher = self.index_reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.title, self.body]);
        // probably want to soft fail on the query error here
        let query = query_parser.parse_query(query)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(10))?;
        for (_score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;
            let title = retrieved_doc.get_first(self.title);
            let anchor = retrieved_doc.get_first(self.anchor);
            tracing::info!("Search result: {title:?} {anchor:?}");
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
}
