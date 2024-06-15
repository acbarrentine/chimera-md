use std::{collections::BTreeMap, ffi::OsStr, path::{Path, PathBuf}, sync::Arc};
use tokio::sync::RwLock;
use handlebars::{DirectorySourceOptions, Handlebars};
use serde::Serialize;

use crate::{chimera_error::ChimeraError, document_scraper::{Doclink, DocumentScraper}, file_manager::{EventType, FileEvent, FileManager}, full_text_index::SearchResult};

type CachedResults = Arc<RwLock<BTreeMap<String, String>>>;

pub struct HtmlGenerator {
    handlebars: Handlebars<'static>,
    site_title: String,
    cached_results: CachedResults,
}

#[derive(Serialize)]
struct MarkdownVars {
    site_title: String,
    body: String,
    title: String,
    code_js: String,
    doclinks: Vec<Doclink>,
    peers: Vec<Doclink>,
    num_peers: usize,
}

#[derive(Serialize)]
struct SearchVars {
    site_title: String,
    query: String,
    num_results: usize,
    results: Vec<SearchResult>,
}

#[derive(Serialize)]
struct ErrorVars {
    site_title: String,
    error_code: String,
    heading: String,
    message: String,
}

impl HtmlGenerator {
    pub fn new(
        document_root: &Path,
        template_root: &Path,
        site_title: String,
        file_manager: &mut FileManager
    ) -> Result<HtmlGenerator, ChimeraError> {
        let mut handlebars = Handlebars::new();

        handlebars.register_templates_directory(template_root, DirectorySourceOptions::default())?;
        handlebars.set_dev_mode(true);

        let required_templates = ["markdown", "error", "search"];
        for name in required_templates {
            if !handlebars.has_template(name) {
                let template_name = format!("{name}.hbs");
                tracing::error!("Missing required template: {}{template_name}", template_root.display());
                return Err(ChimeraError::MissingMarkdownTemplate(template_name));
            }
        }

        let cached_results = Arc::new(RwLock::new(BTreeMap::new()));
        let rx = file_manager.subscribe();
        tokio::spawn(listen_for_changes(rx, cached_results.clone(), document_root.to_path_buf()));

        Ok(HtmlGenerator {
            handlebars,
            site_title,
            cached_results,
        })
    }

    pub fn gen_search(&self, query: &str, results: Vec<SearchResult>) -> Result<String, ChimeraError> {
        tracing::info!("Got {} search results", results.len());
        let vars = SearchVars {
            site_title: self.site_title.clone(),
            query: query.to_string(),
            num_results: results.len(),
            results,
        };
        Ok(self.handlebars.render("search", &vars)?)
    }

    pub async fn gen_markdown(
        &self,
        path: &str,
        html_content: String,
        scraper: DocumentScraper,
        peers: Vec<Doclink>
    ) -> Result<String, ChimeraError> {
        tracing::debug!("Peers: {peers:?}");
        let html_content = add_anchors_to_headings(html_content, &scraper.doclinks);

        let code_js = if scraper.has_code_blocks {
            get_language_blob(&scraper.code_languages)
        }
        else {
            String::new()
        };

        let title = scraper.title.unwrap_or_else(||{
            if let Some((_, slashpos)) = path.rsplit_once('/') {
                slashpos.to_string()
            }
            else {
                path.to_string()
            }
        });
    
        let vars = MarkdownVars {
            body: html_content,
            title,
            site_title: self.site_title.clone(),
            code_js,
            doclinks: scraper.doclinks,
            num_peers: peers.len(),
            peers,
        };

        let html = self.handlebars.render("markdown", &vars)?;
        tracing::debug!("Generated fresh response for {path}");

        {
            let mut cache = self.cached_results.write().await;
            cache.insert(path.to_string(), html.clone());
        }

        Ok(html)
    }

    pub fn gen_error(&self, error_code: &str, heading: &str, message: &str) -> Result<String, ChimeraError> {
        let vars = ErrorVars {
            site_title: self.site_title.clone(),
            error_code: error_code.to_string(),
            heading: heading.to_string(),
            message: message.to_string(),
        };
        let html = self.handlebars.render("error", &vars)?;
        Ok(html)
    }
    
    pub async fn get_cached_result(&self, path: &str) -> Option<String> {
        let cache = self.cached_results.read().await;
        cache.get(path).cloned()
    }
}

pub fn add_anchors_to_headings(original_html: String, links: &[Doclink]) -> String {
    let num_links = links.len() - 1;
    if num_links == 0 {
        return original_html;
    }
    let mut link_index = 0;
    let mut new_html = String::with_capacity(original_html.len() * 11 / 10);
    let mut char_iter = original_html.char_indices();
    while let Some((i, c)) = char_iter.next() {
        if link_index < links.len() && c == '<' {
            if let Some(open_slice) = original_html.get(i..i+4) {
                let mut slice_it = open_slice.chars().skip(1);
                if slice_it.next() == Some('h') {
                    if let Some(heading_size) = slice_it.next() {
                        if slice_it.next() == Some('>') {
                            let anchor = links[link_index].anchor.as_str();
                            tracing::debug!("Rewriting anchor: {anchor}");
                            new_html.push_str(format!("<h{heading_size} id=\"{anchor}\">").as_str());
                            link_index += 1;
                            for _ in 0..open_slice.len()-1 {
                                if char_iter.next().is_none() {
                                    return new_html;
                                }
                            }
                            continue;
                        }
                        else if slice_it.next() == Some(' ') {
                            // already has an id?
                            link_index += 1;
                        }
                    }
                }
            }
        }
        new_html.push(c);
    }
    new_html
}

fn get_language_blob(langs: &[&str]) -> String {
    let min_js_prefix = r#"<script src="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/languages/"#;
    let min_js_suffix = r#"".min.js"></script>
    "#;
    let min_jis_len = langs.iter().fold(0, |len, el| {
        len + el.len()
    });

    let style = r#"<link rel="stylesheet" href="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/styles/an-old-hope.min.css">
    "#;
    let highlight_js = r#"<script src="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/highlight.min.js"></script>
    "#;
    let invoke_js = r#"<script>hljs.highlightAll();</script>
    "#;
    let mut buffer = String::with_capacity(
        style.len() +
        highlight_js.len() +
        min_jis_len +
        invoke_js.len());
    buffer.push_str(style);
    buffer.push_str(highlight_js);
    for lang in langs {
        buffer.push_str(min_js_prefix);
        buffer.push_str(lang);
        buffer.push_str(min_js_suffix);
    }
    buffer.push_str(invoke_js);
    buffer
}

async fn remove_cached_result(relative_path: &Path, cache: CachedResults) {
    let path_string = relative_path.to_string_lossy();
    let path_string = path_string.into_owned();
    let mut map = cache.write().await;
    if map.remove(&path_string).is_some() {
        tracing::info!("Removed {path_string} from HTML cache");
    }
}

async fn listen_for_changes(
    mut rx: tokio::sync::broadcast::Receiver<FileEvent>,
    cache: CachedResults,
    document_root: PathBuf,
) {
    while let Ok(event) = rx.recv().await {
        if let Some(ext) = event.path.extension() {
           if ext == OsStr::new("hbs") {
                tracing::info!("Handlebars template {} changed. Discarding all cached results", event.path.display());
                let mut map = cache.write().await;
                map.clear()
            }
            else if ext == OsStr::new("md") {
                match event.kind {
                    EventType::Add => {},
                    _ => {
                        if let Ok(relative_path) = event.path.strip_prefix(document_root.as_path()) {
                            remove_cached_result(relative_path, cache.clone()).await;
                        }
                    }
                }
            }
        }
    }
}
