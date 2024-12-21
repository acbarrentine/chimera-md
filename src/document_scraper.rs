use std::{cmp::Ordering, collections::{HashMap, HashSet}, ops::Range};
use lazy_static::lazy_static;
use regex::Regex;
use pulldown_cmark::{Event, Tag, TagEnd};
use serde::Serialize;
use slugify::slugify;
use yaml_rust2::YamlLoader;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct InternalLink {
    pub anchor: String,
    pub name: String,
    pub level: u8,
}

impl InternalLink {
    pub fn new(anchor: String, name: String, level: u8) -> Self {
        InternalLink {
            anchor,
            name,
            level,
        }
    }
}

#[derive(Serialize, Debug)]
pub struct ExternalLink {
    pub url: String,
    pub name: String,
}

impl ExternalLink {
    pub fn new(url: String, name: String) -> Self {
        ExternalLink {
            url,
            name,
        }
    }
}

lazy_static! {
    static ref CODE_LANGUAGES: HashSet<&'static str> = HashSet::from([
        "applescript", "bash", "c", "cpp", "csharp", "erlang", "fortran", "go", "haskell",
        "html", "ini", "java", "js", "make", "markdown", "objectivec", "perl", "php",
        "python", "r", "rust", "sql", "text", "xml", "yaml",
    ]);
}

#[derive(Clone)]
pub struct DocumentScraper {
    pub internal_links: Vec<InternalLink>,
    pub code_languages: Vec<&'static str>,
    pub metadata: HashMap<String, String>,
    pub title: Option<String>,
    heading_re: Regex,
    id_re: Regex,
    text_collector: Option<String>,
    pub has_code_blocks: bool,
    pub starts_with_heading: bool,
    has_readable_text: bool,
}

impl DocumentScraper {
    pub fn new() -> Self {
        let heading_re = Regex::new(r"<[hH](\d)\s*([^<]*)>([^<]*)</[hH]\d>").unwrap();
        let id_re = Regex::new("id=\"([^\"]+)\"").unwrap();
        DocumentScraper {
            internal_links: Vec::new(),
            code_languages: Vec::new(),
            metadata: HashMap::new(),
            title: None,
            heading_re,
            id_re,
            text_collector: None,
            has_code_blocks: false,
            starts_with_heading: false,
            has_readable_text: false,
        }
    }

    pub fn get_template(&self) -> &str {
        self.metadata.get("template").map_or("markdown.html", |v| {v.as_str()})
    }

    pub fn check_event(&mut self, ev: &Event, range: Range<usize>) {
        tracing::trace!("md-event: {ev:?} - {range:?}");
        match ev {
            Event::Start(tag) => {
                match tag {
                    Tag::MetadataBlock(_) => {
                        self.text_collector = Some(String::with_capacity(1024));
                    },
                    Tag::Heading { level: _, id: _, classes: _, attrs: _ } => {
                        if !self.has_readable_text {
                            self.starts_with_heading = true;
                            self.has_readable_text = true;
                        }
                        self.text_collector = Some(String::with_capacity(64));
                    },
                    Tag::CodeBlock(kind) => {
                        self.has_code_blocks = true;
                        if let pulldown_cmark::CodeBlockKind::Fenced(lang) = kind {
                            let lang = lang.to_ascii_lowercase();
                            if let Some(js) = CODE_LANGUAGES.get(lang.as_str()) {
                                self.code_languages.push(js);
                            }
                        }
                    },
                    _ => {
                        self.has_readable_text = true;
                    }
                }
            },
            Event::Html(text) => {
                // <h3 id="the-middle">The middle</h3>
                if let Some(captures) = self.heading_re.captures(text) {
                    let level = captures.get(1);
                    let id_text = captures.get(2);
                    let heading_match = captures.get(3);
                    let Some(heading_match) = heading_match else {
                        return;
                    };
                    let heading_text = heading_match.as_str();
                    let anchor = match id_text {
                        Some(id_text) => {
                            // id="the-middle"
                            tracing::debug!("id_text: {}", id_text.as_str());
                            if let Some(id_captures) = self.id_re.captures(id_text.as_str()) {
                                match id_captures.get(1) {
                                    Some(id) => id.as_str(),
                                    None => return,
                                }
                            }
                            else {
                                tracing::debug!("No id found for heading");
                                return
                            }
                        },
                        None => {
                            heading_text
                        }
                    };
                    let level = match level {
                        Some(level_text) => {
                            level_text.as_str().parse::<u8>().unwrap()
                        },
                        None => {
                            1_u8
                        }
                    };
                    tracing::debug!("Found doclink: {anchor} -> {heading_text}");
                    self.internal_links.push(
                        InternalLink::new(
                            anchor.to_string(),
                            heading_text.to_string(), 
                            level
                        )
                    );
                }
            },
            Event::Text(t) => {
                if let Some(name) = self.text_collector.as_mut() {
                    name.push_str(t);
                }
            },
            Event::End(tag) => {
                match tag {
                    TagEnd::Heading(level) => {
                        if let Some(name) = self.text_collector.take() {
                            // first heading is also the title
                            if self.title.is_none() {
                                self.title = Some(name.clone());
                            }
                            let link = InternalLink::new(
                                slugify!(name.as_str()),
                                name, *level as u8);
                            tracing::debug!("Doclink found: {link:?}");
                            self.internal_links.push(link);
                        }
                    },
                    TagEnd::MetadataBlock(_) => {
                        if let Some(metadata) = self.text_collector.take() {
                            if let Ok(docs) = YamlLoader::load_from_str(metadata.as_str()) {
                                for doc in docs {
                                    match doc {
                                        yaml_rust2::Yaml::Real(_) => todo!(),
                                        yaml_rust2::Yaml::Integer(_) => todo!(),
                                        yaml_rust2::Yaml::String(_) => todo!(),
                                        yaml_rust2::Yaml::Boolean(_) => todo!(),
                                        yaml_rust2::Yaml::Array(vec) => {
                                            tracing::debug!("Vec: {vec:?}");
                                        },
                                        yaml_rust2::Yaml::Hash(linked_hash_map) => {
                                            //tracing::debug!("Hash: {linked_hash_map:?}");
                                            for (key,value) in linked_hash_map {
                                                let key = key.as_str().unwrap();
                                                let value = value.as_str().unwrap();
                                                tracing::debug!("Adding metadata var: {key} = {value}");
                                                self.metadata.insert(key.to_string(), value.to_string());
                                            }
                                        },
                                        yaml_rust2::Yaml::Alias(_) => todo!(),
                                        yaml_rust2::Yaml::Null => todo!(),
                                        yaml_rust2::Yaml::BadValue => todo!(),
                                    }
                                }
                            }
                        }
                    },
                    _ => {
                    }
                }
            },
            _ => {}
        }
    }

    // The indenting scheme requires that we not grow more than 1 step at a time
    // Unfortunately, because this depends on user data, we can easily be asked
    // to process an invalid setup. Eg: <h1> directly to <h3>
    // Outdents don't have the same problem
    // Renumber the link list so we don't violate that assumption
    fn normalize_headings(&mut self) {
        let mut last_used_level = 0;
        let mut last_seen_level = 0;
        for link in self.internal_links.iter_mut() {
            match link.level.cmp(&last_seen_level) {
                Ordering::Greater => {
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
        }
    }
}

pub fn parse_markdown(md: &str) -> (String, DocumentScraper) {
    let mut scraper = DocumentScraper::new();
    let parser = pulldown_cmark::Parser::new_ext(
        md, pulldown_cmark::Options::ENABLE_TABLES |
        pulldown_cmark::Options::ENABLE_SMART_PUNCTUATION |
        pulldown_cmark::Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
    ).into_offset_iter().map(|(ev, range)| {
        scraper.check_event(&ev, range);
        ev
    });
    let mut html_content = String::with_capacity(md.len() * 3 / 2);
    pulldown_cmark::html::push_html(&mut html_content, parser);
    if !scraper.starts_with_heading {
        scraper.internal_links.insert(0, InternalLink::new("top".to_string(), "Top".to_string(), 1));
    }
    if scraper.get_template().eq("index.html") {
        scraper.internal_links.push(InternalLink::new("contents".to_string(), "Contents".to_string(), 2));
    }
    scraper.normalize_headings();
    (html_content, scraper)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_in_md_heading() {
        let md = "# / [Home](/index.md) / [Documents](/Documents/index.md) / [Work](index.md)";
        let (_html_content, scraper) = parse_markdown(md);
        assert_eq!(scraper.internal_links.len(), 1);
        assert_eq!(scraper.internal_links[0], InternalLink::new(
            "home-documents-work".to_string(),
            "/ Home / Documents / Work".to_string(),
            1
        ));
    }

    #[test]
    fn test_heart_in_md_heading() {
        let md = "### Kisses <3!";
        let (_html_content, scraper) = parse_markdown(md);
        assert_eq!(scraper.internal_links.len(), 1);
        assert_eq!(scraper.internal_links[0], InternalLink::new(
            "kisses-3".to_string(),
            "Kisses <3!".to_string(),
            1
        ));
    }

    #[test]
    fn test_first_heading_is_also_title() {
        let md = "# The title\n\nBody\n\n## Subhead\n\nBody 2";
        let (_html_content, scraper) = parse_markdown(md);
        assert_eq!(scraper.internal_links.len(), 2);
        assert_eq!(scraper.internal_links[0], InternalLink::new(
            "the-title".to_string(),
            "The title".to_string(),
            1));
        assert_eq!(scraper.internal_links[1], InternalLink::new(
            "subhead".to_string(),
            "Subhead".to_string(),
            2
        ));
        assert_eq!(scraper.title, Some("The title".to_string()));
    }

    #[test]
    fn test_metadata_with_multiple_lines() {
        let md = 
"---
template: index.html
title: Index
url: https://my.site.com
image: /media/fancy.jpg
type: website
---";
        let (_html_content, scraper) = parse_markdown(md);
        assert_eq!(scraper.metadata.len(), 5);
        assert_eq!(scraper.metadata.get("template"), Some(&String::from("index.html")));
        assert_eq!(scraper.metadata.get("title"), Some(&String::from("Index")));
        assert_eq!(scraper.metadata.get("image"), Some(&String::from("/media/fancy.jpg")));
        assert_eq!(scraper.metadata.get("type"), Some(&String::from("website")));
        assert_eq!(scraper.metadata.get("url"), Some(&String::from("https://my.site.com")));
    }

    #[test]
    fn test_metadata_with_nested_struct() {
        let _md = 
"---
template: index.html
og:
  - title: Index
  - url: https://my.site.com
  - image: /media/fancy.jpg
  - type: website
---";
    }
}
