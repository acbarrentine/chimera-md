use std::{collections::HashSet, ops::Range};
use regex::Regex;
use pulldown_cmark::{Event, Tag, TagEnd};
use serde::Serialize;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Doclink {
    pub anchor: String,
    pub name: String,
    pub level: u8,
}

impl Doclink {
    pub fn new(anchor: String, name: String, level: u8) -> Self {
        Doclink {
            anchor,
            name,
            level,
        }
    }
}

#[derive(Clone)]
pub struct DocumentScraper {
    language_map: HashSet<&'static str>,
    pub doclinks: Vec<Doclink>,
    pub code_languages: Vec<&'static str>,
    pub plugins: Vec<String>,
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
            language_map: HashSet::from([
                "applescript", "bash", "c", "cpp", "csharp", "erlang", "fortran", "go", "haskell",
                "html", "ini", "java", "js", "make", "markdown", "objectivec", "perl", "php",
                "python", "r", "rust", "sql", "text", "xml", "yaml",
            ]),
            doclinks: Vec::new(),
            code_languages: Vec::new(),
            plugins: Vec::new(),
            title: None,
            heading_re,
            id_re,
            text_collector: None,
            has_code_blocks: false,
            starts_with_heading: false,
            has_readable_text: false,
        }
    }

    pub fn check_event(&mut self, ev: &Event, range: Range<usize>) {
        tracing::debug!("md-event: {ev:?} - {range:?}");
        match ev {
            Event::Start(tag) => {
                match tag {
                    Tag::MetadataBlock(_) => {
                        self.text_collector = Some(String::with_capacity(128));
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
                            if let Some(js) = self.language_map.get(lang.as_str()) {
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
                    self.doclinks.push(Doclink::new(anchor.to_string(), heading_text.to_string(), level));
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
                            let link = Doclink::new(
                                name.clone(), 
                                name, *level as u8);
                            tracing::debug!("Doclink found: {link:?}");
                            self.doclinks.push(link);
                        }
                    },
                    TagEnd::MetadataBlock(_) => {
                        if let Some(metadata) = self.text_collector.take() {
                            let mut it = metadata.split(':');
                            while let Some(chunk) = it.next() {
                                if chunk.eq_ignore_ascii_case("plugin") {
                                    if let Some(plugin) = it.next() {
                                        let plugin = plugin.trim();
                                        tracing::debug!("Plugin: {plugin:?}");
                                        self.plugins.push(plugin.to_string());
                                    }
                                    else {
                                        break;
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
        scraper.doclinks.insert(0, Doclink::new("top".to_string(), "Top".to_string(), 1));
    }
    (html_content, scraper)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_in_md_heading() {
        let md = "# / [Home](/index.md) / [Documents](/Documents/index.md) / [Work](index.md)";
        let (_html_content, scraper) = parse_markdown(md);
        assert_eq!(scraper.doclinks.len(), 1);
        assert_eq!(scraper.doclinks[0], Doclink::new(
            "/-home-/-documents-/-work".to_string(),
            "/ Home / Documents / Work".to_string(),
            1));
    }

    #[test]
    fn test_heart_in_md_heading() {
        let md = "### Kisses <3!";
        let (_html_content, scraper) = parse_markdown(md);
        assert_eq!(scraper.doclinks.len(), 1);
        assert_eq!(scraper.doclinks[0], Doclink::new(
            "kisses-<3!".to_string(),
            "Kisses <3!".to_string(),
            3));
    }

    #[test]
    fn test_first_heading_is_also_title() {
        let md = "# The title\n\nBody\n\n## Subhead\n\nBody 2";
        let (_html_content, scraper) = parse_markdown(md);
        assert_eq!(scraper.doclinks.len(), 2);
        assert_eq!(scraper.doclinks[0], Doclink::new(
            "the-title".to_string(),
            "The title".to_string(),
            1));
        assert_eq!(scraper.doclinks[1], Doclink::new(
            "subhead".to_string(),
            "Subhead".to_string(),
            2));
        assert_eq!(scraper.title, Some("The title".to_string()));
    }
}
