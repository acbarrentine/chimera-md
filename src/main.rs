use std::{collections::BTreeMap, sync::Arc};

use axum::{
//    debug_handler,
    extract::State, http::{HeaderMap, Request}, response::{Html, IntoResponse}, routing::get, Router
};
use pulldown_cmark::{Event, Options, Tag, TagEnd};
use tokio::sync::RwLock;
use tower_http::services::ServeFile;
use handlebars::Handlebars;

struct AppState {
    handlebars: Handlebars<'static>,
}

impl AppState {
    pub fn new() -> Result<Self, String> {
        let mut handlebars = Handlebars::new();
        handlebars.set_dev_mode(true);
        if let Err(e) = handlebars.register_template_file("markdown", "templates/markdown.html") {
            return Err(format!("Failed to get markdown html template: {e}"));
        }
        Ok(Self {
            handlebars,
        })
    }
}

type AppStateType = Arc<RwLock<AppState>>;

#[tokio::main]
async fn main() -> Result<(), String> {
    let state = match AppState::new() {
        Ok(state) => state,
        Err(e) => {
            eprintln!("Failed establishing app state: {e}");
            return Err(e);
        }
    };
    let app = Router::new()
        .route("/", get(serve_index))
        .route("/*path", get(serve_file))
        .with_state(Arc::new(RwLock::new(state)))
        ;

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
    Ok(())
}

//#[debug_handler]
async fn serve_index(
    State(_app_state): State<AppStateType>,
    _headers: HeaderMap
) -> impl IntoResponse {
    let body = "<h1>Chimera root</h1>";
    Html(body)
}

#[derive(Default)]
struct TitleFinder {
    title: Option<String>,
    in_header: bool,
}

impl TitleFinder {
    fn check_event( &mut self, ev: &Event) {
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

//#[debug_handler]
async fn serve_file(
    State(app_state): State<AppStateType>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap
) -> impl IntoResponse {
    if let Some((_, ext)) = path.rsplit_once('.') {
        if ext.eq_ignore_ascii_case("md") {
            match tokio::fs::read_to_string(path).await {
                Ok(md_content) => {
                    let mut title_finder = TitleFinder::default();
                    let parser = pulldown_cmark::Parser::new_ext(
                        md_content.as_str(), Options::ENABLE_TABLES
                    ).map(|ev| {
                        title_finder.check_event(&ev);
                        ev
                    });
                    let mut html_content = String::with_capacity(md_content.len() * 3 / 2);
                    pulldown_cmark::html::push_html(&mut html_content, parser);
                    let title = title_finder.title.unwrap_or("Chimera markdown".to_string());

                    {
                        let state = app_state.read().await;
                        let mut map = BTreeMap::new();
                        map.insert("body".to_string(), html_content);
                        map.insert("title".to_string(), title);
                        match state.handlebars.render("markdown", &map) {
                            Ok(html) => {
                                // todo: cache this result
                                return Html(html).into_response();
                            }
                            Err(e) => {
                                return Html(format!("Failed to render template: {e}")).into_response();
                            }
                        }
                    }
                },
                Err(msg) => {
                    eprintln!("Could not read source: {}", msg);
                    return Html("Failed to find md file").into_response();
                }
            }
        }
    }

    let mut req = Request::new(axum::body::Body::empty());
    *req.headers_mut() = headers;
    let resp = ServeFile::new(path).try_call(req).await.unwrap();
    resp.into_response()
}
