use pulldown_cmark::{Event, LinkType, Tag, TagEnd};
use serde::Serialize;

#[derive(Serialize)]
pub struct Doclink {
    anchor: String,
    name: String,
}

#[derive(Default)]
pub struct TitleFinder {
    pub doclinks: Vec<Doclink>,
    pub title: Option<String>,
    doclink_url: Option<String>,
    in_header: bool,
    in_link: bool,
}

impl TitleFinder {
    pub fn new() -> Self {
        TitleFinder {
            doclinks: vec![Doclink { anchor: "#top".to_string(), name: "Top".to_string() }],
            title: None,
            doclink_url: None,
            in_header: false,
            in_link: false,
        }
    }

    pub fn check_event(&mut self, ev: &Event) {
        tracing::debug!("{ev:?}");
        // potentially we could scan through the document here for internal links and subheads
        match ev {
            Event::Start(Tag::Heading{level: _, id: _, classes: _, attrs: _}) => {
                self.in_header = true;
            },
            Event::Start(Tag::Link { link_type, dest_url, title: _, id: _ }) => {
                if *link_type == LinkType::Inline && dest_url.starts_with('#') {
                    self.in_link = true;
                    self.doclink_url = Some(dest_url.to_string());
                }
            }
            Event::Text(t) => {
                if self.in_header {
                    if self.title.is_none() {
                        self.title = Some(t.to_string());
                    }
                }
                else if self.in_link && self.doclink_url.is_some() {
                    self.doclinks.push(Doclink {
                        anchor: self.doclink_url.take().unwrap(),
                        name: t.to_string(),
                    })
                }
                // it would be nice to doclink it, but I don't know how right now
                // the html generator isn't going to put an anchor on it
            },
            Event::End(TagEnd::Heading(_level)) => {
                self.in_header = false;
            },
            Event::End(TagEnd::Link) => {
                self.in_link = false;
            }
            _ => {}
        }
    }
}
