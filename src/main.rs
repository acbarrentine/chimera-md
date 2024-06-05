mod chimera_error;
mod title_finder;

use std::{collections::BTreeMap, sync::Arc};
use axum::{
//    debug_handler,
    extract::State, http::{HeaderMap, Request, StatusCode}, response::{Html, IntoResponse}, routing::get, Router
};
use tokio::sync::RwLock;
use tower_http::services::ServeFile;
use handlebars::Handlebars;

use crate::chimera_error::ChimeraError;
use crate::title_finder::TitleFinder;

struct AppState {
    handlebars: Handlebars<'static>,
}

impl AppState {
    pub fn new() -> Result<Self, ChimeraError> {
        let mut handlebars = Handlebars::new();
        handlebars.set_dev_mode(true);
        handlebars.register_template_file("markdown", "templates/markdown.html")?;
        Ok(AppState{
            handlebars
        })
    }
}

type AppStateType = Arc<RwLock<AppState>>;

#[tokio::main]
async fn main() -> Result<(), ChimeraError> {
    let state = AppState::new()?;
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

//#[debug_handler]
async fn serve_file(
    State(app_state): State<AppStateType>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap
) -> Result<axum::response::Response, ChimeraError> {
    if let Some((_, ext)) = path.rsplit_once('.') {
        if ext.eq_ignore_ascii_case("md") {
            let md_content = tokio::fs::read_to_string(path).await?;
            let mut title_finder = TitleFinder::default();
            let parser = pulldown_cmark::Parser::new_ext(
                md_content.as_str(), pulldown_cmark::Options::ENABLE_TABLES
            ).map(|ev| {
                title_finder.check_event(&ev);
                ev
            });
            let mut html_content = String::with_capacity(md_content.len() * 3 / 2);
            pulldown_cmark::html::push_html(&mut html_content, parser);
            // todo: the title fallback should come from config/environment
            let title = title_finder.title.unwrap_or("Chimera markdown".to_string());

            {
                let state = app_state.read().await;
                let mut map = BTreeMap::new();
                map.insert("body".to_string(), html_content);
                map.insert("title".to_string(), title);

                let html = state.handlebars.render("markdown", &map)?;
                // todo: cache this result
                return Ok((StatusCode::ACCEPTED, Html(html)).into_response());
            }
        }
    }

    let mut req = Request::new(axum::body::Body::empty());
    *req.headers_mut() = headers;
    let resp = ServeFile::new(path).try_call(req).await.unwrap();
    Ok(resp.into_response())
}
