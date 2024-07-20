use std::{collections::{BTreeMap, HashMap}, ffi::{OsStr, OsString}, path::{Path, PathBuf}};
use serde::Serialize;
use tera::Tera;

use crate::{chimera_error::ChimeraError, document_scraper::{DocumentScraper, ExternalLink, InternalLink}, file_manager::FolderInfo, full_text_index::SearchResult, HOME_DIR};

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
        peers: BTreeMap<String, FolderInfo>,
    ) -> Result<String, ChimeraError> {
        tracing::debug!("Peers: {peers:?}");
        let html_content = self.add_anchors_to_headings(body, &scraper.internal_links, !scraper.starts_with_heading);
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
        vars.insert("doclinks", &scraper.internal_links);
        vars.insert("peers", &peers);
        if !scraper.plugins.is_empty() {
            vars.insert("plugins", &scraper.plugins);
        }
        if !scraper.code_languages.is_empty() {
            vars.insert("code_languages", &scraper.code_languages);
        }
        vars.insert("breadcrumbs", &breadcrumbs);

        // #[derive(Serialize)]
        // struct FolderInfo {
        //     files: Vec<String>,
        //     folders: Vec<String>,
        // }

        // let root = FolderInfo {
        //     files: vec!["Index".to_string()],
        //     folders: vec!["aaa".to_string(), "bbb".to_string(), "ccc".to_string()],
        // };
        // let aaa = FolderInfo {
        //     files: vec!["File 1".to_string(), "File 2".to_string()],
        //     folders: vec![],
        // };
        // let bbb = FolderInfo {
        //     files: vec!["Index".to_string()],
        //     folders: vec!["Subfolder".to_string()],
        // };
        // let ccc = FolderInfo {
        //     files: vec!["File 3".to_string(), "File 4".to_string()],
        //     folders: vec![],
        // };
        // let test_map = HashMap::from([
        //     ("root".to_string(), root),
        //     ("aaa".to_string(), aaa),
        //     ("bbb".to_string(), bbb),
        //     ("ccc".to_string(), ccc),
        // ]);
        // vars.insert("test_map", &test_map);

        let template = scraper.template.unwrap_or("markdown.html".to_string());
        let html = self.tera.render(template.as_str(), &vars)?;
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

    pub async fn gen_index(&self, path: &Path, peers: BTreeMap<String, FolderInfo>) -> Result<String, ChimeraError> {
        let breadcrumbs = get_breadcrumbs(path, self.index_file.as_os_str());
        let path_os_str = path.iter().last().unwrap_or(path.as_os_str());
        let path_str = path_os_str.to_string_lossy().to_string();
        let title = format!("{}: {}", self.site_title, path_str);
        let mut vars = self.get_vars(title.as_str(), false);
        vars.insert("path", path_str.as_str());
        vars.insert("breadcrumbs", &breadcrumbs);
        vars.insert("peers", &peers);
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
    url.push_str(HOME_DIR);

    crumbs.push(ExternalLink::new(url.clone(), "Home".to_string()));

    for p in parts {
        url.push_str(&urlencoding::encode(&p.to_string_lossy()));
        url.push('/');
        crumbs.push(ExternalLink::new(url.clone(), p.to_string_lossy().into_owned()));
    }
    crumbs
}
