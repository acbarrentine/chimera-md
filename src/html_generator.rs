use std::path::{Path, PathBuf};
use indexmap::IndexMap;
use serde::Serialize;
use tera::Tera;

use crate::{chimera_error::ChimeraError, document_scraper::{DocumentScraper, ExternalLink, InternalLink}, file_manager::PeerInfo, full_text_index::SearchResult};

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct HtmlGeneratorCfg {
    pub template_root: PathBuf,
    pub site_title: String,
    pub site_lang: String,
    pub highlight_style: String,
    pub menu: IndexMap<String, String>,
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
    menu: Vec<MenuItem>,
}

impl HtmlGenerator {
    pub fn new(
        cfg: HtmlGeneratorCfg
    ) -> Result<HtmlGenerator, ChimeraError> {
        let template_glob = cfg.template_root.join("*.html");
        let template_glob = match template_glob.to_str() {
            Some(glob) => glob,
            None => {
                return Err(ChimeraError::IOError("Could not get template dir glob".to_string()));
            },
        };
        //let template_glob = format!("{}/*.html", cfg.template_root.display());
        let mut tera = Tera::new(template_glob)?;
        tera.autoescape_on(vec![]);
        let required_templates = ["markdown.html", "error.html", "search.html"];
        let found_templates = Vec::from_iter(tera.get_template_names());
        for name in required_templates {
            if !found_templates.contains(&name) {
                let path_to_template = cfg.template_root.join(name);
                tracing::error!("Missing required template: {}", path_to_template.display());
                return Err(ChimeraError::MissingMarkdownTemplate);
            }
        }

        Ok(HtmlGenerator {
            tera,
            site_title: cfg.site_title,
            site_lang: cfg.site_lang,
            highlight_style: cfg.highlight_style,
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
        let breadcrumbs = get_breadcrumbs(path);
        let title = format!("{}: {}", self.site_title, title);

        let mut vars = self.get_vars(title.as_str(), scraper.has_code_blocks);
        vars.insert("body", html_content.as_str());
        vars.insert("doclinks", &scraper.internal_links);
        vars.insert("peers", &peers);
        vars.insert("code_languages", &scraper.code_languages);
        vars.insert("breadcrumbs", &breadcrumbs);
        vars.insert("url", &path.to_string_lossy());

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
        let breadcrumbs = get_breadcrumbs(path);
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

fn get_breadcrumbs(path: &Path) -> Vec<ExternalLink> {
    let mut crumbs = Vec::with_capacity(8);
    let mut url = String::with_capacity(path.as_os_str().len() * 3 / 2);
    url.push('/');
    for p in path.iter() {
        url.push_str(&urlencoding::encode(&p.to_string_lossy()));
        url.push('/');
        let mut name = p.to_string_lossy().into_owned();
        if name.eq("home") {
            name.replace_range(0..1, "H");
        }
        crumbs.push(ExternalLink::new(url.clone(), name));
    }
    crumbs
}
