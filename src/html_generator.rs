use std::{cmp::Ordering, collections::BTreeMap, ffi::{OsStr, OsString}, path::{Path, PathBuf}, sync::Arc};
use tokio::sync::RwLock;
use handlebars::{DirectorySourceOptions, Handlebars};
use serde::Serialize;

use crate::{chimera_error::ChimeraError,
    document_scraper::{Doclink, DocumentScraper},
    full_text_index::SearchResult, FileManager, HOME_DIR
};

type CachedResults = Arc<RwLock<BTreeMap<PathBuf, String>>>;

pub struct HtmlGenerator {
    handlebars: Handlebars<'static>,
    site_title: String,
    version: &'static str,
    cached_results: CachedResults,
}

#[derive(Serialize)]
struct MarkdownVars {
    site_title: String,
    version: String,
    body: String,
    title: String,
    code_js: String,
    doclinks: String,
    peers: String,
    breadcrumbs: String,
    peers_len: usize,
    doclinks_len: usize,
}

#[derive(Serialize)]
struct SearchVars {
    site_title: String,
    query: String,
    placeholder: String,
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
        template_root: &Path,
        site_title: String,
        version: &'static str,
        file_manager: &mut FileManager
    ) -> Result<HtmlGenerator, ChimeraError> {
        let mut handlebars = Handlebars::new();

        handlebars.set_dev_mode(true);
        handlebars.register_templates_directory(template_root, DirectorySourceOptions::default())?;
        let required_templates = ["markdown", "error", "search"];
        for name in required_templates {
            if !handlebars.has_template(name) {
                let template_name = format!("{name}.hbs");
                tracing::error!("Missing required template: {}{template_name}", template_root.display());
                return Err(ChimeraError::MissingMarkdownTemplate);
            }
        }

        let cached_results = Arc::new(RwLock::new(BTreeMap::new()));
        let rx = file_manager.subscribe();
        tokio::spawn(listen_for_changes(rx, cached_results.clone()));

        Ok(HtmlGenerator {
            handlebars,
            site_title,
            version,
            cached_results,
        })
    }

    pub fn gen_search(&self, query: &str, results: Vec<SearchResult>) -> Result<String, ChimeraError> {
        tracing::debug!("Got {} search results", results.len());
        let vars = SearchVars {
            site_title: self.site_title.clone(),
            query: query.to_string(),
            placeholder: query.to_string(),
            num_results: results.len(),
            results,
        };
        Ok(self.handlebars.render("search", &vars)?)
    }

    pub fn gen_search_blank(&self) -> Result<String, ChimeraError> {
        tracing::debug!("No query, generating blank search page");
        let vars = SearchVars {
            site_title: self.site_title.clone(),
            query: "".to_string(),
            placeholder: "Search...".to_string(),
            num_results: 0,
            results: Vec::new(),
        };
        Ok(self.handlebars.render("search", &vars)?)
    }

    pub async fn gen_markdown(
        &self,
        path: &std::path::Path,
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
            if let Some(name) = path.file_name() {
                name.to_string_lossy().to_string()
            }
            else {
                path.to_string_lossy().to_string()
            }
        });

        let doclinks_html = generate_doclink_html(scraper.doclinks, true);
        let peers_html = generate_doclink_html(peers, false);
        let breadcrumbs = get_breadcrumbs(path);

        let vars = MarkdownVars {
            body: html_content,
            title,
            site_title: self.site_title.clone(),
            version: self.version.to_string(),
            code_js,
            doclinks_len: doclinks_html.len(),
            doclinks: doclinks_html,
            peers_len: peers_html.len(),
            peers: peers_html,
            breadcrumbs,
        };

        let html = self.handlebars.render("markdown", &vars)?;
        tracing::debug!("Generated fresh response for {}", path.display());

        {
            let mut cache = self.cached_results.write().await;
            cache.insert(path.to_path_buf(), html.clone());
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

    pub async fn get_cached_result(&self, path: &std::path::Path) -> Option<String> {
        let cache = self.cached_results.read().await;
        cache.get(path).cloned()
    }
}

// The indenting scheme requires that we not grow more than 1 step at a time
// Unfortunately, because this depends on user data, we can easily be asked
// to process an invalid setup. Eg: <h1> directly to <h3>
// Outdents don't have the same problem
// Renumber the link list so we don't violate that assumption
fn normalize_headings(doclinks: &mut [Doclink]) -> (usize, usize) {
    let mut last_used_level = 0;
    let mut last_seen_level = 0;
    let mut num_indents = 0;
    let mut text_len = 0;
    for link in doclinks {
        match link.level.cmp(&last_seen_level) {
            Ordering::Greater => {
                num_indents += 1;
                last_seen_level = link.level;
                link.level = last_used_level + 1;
                last_used_level = link.level;
            },
            Ordering::Less => {
                last_used_level = link.level;
                last_seen_level = link.level;
            },
            Ordering::Equal => {
                link.level = last_used_level;
            }
        }
        text_len += link.anchor.len() + link.name.len();
    }
    (num_indents, text_len)
}

fn generate_doclink_html(mut doclinks: Vec<Doclink>, anchors_are_local: bool) -> String {
    if doclinks.is_empty() {
        return "".to_string()
    }
    let (num_indents, text_len) = normalize_headings(&mut doclinks);
    let list_prefix = "<ul>\n";
    let list_suffix = "</ul>\n";
    let item_prefix = if anchors_are_local {"<li><a href=\"#"} else {"<li><a href=\""};
    let item_middle = "\">";
    let item_suffix = "</a></li>\n";
    let expected_size = (num_indents * (list_prefix.len() + list_suffix.len())) +
        (doclinks.len() * (item_prefix.len() + item_middle.len() + item_suffix.len())) +
        text_len;
    let mut last_level = 0;
    let mut html = String::with_capacity(expected_size);
    for link in doclinks {
        if last_level < link.level {
            html.push_str(list_prefix);
            last_level = link.level;
        }
        else {
            while last_level != link.level {
                html.push_str(list_suffix);
                last_level -= 1;
            }
        }
        html.push_str(item_prefix);
        html.push_str(link.anchor.as_str());
        html.push_str(item_middle);
        html.push_str(link.name.as_str());
        html.push_str(item_suffix);
    }
    while last_level > 0 {
        html.push_str(list_suffix);
        last_level -= 1;
    }
    assert_eq!(html.len(), expected_size);
    html
}

fn add_anchors_to_headings(original_html: String, links: &[Doclink]) -> String {
    let num_links = links.len();
    if num_links == 1 {
        return original_html;
    }
    let mut link_index = 1;
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
        len + el.len() + min_js_prefix.len() + min_js_suffix.len()
    });

    let style = r#"<link rel="stylesheet" href="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/styles/an-old-hope.min.css">
    "#;
    let highlight_js = r#"<script src="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/highlight.min.js"></script>
    "#;
    let invoke_js = r#"<script>hljs.highlightAll();</script>
    "#;
    let expected_len = style.len() + highlight_js.len() + min_jis_len + invoke_js.len();
    let mut buffer = String::with_capacity(expected_len);
    buffer.push_str(style);
    buffer.push_str(highlight_js);
    for lang in langs {
        buffer.push_str(min_js_prefix);
        buffer.push_str(lang);
        buffer.push_str(min_js_suffix);
    }
    buffer.push_str(invoke_js);
    assert_eq!(buffer.len(), expected_len);
    buffer
}

const HOME_PREFIX: &str = r#"<span class="home"><a href=""#;
const HOME_SUFFIX: &str = r#"">Home</a></span>"#;
const CRUMB_PREFIX: &str = r#"<span class="crumb"><a href=""#;
const CRUMB_MIDDLE: &str = r#"">"#;
const CRUMB_SUFFIX: &str = r#"</a></span>"#;
const FINAL_PREFIX: &str = r#"<span class="crumb">"#;
const FINAL_SUFFIX: &str = r#"</span>"#;

fn get_breadcrumb_name_and_anchor_len(parts: &[&OsStr]) -> (usize, usize) {
    let mut anchor_len = HOME_DIR.len();
    let mut prev_anchor_len = anchor_len;
    let mut name_len = 0;
    for str in &parts[0..parts.len()-1] {
        let new_anchor_len = prev_anchor_len + str.len() + 1;
        anchor_len += new_anchor_len;
        prev_anchor_len = new_anchor_len;
        name_len += str.len();
    }
    name_len += parts[parts.len() - 1].len();
    (name_len, anchor_len)
}

fn get_breadcrumbs_len(parts: &[&OsStr]) -> usize {
    let (name_len, anchor_len) = get_breadcrumb_name_and_anchor_len(parts);
    (HOME_PREFIX.len() + HOME_SUFFIX.len()) +
        (parts.len() - 1) * (CRUMB_PREFIX.len() + CRUMB_MIDDLE.len() + CRUMB_SUFFIX.len()) +
        (FINAL_PREFIX.len() + FINAL_SUFFIX.len()) +
        anchor_len + name_len
}

fn get_breadcrumbs(path: &Path) -> String {
    let mut url = OsString::from(HOME_DIR);
    let parts: Vec<&OsStr> = path.iter().collect();
    let expected_len = get_breadcrumbs_len(&parts);
    let mut breadcrumbs = OsString::with_capacity(expected_len);
    breadcrumbs.push(HOME_PREFIX);
    breadcrumbs.push(url.as_os_str());
    breadcrumbs.push(HOME_SUFFIX);
    let num_parts = parts.len();
    for part in &parts[0..num_parts-1] {
        url.push(part);
        url.push("/");
        breadcrumbs.push(CRUMB_PREFIX);
        breadcrumbs.push(url.as_os_str());
        breadcrumbs.push(CRUMB_MIDDLE);
        breadcrumbs.push(part);
        breadcrumbs.push(CRUMB_SUFFIX);
    }
    breadcrumbs.push(FINAL_PREFIX);
    breadcrumbs.push(parts[num_parts-1]);
    breadcrumbs.push(FINAL_SUFFIX);
    assert_eq!(breadcrumbs.len(), expected_len);
    breadcrumbs.to_string_lossy().to_string()
}

async fn listen_for_changes(
    mut rx: tokio::sync::broadcast::Receiver<PathBuf>,
    cache: CachedResults,
) {
    while let Ok(path) = rx.recv().await {
        tracing::info!("HG change event {}", path.display());
        if let Some(ext) = path.extension() {
            if ext == OsStr::new("hbs") || ext == OsStr::new("md") {
                tracing::info!("Discarding cached HTML results");
                let mut map = cache.write().await;
                map.clear()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_doclink_normalization_1() {
        let mut nochange = vec![
            Doclink::new("a".to_string(), "a".to_string(), 1),
            Doclink::new("b".to_string(), "b".to_string(), 2),
            Doclink::new("c".to_string(), "c".to_string(), 1),
        ];
        let (num_indents, text_len) = normalize_headings(&mut nochange);
        assert_eq!(num_indents, 2);
        assert_eq!(text_len, 6);
        assert_eq!(nochange[0].level, 1);
        assert_eq!(nochange[1].level, 2);
        assert_eq!(nochange[2].level, 1);
    }

    #[test]
    fn test_doclink_normalization_2() {
        let mut bad_initial_level = vec![
            Doclink::new("a".to_string(), "a".to_string(), 2),
        ];
        let (num_indents, text_len) = normalize_headings(&mut bad_initial_level);
        assert_eq!(num_indents, 1);
        assert_eq!(text_len, 2);
        assert_eq!(bad_initial_level[0].level, 1);
    }

    #[test]
    fn test_doclink_normalization_3() {
        let mut bad_growth = vec![
            Doclink::new("a".to_string(), "a".to_string(), 1),
            Doclink::new("c".to_string(), "c".to_string(), 3),
        ];
        let (num_indents, text_len) = normalize_headings(&mut bad_growth);
        assert_eq!(num_indents, 2);
        assert_eq!(text_len, 4);
        assert_eq!(bad_growth[0].level, 1);
        assert_eq!(bad_growth[1].level, 2);
    }

    #[test]
    fn test_doclink_normalization_4() {
        let mut series_continues_the_jump = vec![
            Doclink::new("a".to_string(), "a".to_string(), 1),
            Doclink::new("b".to_string(), "b".to_string(), 3),
            Doclink::new("c".to_string(), "c".to_string(), 3),
        ];
        let (num_indents, text_len) = normalize_headings(&mut series_continues_the_jump);
        assert_eq!(num_indents, 2);
        assert_eq!(text_len, 6);
        assert_eq!(series_continues_the_jump[0].level, 1);
        assert_eq!(series_continues_the_jump[1].level, 2);
        assert_eq!(series_continues_the_jump[2].level, 2);
    }

    #[test]
    fn test_doclink_normalization_5() {
        let mut series_continues_the_jump = vec![
            Doclink::new("a".to_string(), "a".to_string(), 1),
            Doclink::new("b".to_string(), "b".to_string(), 2),
            Doclink::new("c".to_string(), "c".to_string(), 2),
            Doclink::new("d".to_string(), "d".to_string(), 3),
            Doclink::new("e".to_string(), "e".to_string(), 2),
            Doclink::new("f".to_string(), "f".to_string(), 3),
            Doclink::new("g".to_string(), "g".to_string(), 3),
            Doclink::new("h".to_string(), "h".to_string(), 3),
        ];
        let (num_indents, text_len) = normalize_headings(&mut series_continues_the_jump);
        assert_eq!(num_indents, 4);
        assert_eq!(text_len, 16);
    }

    #[test]
    fn test_breadcrumbs_1() {
        let path = PathBuf::from("Documents/Example/index.md");
        let parts: Vec<&OsStr> = path.iter().collect();
        let (name_len, anchor_len) = get_breadcrumb_name_and_anchor_len(&parts);
        assert_eq!(name_len, 24);
        assert_eq!(anchor_len, 31);
    }

    #[test]
    fn test_breadcrumbs_2() {
        let path = PathBuf::from("Documents/Example/Recipes/pizza.md");
        let parts: Vec<&OsStr> = path.iter().collect();
        let (name_len, anchor_len) = get_breadcrumb_name_and_anchor_len(&parts);
        assert_eq!(name_len, 31);
        assert_eq!(anchor_len, 58);
    }

    #[test]
    fn test_breadcrumbs_3() {
        let path = PathBuf::from("index.md");
        let parts: Vec<&OsStr> = path.iter().collect();
        let (name_len, anchor_len) = get_breadcrumb_name_and_anchor_len(&parts);
        assert_eq!(name_len, 8);
        assert_eq!(anchor_len, 1);
    }
}
