use std::{collections::HashSet, ffi::{OsStr, OsString}, path::{Path, PathBuf}};
use indexmap::IndexMap;
use serde::Serialize;
use tera::Tera;

use crate::chimera_error::ChimeraError;
use crate::document_scraper::{DocumentScraper, ExternalLink, InternalLink};
use crate::file_manager::{FileManager, PeerInfo};
use crate::full_text_index::SearchResult;
use crate::HOME_DIR;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct HtmlGeneratorCfg<'a> {
    pub user_template_root: PathBuf,
    pub internal_template_root: PathBuf,
    pub site_title: &'a str,
    pub index_file: &'a str,
    pub site_lang: &'a str,
    pub highlight_style: &'a str,
    pub menu: IndexMap<String, String>,
    pub file_manager: &'a FileManager,
}

#[derive (Debug, Serialize)]
struct MenuItem {
    title: String,
    target: String,
}

pub struct HtmlGenerator {
    tera: Tera,
    site_title: String,
    site_lang: String,
    highlight_style: String,
    index_file: OsString,
    menu: Vec<MenuItem>,
}

impl HtmlGenerator {
    pub fn new(
        cfg: HtmlGeneratorCfg
    ) -> Result<HtmlGenerator, ChimeraError> {
        let mut tera = Tera::default();
        tera.autoescape_on(vec![]);

        let html_ext = OsString::from("html");
        let mut found = HashSet::new();
        for entry in cfg.file_manager.find_files(&cfg.user_template_root, html_ext.as_os_str()).into_iter() {
            let fname = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            tera.add_template_file(path, Some(fname.as_str()))?;
            found.insert(fname);
        }
        for entry in cfg.file_manager.find_files(&cfg.internal_template_root, html_ext.as_os_str()).into_iter() {
            let fname = entry.file_name().to_string_lossy().into_owned();
            if !found.contains(fname.as_str()) {
                let path = entry.path();
                tera.add_template_file(path, Some(fname.as_str()))?;
                found.insert(fname);
            }
        }
        let names: Vec<_> = tera.get_template_names().collect();
        tracing::info!("Templates: {names:?}");

        Ok(HtmlGenerator {
            tera,
            site_title: cfg.site_title.to_owned(),
            site_lang: cfg.site_lang.to_owned(),
            highlight_style: cfg.highlight_style.to_owned(),
            index_file: OsString::from(cfg.index_file),
            menu: cfg.menu.into_iter().map(|(title, target)| {
                MenuItem {
                    title,
                    target
                }
            }).collect(),
        })
    }

    fn get_vars(&self, title: &str, has_code: bool) -> tera::Context {
        let mut vars = tera::Context::new();
        vars.insert("title", title);
        vars.insert("site_title", self.site_title.as_str());
        vars.insert("site_lang", self.site_lang.as_str());
        vars.insert("highlight_style", self.highlight_style.as_str());
        vars.insert("has_code", &has_code);
        vars.insert("version", VERSION);
        vars.insert("menu", &self.menu);
        vars
    }

    pub fn gen_search(&self, query: &str, results: Vec<SearchResult>) -> Result<String, ChimeraError> {
        tracing::debug!("Got {} search results", results.len());
        let title = format!("{}: Search results", self.site_title);
        let mut vars = self.get_vars(title.as_str(), false);
        vars.insert("query", query);
        vars.insert("placeholder", query);
        if !results.is_empty() {
            vars.insert("results", &results);
        }
        Ok(self.tera.render("search.html", &vars)?)
    }

    pub fn gen_search_blank(&self) -> Result<String, ChimeraError> {
        tracing::debug!("No query, generating blank search page");
        let title = format!("{}: Search results", self.site_title);
        let mut vars = self.get_vars(title.as_str(), false);
        vars.insert("query", "");
        vars.insert("placeholder", "Search...");
        Ok(self.tera.render("search.html", &vars)?)
    }

    pub fn gen_markdown(
        &self,
        path: &std::path::Path,
        body: String,
        scraper: DocumentScraper,
        peers: Option<PeerInfo>,
    ) -> Result<String, ChimeraError> {
        let html_content = self.add_anchors_to_headings(body, &scraper.internal_links, !scraper.starts_with_heading);
        let template = scraper.get_template();
        let title = scraper.title.as_ref().cloned().unwrap_or_else(|| {
            match path.file_name() {
                Some(name) => name,
                None => path.as_os_str(),
            }.to_string_lossy().into_owned()
        });
        let breadcrumbs = get_breadcrumbs(path, self.index_file.as_os_str());
        let title = format!("{}: {}", self.site_title, title);

        let mut vars = self.get_vars(title.as_str(), scraper.has_code_blocks);
        vars.insert("body", html_content.as_str());
        vars.insert("doclinks", &scraper.internal_links);
        vars.insert("peers", &peers);
        vars.insert("code_languages", &scraper.code_languages);
        vars.insert("breadcrumbs", &breadcrumbs);
        vars.insert("url", format!("{HOME_DIR}/{}", &path.to_string_lossy()).as_str());

        for (key, value) in &scraper.metadata {
            vars.insert(key, value);
        }

        let html = self.tera.render(template, &vars)?;
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

    pub async fn gen_index(&self, path: &Path, peers: Option<PeerInfo>) -> Result<String, ChimeraError> {
        let breadcrumbs = get_breadcrumbs(path, self.index_file.as_os_str());
        let path_os_str = path.iter().last().unwrap_or(path.as_os_str());
        let path_str = path_os_str.to_string_lossy().to_string();
        let title = format!("{}: {}", self.site_title, path_str);
        let mut vars = self.get_vars(title.as_str(), false);
        vars.insert("path", path_str.as_str());
        vars.insert("breadcrumbs", &breadcrumbs);
        let doclinks = vec![InternalLink::new("contents".to_string(), "Contents".to_string(), 2)];
        vars.insert("doclinks", &doclinks);
        vars.insert("peers", &peers);
        vars.insert("body", "");
        let html = self.tera.render("index.html", &vars)?;
        Ok(html)
    }

    fn add_anchors_to_headings(&self, original_html: String, links: &[InternalLink], inserted_top: bool) -> String {
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
}

fn get_breadcrumbs(path: &Path, skip: &OsStr) -> Vec<ExternalLink> {
    let parts: Vec<&OsStr> = path.iter().filter(|el| {
        el != &skip
    }).collect();
    let mut crumbs = Vec::with_capacity(parts.len());
    let mut url = String::with_capacity(path.as_os_str().len() * 3 / 2);
    url.push_str(format!("{HOME_DIR}/").as_str());

    crumbs.push(ExternalLink::new(url.clone(), "Home".to_string()));

    for p in parts {
        url.push_str(&urlencoding::encode(&p.to_string_lossy()));
        url.push('/');
        crumbs.push(ExternalLink::new(url.clone(), p.to_string_lossy().into_owned()));
    }
    crumbs
}
