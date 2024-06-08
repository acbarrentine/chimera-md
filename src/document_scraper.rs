use pulldown_cmark::{Event, Tag, TagEnd};
use serde::Serialize;

#[derive(Serialize)]
pub struct Doclink {
    pub anchor: String,
    pub name: String,
}

#[derive(Default)]
pub struct DocumentScraper {
    pub doclinks: Vec<Doclink>,
    pub title: Option<String>,
    in_header: bool,
}

fn get_munged_anchor(anchor: &str) -> String {
    anchor.replace(' ', "-")
}

impl DocumentScraper {
    pub fn new() -> Self {
        DocumentScraper {
            doclinks: vec![Doclink { anchor: "top".to_string(), name: "Top".to_string() }],
            title: None,
            in_header: false,
        }
    }

    pub fn check_event(&mut self, ev: &Event) {
        // tracing::debug!("{ev:?}");
        match ev {
            Event::Start(Tag::Heading{level: _, id: _, classes: _, attrs: _}) => {
                self.in_header = true;
            },
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
