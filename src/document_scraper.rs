use std::collections::HashSet;
use regex::Regex;
use pulldown_cmark::{Event, Tag, TagEnd};
use serde::Serialize;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Doclink {
    pub anchor: String,
    pub name: String,
}

pub struct DocumentScraper {
    language_map: HashSet<&'static str>,
    pub doclinks: Vec<Doclink>,
    pub code_languages: Vec<&'static str>,
    pub title: Option<String>,
    heading_re: Regex,
    id_re: Regex,
    heading_text: Option<String>,
    pub has_code_blocks: bool,
}

fn get_munged_anchor(anchor: &str) -> String {
    anchor.replace(' ', "-")
}

impl DocumentScraper {
    pub fn new() -> Self {
        let heading_re = Regex::new(r"<[hH]\d\s*([^<]*)>([^<]*)</[hH]\d>").unwrap();
        let id_re = Regex::new("id=\"([^\"]+)\"").unwrap();
        DocumentScraper {
            language_map: HashSet::from([
                "applescript", "bash", "c", "cpp", "csharp", "erlang", "fortran", "go", "haskell",
                "html", "ini", "java", "js", "make", "markdown", "objectivec", "perl", "php",
                "python", "r", "rust", "sql", "text", "xml", "yaml",
            ]),
            doclinks: vec![Doclink { anchor: "top".to_string(), name: "Top".to_string() }],
            code_languages: Vec::new(),
            title: None,
            heading_re,
            id_re,
            heading_text: None,
            has_code_blocks: false,
        }
    }

    pub fn check_event(&mut self, ev: &Event) {
        tracing::trace!("md-event: {ev:?}");
        match ev {
            Event::Start(Tag::Heading{level: _, id: _, classes: _, attrs: _}) => {
                self.heading_text = Some(String::with_capacity(64));
            },
            Event::Start(Tag::CodeBlock(kind)) => {
                self.has_code_blocks = true;
                if let pulldown_cmark::CodeBlockKind::Fenced(lang) = kind {
                    let lang = lang.to_ascii_lowercase();
                    if let Some(js) = self.language_map.get(lang.as_str()) {
                        self.code_languages.push(js);
                    }
                }
            }
            Event::Html(text) => {
                // <h3 id="the-middle">The middle</h3>
                if let Some(captures) = self.heading_re.captures(text) {
                    let id_text = captures.get(1);
                    let heading_match = captures.get(2);
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
                    tracing::debug!("Found doclink: {anchor} -> {heading_text}");
                    self.doclinks.push(Doclink {
                        anchor: get_munged_anchor(anchor),
                        name: heading_text.to_string(),
                    });
                }
            },
            Event::Text(t) => {
                if let Some(name) = self.heading_text.as_mut() {
                    name.push_str(t);
                }
            },
            Event::End(TagEnd::Heading(_level)) => {
                if let Some(name) = self.heading_text.take() {
                    // first heading is also the title
                    if self.title.is_none() {
                        self.title = Some(name.clone());
                    }
                    let link = Doclink {
                        anchor: get_munged_anchor(name.to_lowercase().as_str()),
                        name,
                    };
                    tracing::debug!("Doclink found: {link:?}");
                    self.doclinks.push(link);
                }
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_in_md_heading() {
        let md = "# / [Home](/index.md) / [Documents](/Documents/index.md) / [Work](index.md)";
        let mut scraper = DocumentScraper::new();
        let parser = pulldown_cmark::Parser::new(md).map(|ev| {
            scraper.check_event(&ev);
            ev
        });
        let mut html_content = String::with_capacity(md.len() * 3 / 2);
        pulldown_cmark::html::push_html(&mut html_content, parser);
        assert_eq!(scraper.doclinks.len(), 2);
        assert_eq!(scraper.doclinks[1], Doclink {
            name: "/ Home / Documents / Work".to_string(),
            anchor: "/-home-/-documents-/-work".to_string()
        });
    }

    #[test]
    fn test_heart_in_md_heading() {
        let md = "### Kisses <3!";
        let mut scraper = DocumentScraper::new();
        let parser = pulldown_cmark::Parser::new(md).map(|ev| {
            scraper.check_event(&ev);
            ev
        });
        let mut html_content = String::with_capacity(md.len() * 3 / 2);
        pulldown_cmark::html::push_html(&mut html_content, parser);
        assert_eq!(scraper.doclinks.len(), 2);
        assert_eq!(scraper.doclinks[1], Doclink {
            name: "Kisses <3!".to_string(),
            anchor: "kisses-<3!".to_string()
        });
    }

    #[test]
    fn test_first_heading_is_also_title() {
        let md = "# The title\n\nBody\n\n# Subhead\n\nBody 2";
        let mut scraper = DocumentScraper::new();
        let parser = pulldown_cmark::Parser::new(md).map(|ev| {
            scraper.check_event(&ev);
            ev
        });
        let mut html_content = String::with_capacity(md.len() * 3 / 2);
        pulldown_cmark::html::push_html(&mut html_content, parser);
        assert_eq!(scraper.doclinks.len(), 3);
        assert_eq!(scraper.doclinks[1], Doclink {
            name: "The title".to_string(),
            anchor: "the-title".to_string()
        });
        assert_eq!(scraper.doclinks[2], Doclink {
            name: "Subhead".to_string(),
            anchor: "subhead".to_string()
        });
        assert_eq!(scraper.title, Some("The title".to_string()));
    }
}