use std::{ffi::{OsStr, OsString}, path::{Path, PathBuf}};
use tera::Tera;

use crate::{chimera_error::ChimeraError, document_scraper::{Doclink, DocumentScraper}, file_manager::PeerInfo, full_text_index::SearchResult, HOME_DIR
};

pub struct HtmlGeneratorCfg<'a> {
    pub template_root: &'a str,
    pub site_title: String,
    pub index_file: &'a str,
    pub site_lang: String,
    pub highlight_style: String,
    pub version: &'static str,
}

pub struct HtmlGenerator {
    tera: Tera,
    site_title: String,
    site_lang: String,
    highlight_style: String,
    index_file: OsString,
    version: &'static str,
}

impl HtmlGenerator {
    pub fn new(
        cfg: HtmlGeneratorCfg
    ) -> Result<HtmlGenerator, ChimeraError> {
        let template_glob = format!("{}/*.html", cfg.template_root);
        let mut tera = Tera::new(template_glob.as_str())?;
        tera.autoescape_on(vec![]);
        let required_templates = ["markdown.html", "error.html", "search.html"];
        let found_templates = Vec::from_iter(tera.get_template_names());
        for name in required_templates {
            if !found_templates.contains(&name) {
                let path_to_template = PathBuf::from(cfg.template_root).join(name);
                tracing::error!("Missing required template: {}", path_to_template.display());
                return Err(ChimeraError::MissingMarkdownTemplate);
            }
        }

        Ok(HtmlGenerator {
            tera,
            site_title: cfg.site_title,
            site_lang: cfg.site_lang,
            highlight_style: cfg.highlight_style,
            index_file: OsString::from(cfg.index_file),
            version: cfg.version,
        })
    }

    fn get_vars(&self, title: &str, has_code: bool) -> tera::Context {
        let mut vars = tera::Context::new();
        vars.insert("title", title);
        vars.insert("site_title", self.site_title.as_str());
        vars.insert("site_lang", self.site_lang.as_str());
        vars.insert("version", self.version);
        vars.insert("highlight_style", self.highlight_style.as_str());
        vars.insert("has_code", &has_code);
        vars
    }

    pub fn gen_search(&self, query: &str, results: Vec<SearchResult>) -> Result<String, ChimeraError> {
        tracing::debug!("Got {} search results", results.len());
        let title = format!("{}: Search results", self.site_title);
        let mut vars = self.get_vars(title.as_str(), false);
        vars.insert("query", query);
        vars.insert("placeholder", query);
        vars.insert("results", &results);
        Ok(self.tera.render("search.html", &vars)?)
    }

    pub fn gen_search_blank(&self) -> Result<String, ChimeraError> {
        tracing::debug!("No query, generating blank search page");
        let title = format!("{}: Search results", self.site_title);
        let mut vars = self.get_vars(title.as_str(), false);
        vars.insert("query", "");
        vars.insert("placeholder", "Search...");
        let results: Vec<&str> = Vec::new();
        vars.insert("results", &results);
        Ok(self.tera.render("search.html", &vars)?)
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
        let title = scraper.title.unwrap_or_else(||{
            if let Some(name) = path.file_name() {
                name.to_string_lossy().into_owned()
            }
            else {
                path.to_string_lossy().into_owned()
            }
        });
        let breadcrumbs = get_breadcrumbs(path, self.index_file.as_os_str());
        let title = format!("{}: {}", self.site_title, title);

        let mut vars = self.get_vars(title.as_str(), scraper.has_code_blocks);
        vars.insert("body", html_content.as_str());
        vars.insert("doclinks", &scraper.doclinks);
        if !peers.files.is_empty() {
            vars.insert("peer_files", &peers.files);
        }
        if !peers.folders.is_empty() {
            vars.insert("peer_folders", &peers.folders);
        }
        if !scraper.plugins.is_empty() {
            vars.insert("plugins", &scraper.plugins);
        }
        if !scraper.code_languages.is_empty() {
            vars.insert("code_languages", &scraper.code_languages);
        }
        vars.insert("breadcrumbs", breadcrumbs.as_str());

        let html = self.tera.render("markdown.html", &vars)?;
        tracing::debug!("Generated fresh response for {}", path.display());

        Ok(html)
    }

    pub fn gen_error(&self, error_code: &str, heading: &str, message: &str) -> Result<String, ChimeraError> {
        let title = format!("{}: Error", self.site_title);

        let mut vars = self.get_vars(title.as_str(), false);
        vars.insert("error_code", error_code);
        vars.insert("heading", heading);
        vars.insert("message", message);

        let html = self.tera.render("error.html", &vars)?;
        Ok(html)
    }

    pub async fn gen_index(&self, path: &Path, peers: PeerInfo) -> Result<String, ChimeraError> {
        let breadcrumbs = get_breadcrumbs(path, self.index_file.as_os_str());
        let path_os_str = path.iter().last().unwrap_or(path.as_os_str());
        let path_str = path_os_str.to_string_lossy().to_string();
        let title = format!("{}: {}", self.site_title, path_str);
        let mut vars = self.get_vars(title.as_str(), false);
        vars.insert("path", path_str.as_str());
        vars.insert("breadcrumbs", breadcrumbs.as_str());
        if !peers.files.is_empty() {
            vars.insert("peer_files", &peers.files);
        }
        if !peers.folders.is_empty() {
            vars.insert("peer_folders", &peers.folders);
        }
        let html = self.tera.render("index.html", &vars)?;
        Ok(html)
    }
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
