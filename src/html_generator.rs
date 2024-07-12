use std::{cmp::Ordering, ffi::{OsStr, OsString}, path::{Path, PathBuf}};
use handlebars::{DirectorySourceOptions, Handlebars};
use serde::Serialize;

use crate::{chimera_error::ChimeraError, document_scraper::{Doclink, DocumentScraper}, file_manager::PeerInfo, full_text_index::SearchResult, HOME_DIR
};

pub struct HtmlGeneratorCfg<'a> {
    pub template_root: PathBuf,
    pub site_title: String,
    pub index_file: &'a str,
    pub site_lang: String,
    pub highlight_style: String,
    pub version: &'static str,
}

pub struct HtmlGenerator {
    handlebars: Handlebars<'static>,
    site_title: String,
    site_lang: String,
    highlight_style: String,
    index_file: OsString,
    version: &'static str,
}

#[derive(Serialize)]
struct MarkdownVars<'a> {
    site_title: &'a str,
    site_lang: &'a str,
    version: String,
    body: String,
    title: String,
    code_js: String,
    plugin_js: String,
    doclinks: String,
    peers: String,
    breadcrumbs: String,
    folders: String,
    doclinks_len: usize,
    peers_len: usize,
    folders_len: usize,
}

#[derive(Serialize)]
struct SearchVars<'a> {
    title: String,
    site_title: &'a str,
    query: String,
    placeholder: String,
    num_results: usize,
    results: Vec<SearchResult>,
}

#[derive(Serialize)]
struct ErrorVars<'a> {
    title: String,
    site_title: &'a str,
    error_code: String,
    heading: String,
    message: String,
}

#[derive(Serialize)]
struct IndexVars<'a> {
    title: String,
    site_title: &'a str,
    path: String,
    peers: String,
    folders: String,
    breadcrumbs: String,
    peers_len: usize,
    folders_len: usize,
}

impl HtmlGenerator {
    pub fn new(
        cfg: HtmlGeneratorCfg
    ) -> Result<HtmlGenerator, ChimeraError> {
        let mut handlebars = Handlebars::new();

        handlebars.set_dev_mode(true);
        handlebars.register_templates_directory(cfg.template_root.as_path(), DirectorySourceOptions::default())?;
        let required_templates = ["markdown", "error", "search"];
        for name in required_templates {
            if !handlebars.has_template(name) {
                let template_name = format!("{name}.hbs");
                tracing::error!("Missing required template: {}{template_name}", cfg.template_root.display());
                return Err(ChimeraError::MissingMarkdownTemplate);
            }
        }

        Ok(HtmlGenerator {
            handlebars,
            site_title: cfg.site_title,
            site_lang: cfg.site_lang,
            highlight_style: cfg.highlight_style,
            index_file: OsString::from(cfg.index_file),
            version: cfg.version,
        })
    }

    pub fn gen_search(&self, query: &str, results: Vec<SearchResult>) -> Result<String, ChimeraError> {
        tracing::debug!("Got {} search results", results.len());
        let vars = SearchVars {
            title: format!("{}: Search results", self.site_title),
            site_title: self.site_title.as_str(),
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
            title: format!("{}: Search", self.site_title),
            site_title: self.site_title.as_str(),
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
        body: String,
        scraper: DocumentScraper,
        peers: PeerInfo,
    ) -> Result<String, ChimeraError> {
        tracing::debug!("Peers: {peers:?}");
        let html_content = add_anchors_to_headings(body, &scraper.doclinks, !scraper.starts_with_heading);
        let code_js = get_code_blob(&scraper, self.highlight_style.as_str());
        let plugin_js = get_plugins(&scraper);
        let title = scraper.title.unwrap_or_else(||{
            if let Some(name) = path.file_name() {
                name.to_string_lossy().into_owned()
            }
            else {
                path.to_string_lossy().into_owned()
            }
        });
        let doclinks_html = generate_anchor_html(scraper.doclinks);
        let peers_html = generate_filelink_html(&peers.files);
        let folders_html = generate_filelink_html(&peers.folders);
        let breadcrumbs = get_breadcrumbs(path, self.index_file.as_os_str());

        let vars = MarkdownVars {
            body: html_content,
            title: format!("{}: {}", self.site_title, title),
            site_title: self.site_title.as_str(),
            site_lang: self.site_lang.as_str(),
            version: self.version.to_string(),
            code_js,
            plugin_js,
            doclinks_len: doclinks_html.len(),
            peers_len: peers_html.len(),
            folders_len: folders_html.len(),
            doclinks: doclinks_html,
            peers: peers_html,
            folders: folders_html,
            breadcrumbs,
            directory: "ggg".to_string(),
        };

        let html = self.handlebars.render("markdown", &vars)?;
        tracing::debug!("Generated fresh response for {}", path.display());

        Ok(html)
    }

    pub fn gen_error(&self, error_code: &str, heading: &str, message: &str) -> Result<String, ChimeraError> {
        let vars = ErrorVars {
            title: format!("{}: Error", self.site_title),
            site_title: self.site_title.as_str(),
            error_code: error_code.to_string(),
            heading: heading.to_string(),
            message: message.to_string(),
        };
        let html = self.handlebars.render("error", &vars)?;
        Ok(html)
    }

    pub async fn gen_index(&self, path: &Path, peers: PeerInfo) -> Result<String, ChimeraError> {
        let peers_html = generate_filelink_html(&peers.files);
        let folders_html = generate_filelink_html(&peers.folders);
        let breadcrumbs = get_breadcrumbs(path, self.index_file.as_os_str());

        let path_os_str = path.iter().last().unwrap_or(path.as_os_str());
        let path_str = path_os_str.to_string_lossy().into_owned();
        let vars = IndexVars {
            title: format!("{}: {}", self.site_title, path_str),
            site_title: self.site_title.as_str(),
            path: path_str,
            peers_len: peers_html.len() + folders_html.len(),
            peers: peers_html,
            breadcrumbs,
            folders_len: folders_html.len(),
            folders: folders_html,
        };
        let html = self.handlebars.render("index", &vars)?;
        Ok(html)
    }
}

fn generate_filelink_html(doclinks: &[Doclink]) -> String {
    if doclinks.is_empty() {
        return "".to_string()
    };
    let item_prefix = "\n<li><a href=\"";
    let item_middle = "\">";
    let item_suffix = "</a>";
    let list_item_end = "</li>";
    let text_len = doclinks.iter().fold(0, |acc, link| {acc + link.anchor.len() + link.name.len()});
    let expected_size = doclinks.len() *
        (item_prefix.len() + item_middle.len() + item_suffix.len() + list_item_end.len()) +
        text_len;
    let mut html = String::with_capacity(expected_size);
    for link in doclinks.iter() {
        html.push_str(item_prefix);
        html.push_str(link.anchor.as_str());
        html.push_str(item_middle);
        html.push_str(link.name.as_str());
        html.push_str(item_suffix);
        html.push_str(list_item_end);
    }
    if html.len() != expected_size {
        tracing::warn!("Miscalculated file links size. Actual: {}, expected: {}", html.len(), expected_size);
        tracing::warn!("text_len: {text_len}");
        tracing::warn!("Docs: {doclinks:?}");
        tracing::warn!("Doclinks:({})", html);
    }
    html
}

fn generate_anchor_html(mut doclinks: Vec<Doclink>) -> String {
    if doclinks.is_empty() {
        return "".to_string()
    };
    let (num_indents, text_len) = normalize_headings(&mut doclinks);
    let list_prefix = "\n<ul>";
    let list_suffix = "\n</ul>";
    let item_prefix = "\n<li><a href=\"#";

    let item_middle = "\">";
    let item_suffix = "</a>";
    let list_item_end = "</li>";
    let expected_size = (num_indents * (list_prefix.len() + list_suffix.len())) +
        (doclinks.len() * (item_prefix.len() + item_middle.len() + item_suffix.len() + list_item_end.len())) +
        text_len;
    let mut last_level = 0;
    let mut html = String::with_capacity(expected_size);
    for link in doclinks.iter() {
        if last_level < link.level {
            html.push_str(list_prefix);
            last_level = link.level;
        }
        else {
            while last_level != link.level {
                html.push_str(list_item_end);
                html.push_str(list_suffix);
                last_level -= 1;
            }
            html.push_str(list_item_end);
        }
        html.push_str(item_prefix);
        html.push_str(link.anchor.as_str());
        html.push_str(item_middle);
        html.push_str(link.name.as_str());
        html.push_str(item_suffix);
    }
    while last_level > 0 {
        html.push_str(list_item_end);
        html.push_str(list_suffix);
        last_level -= 1;
    }
    if html.len() != expected_size {
        tracing::warn!("Miscalculated anchor links size. Actual: {}, expected: {}", html.len(), expected_size);
        tracing::warn!("num_indents: {num_indents}, text: {text_len}");
        tracing::warn!("Docs: {doclinks:?}");
        tracing::warn!("Doclinks:({})", html);
    }
    html
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

fn add_anchors_to_headings(original_html: String, links: &[Doclink], inserted_top: bool) -> String {
    let start_index = if inserted_top { 1 } else { 0 };
    let num_links = links.len();
    if num_links == start_index {
        return original_html;
    }
    let mut link_index = start_index;
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

fn get_plugins(scraper: &DocumentScraper) -> String {
    tracing::debug!("Including plugins: {:?}", scraper.plugins);
    let mut plugins = String::with_capacity(1024);
    if !scraper.plugins.is_empty() {
        plugins.push_str("<script src=\"https://ajax.googleapis.com/ajax/libs/jquery/3.7.1/jquery.min.js\"></script>\n");
    }
    for plugin in scraper.plugins.iter() {
        plugins.push_str("<script src=\"/home/script/");
        plugins.push_str(plugin.as_str());
        plugins.push_str(".js\"></script>\n");
    }
    plugins
}

fn get_code_blob(scraper: &DocumentScraper, highlight_style: &str) -> String {
    if !scraper.has_code_blocks {
        return String::new()
    }
    let lang_prefix = "    <script src=\"https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/languages/";
    let lang_suffix = ".min.js\"></script>\n";
    let lang_len = scraper.code_languages.iter().fold(0, |len, el| {
        len + el.len() + lang_prefix.len() + lang_suffix.len()
    });

    let highlight_style_prefix = "<link rel=\"stylesheet\" href=\"https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/styles/";
    let highlight_style_suffix = ".min.css\">\n";
    let highlight_js = "    <script src=\"https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/highlight.min.js\"></script>\n";
    let invoke_js = "    <script>hljs.highlightAll();</script>";
    let expected_len = highlight_style_prefix.len() + highlight_style.len() + highlight_style_suffix.len() +
        highlight_js.len() + lang_len + invoke_js.len();
    let mut buffer = String::with_capacity(expected_len);
    buffer.push_str(highlight_style_prefix);
    buffer.push_str(highlight_style);
    buffer.push_str(highlight_style_suffix);
    buffer.push_str(highlight_js);
    for lang in scraper.code_languages.iter() {
        buffer.push_str(lang_prefix);
        buffer.push_str(lang);
        buffer.push_str(lang_suffix);
    }
    buffer.push_str(invoke_js);
    if buffer.len() != expected_len {
        tracing::warn!("Miscalculated code blob size. Actual: {}, Expected: {}", buffer.len(), expected_len);
    }
    buffer
}

const HOME_PREFIX: &str = r#"<span class="home"><a href=""#;
const HOME_SUFFIX: &str = r#"">Home</a></span>"#;
const CRUMB_PREFIX: &str = r#"<span class="crumb"><a href=""#;
const CRUMB_MIDDLE: &str = r#"">"#;
const CRUMB_SUFFIX: &str = r#"</a></span>"#;
const FINAL_PREFIX: &str = r#"<span class="crumb">"#;
const FINAL_SUFFIX: &str = r#"</span>"#;

fn get_breadcrumb_name_and_url_len(parts: &[&OsStr]) -> (usize, usize, usize) {
    let mut url_len = HOME_DIR.len();
    let mut prev_url_len = url_len;
    let mut name_len = 0;
    let mut it = parts.iter().peekable();
    while let Some(p) = it.next() {
        if it.peek().is_some() {
            let p_len = urlencoding::encode(&p.to_string_lossy()).len();
            let new_url_len = prev_url_len + p_len + 1;
            url_len += new_url_len;
            prev_url_len = new_url_len;
            name_len += p.len();
        }
        else {
            name_len += parts[parts.len() - 1].len();
        }
    }
    (name_len, url_len, prev_url_len)
}

fn get_breadcrumbs_len(parts: &[&OsStr]) -> (usize, usize) {
    let (name_len, url_len, max_url_len) = get_breadcrumb_name_and_url_len(parts);
    let breadcrumb_len = (HOME_PREFIX.len() + HOME_SUFFIX.len()) +
        if parts.is_empty() {
            0
        }
        else {
            (parts.len() - 1) * (CRUMB_PREFIX.len() + CRUMB_MIDDLE.len() + CRUMB_SUFFIX.len()) +
            (FINAL_PREFIX.len() + FINAL_SUFFIX.len())
        }
        + url_len + name_len;
    (breadcrumb_len, max_url_len)
}

fn get_breadcrumbs(path: &Path, skip: &OsStr) -> String {
    let parts: Vec<&OsStr> = path.iter().filter(|el| {
        el != &skip
    }).collect();
    let (breadcrumb_len, url_len) = get_breadcrumbs_len(&parts);
    let mut breadcrumbs = String::with_capacity(breadcrumb_len);
    let mut url = String::with_capacity(url_len);
    url.push_str(HOME_DIR);
    breadcrumbs.push_str(HOME_PREFIX);
    breadcrumbs.push_str(url.as_str());
    breadcrumbs.push_str(HOME_SUFFIX);

    let mut it = parts.iter().peekable();
    while let Some(p) = it.next() {
        if it.peek().is_some() {
            url.push_str(&urlencoding::encode(&p.to_string_lossy()));
            url.push('/');
            breadcrumbs.push_str(CRUMB_PREFIX);
            breadcrumbs.push_str(url.as_str());
            breadcrumbs.push_str(CRUMB_MIDDLE);
            breadcrumbs.push_str(&p.to_string_lossy());
            breadcrumbs.push_str(CRUMB_SUFFIX);
        }
        else {
            breadcrumbs.push_str(FINAL_PREFIX);
            breadcrumbs.push_str(&p.to_string_lossy());
            breadcrumbs.push_str(FINAL_SUFFIX);
        }
    }
    if breadcrumbs.len() != breadcrumb_len {
        tracing::warn!("Miscalculated breadcrumbs size. Actual: {}, Expected: {}", breadcrumbs.len(), breadcrumb_len);
    }
    if url.len() != url_len {
        tracing::warn!("Miscalculated url size. Actual: {}, Expected: {}", url.len(), url_len);
    }
    breadcrumbs
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
        let (name_len, url_len, max_url_len) = get_breadcrumb_name_and_url_len(&parts);
        assert_eq!(name_len, 24);
        assert_eq!(url_len, 46);
        assert_eq!(max_url_len, 24);
    }

    #[test]
    fn test_breadcrumbs_2() {
        let path = PathBuf::from("Documents/Example/Recipes/pizza.md");
        let parts: Vec<&OsStr> = path.iter().collect();
        let (name_len, url_len, max_url_len) = get_breadcrumb_name_and_url_len(&parts);
        assert_eq!(name_len, 31);
        assert_eq!(url_len, 78);
        assert_eq!(max_url_len, 32);
    }

    #[test]
    fn test_breadcrumbs_3() {
        let path = PathBuf::from("index.md");
        let parts: Vec<&OsStr> = path.iter().collect();
        let (name_len, url_len, max_url_len) = get_breadcrumb_name_and_url_len(&parts);
        assert_eq!(name_len, 8);
        assert_eq!(url_len, 6);
        assert_eq!(max_url_len, 6);
    }
}
