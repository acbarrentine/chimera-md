use pulldown_cmark::{Event, Tag, TagEnd};

#[derive(Default)]
pub struct TitleFinder {
    pub title: Option<String>,
    in_header: bool,
}

impl TitleFinder {
    pub fn check_event( &mut self, ev: &Event) {
        tracing::debug!("{ev:?}");
        // potentially we could scan through the document here for internal links and subheads
        if self.title.is_some() {
            return;
        }
        match ev {
            Event::Start(Tag::Heading{level: _, id: _, classes: _, attrs: _}) => {
                self.in_header = true;
            },
            Event::Text(t) => {
                if self.in_header {
                    self.title = Some(t.to_string());
                }
            },
            Event::End(TagEnd::Heading(_level)) => {
                if self.in_header {
                    self.in_header = false;
                }
            },
            _ => {}
        }
    }
}
