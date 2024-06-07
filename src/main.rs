mod chimera_error;
mod title_finder;

use std::{collections::BTreeMap, sync::Arc, time::SystemTime};
use axum::{
//    debug_handler,
    extract::State, http::{HeaderMap, Request, StatusCode}, response::{Html, IntoResponse}, routing::get, Router
};
use title_finder::Doclink;
use tokio::sync::RwLock;
use tower_http::{services::ServeFile, trace::TraceLayer};
use handlebars::Handlebars;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use serde::Serialize;

use crate::chimera_error::ChimeraError;
use crate::title_finder::TitleFinder;

struct CachedResult {
    html: String,
    md_modtime: SystemTime,
    hb_modtime: SystemTime,
}

struct AppState {
    handlebars: Handlebars<'static>,
    cached_results: BTreeMap<String, CachedResult>,
}

impl AppState {
    pub fn new() -> Result<Self, ChimeraError> {
        let mut handlebars = Handlebars::new();
        handlebars.set_dev_mode(true);
        handlebars.register_template_file("markdown", "templates/markdown.html")?;
        Ok(AppState{
            handlebars,
            cached_results: BTreeMap::new(),
        })
    }
}

type AppStateType = Arc<RwLock<AppState>>;

#[tokio::main]
async fn main() -> Result<(), ChimeraError> {
    let trace_filter = tracing_subscriber::filter::Targets::new()
        .with_target("tower_http::trace::on_response", tracing::Level::TRACE)
        .with_target("tower_http::trace::make_span", tracing::Level::DEBUG)
        .with_default(tracing::Level::INFO);

    let tracing_layer = tracing_subscriber::fmt::layer();
    tracing_subscriber::registry()
        .with(tracing_layer)
        .with(trace_filter)
        .init();

    let state = AppState::new()?;
    let app = Router::new()
        .route("/", get(serve_index))
        .route("/*path", get(serve_file))
        .with_state(Arc::new(RwLock::new(state)))
        .layer(TraceLayer::new_for_http());

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

async fn get_modtime(path: &str) -> Result<SystemTime, ChimeraError> {
    let md_metadata = tokio::fs::metadata(path).await?;
    Ok(md_metadata.modified()?)
}

fn add_anchors_to_headings(original_html: String, links: &[Doclink]) -> String {
    let num_links = links.len() - 1;
    if num_links == 0 {
        return original_html;
    }
    tracing::info!("{num_links}");
    let mut link_index = 0;
    let mut new_html = String::with_capacity(original_html.len() * 11 / 10);
    let mut char_iter = original_html.char_indices();
    while let Some(ch) = char_iter.next() {
        let (i, c) = ch;
        if c == '<' {
            if let Some(open_slice) = original_html.get(i..i+4) {
                let mut slit = open_slice.chars().skip(1);
                if slit.next() == Some('h') {
                    if let Some(heading_size) = slit.next() {
                        if slit.next() == Some('>') {
                            let anchor = links[link_index].anchor.as_str();
                            tracing::debug!("Anchor: {anchor}");
                            new_html.push_str(format!("<h{heading_size}><a id=\"{anchor}\"></a>").as_str());
                            link_index += 1;
                            for _ in 0..open_slice.len()-1 {
                                if char_iter.next().is_none() {
                                    return new_html;
                                }
                            }
                            continue;
                        }
                    }
                }
            }
        }
        new_html.push(c);
    }
    new_html
}

#[derive(Serialize)]
struct HandlebarVars {
    body: String,
    title: String,
    doclinks: Vec<Doclink>,
}

//#[debug_handler]
async fn serve_file(
    State(app_state): State<AppStateType>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap
) -> Result<axum::response::Response, ChimeraError> {
    if let Some((_, ext)) = path.rsplit_once('.') {
        if ext.eq_ignore_ascii_case("md") {
            let md_modtime = get_modtime(path.as_str()).await?;
            let hb_modtime = get_modtime("templates/markdown.html").await?;
            {
                let state_reader = app_state.read().await;
                let cached_results = state_reader.cached_results.get(path.as_str());
                if let Some(results) = cached_results {
                    if results.md_modtime == md_modtime && results.hb_modtime == hb_modtime {
                        tracing::debug!("Returning cached response for {path}");
                        return Ok((StatusCode::ACCEPTED, Html(results.html.clone())).into_response());
                    }
                }
            };

            let md_content = tokio::fs::read_to_string(path.as_str()).await?;
            let mut title_finder = TitleFinder::new();
            let parser = pulldown_cmark::Parser::new_ext(
                md_content.as_str(), pulldown_cmark::Options::ENABLE_TABLES
            ).map(|ev| {
                title_finder.check_event(&ev);
                ev
            });
            let mut html_content = String::with_capacity(md_content.len() * 3 / 2);
            pulldown_cmark::html::push_html(&mut html_content, parser);
            let html_content = add_anchors_to_headings(html_content, &title_finder.doclinks);

            // todo: the title fallback should come from config/environment
            let title = title_finder.title.unwrap_or("Chimera markdown".to_string());
            let vars = HandlebarVars{
                body: html_content,
                title,
                doclinks: title_finder.doclinks,
            };

            {
                let mut state_writer = app_state.write().await;

                let html = state_writer.handlebars.render("markdown", &vars)?;
                tracing::debug!("Generated fresh response for {path}");

                state_writer.cached_results.insert(path, CachedResult {
                    html: html.clone(),
                    md_modtime,
                    hb_modtime,
                });
                return Ok((StatusCode::ACCEPTED, Html(html)).into_response());
            }
        }
    }

    let mut req = Request::new(axum::body::Body::empty());
    *req.headers_mut() = headers;
    let resp = ServeFile::new(path).try_call(req).await.unwrap();
    Ok(resp.into_response())
}
