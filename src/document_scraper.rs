use std::collections::HashSet;

use pulldown_cmark::{Event, Tag, TagEnd};
use serde::Serialize;

#[derive(Serialize)]
pub struct Doclink {
    pub anchor: String,
    pub name: String,
}

#[derive(Default)]
pub struct DocumentScraper {
    language_map: HashSet<&'static str>,
    pub doclinks: Vec<Doclink>,
    pub code_languages: Vec<&'static str>,
    pub title: Option<String>,
    pub has_code_blocks: bool,
    in_header: bool,
}

fn get_munged_anchor(anchor: &str) -> String {
    anchor.replace(' ', "-")
}

impl DocumentScraper {
    pub fn new() -> Self {
        DocumentScraper {
            language_map: HashSet::from([
                "applescript",
                "bash",
                "c",
                "cpp",
                "csharp",
                "erlang",
                "fortran",
                "go",
                "haskell",
                "html",
                "ini",
                "java",
                "js",
                "make",
                "markdown",
                "objectivec",
                "perl",
                "php",
                "python",
                "r",
                "rust",
                "sql",
                "text",
                "xml",
                "yaml",
            ]),
            doclinks: vec![Doclink { anchor: "top".to_string(), name: "Top".to_string() }],
            code_languages: Vec::new(),
            title: None,
            has_code_blocks: false,
            in_header: false,
        }
    }

    pub fn check_event(&mut self, ev: &Event) {
        tracing::trace!("{ev:?}");
        match ev {
            Event::Start(Tag::Heading{level: _, id: _, classes: _, attrs: _}) => {
                self.in_header = true;
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
            Event::Text(t) => {
                if self.in_header {
                    if self.title.is_none() {
                        self.title = Some(t.to_string());
                    }
                    else {
                        self.doclinks.push(Doclink {
                            anchor: get_munged_anchor(t.to_lowercase().as_str()),
                            name: t.to_string(),
                        });
                    }
                }
            },
            Event::End(TagEnd::Heading(_level)) => {
                self.in_header = false;
            },
            _ => {}
        }
    }
}
