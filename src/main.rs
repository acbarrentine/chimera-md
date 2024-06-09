mod chimera_error;
mod document_scraper;
mod cache_info;

use std::{cmp::Ordering, collections::BTreeMap, ffi::OsStr, sync::Arc};
use axum::{
//    debug_handler,
    extract::State, http::{HeaderMap, Request, StatusCode}, response::{Html, IntoResponse, Redirect}, routing::get, Router
};
use tokio::sync::RwLock;
use tower_http::{services::ServeDir, trace::TraceLayer};
use handlebars::Handlebars;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use serde::Serialize;

use cache_info::get_modtime;
use chimera_error::{handle_404, handle_err};
use document_scraper::Doclink;

use crate::chimera_error::ChimeraError;
use crate::document_scraper::DocumentScraper;
use crate::cache_info::Modtimes;

struct CachedResult {
    html: String,
    modtimes: Modtimes,
}

struct AppState {
    handlebars: Handlebars<'static>,
    cached_results: BTreeMap<String, CachedResult>,
    server_root: std::path::PathBuf,
}

impl AppState {
    pub fn new(server_root: std::path::PathBuf) -> Result<Self, ChimeraError> {
        let mut handlebars = Handlebars::new();
        handlebars.set_dev_mode(true);
        handlebars.register_template_file("markdown", "templates/markdown.html")?;
        handlebars.register_template_file("error", "templates/error.html")?;
        Ok(AppState{
            handlebars,
            cached_results: BTreeMap::new(),
            server_root,
        })
    }
}

// Config properties needed
// Port
// Path to document root
// Site title
// Path to templates folder
// Log level

pub(crate) type AppStateType = Arc<RwLock<AppState>>;

#[tokio::main]
async fn main() -> Result<(), ChimeraError> {
    let trace_filter = tracing_subscriber::filter::Targets::new()
        .with_target("tower_http::trace::on_response", tracing::Level::TRACE)
        .with_target("tower_http::trace::make_span", tracing::Level::DEBUG)
        .with_default(tracing::Level::INFO)
        //.with_default(tracing::Level::DEBUG)
        ;

    let tracing_layer = tracing_subscriber::fmt::layer();
    tracing_subscriber::registry()
        .with(tracing_layer)
        .with(trace_filter)
        .init();

    let root_path = std::fs::canonicalize("www")?;

    let state = Arc::new(RwLock::new(AppState::new(root_path)?));
    let app = Router::new()
        .route("/*path", get(handle_path))
        .fallback_service(get(handle_fallback).with_state(state.clone()))
        .with_state(state)
        .layer(TraceLayer::new_for_http()
    );

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
    Ok(())
}

//#[debug_handler]
async fn handle_path(
    State(app_state): State<AppStateType>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap
) -> axum::response::Response {
    handle_response(app_state, path.as_str(), headers).await
}

//#[debug_handler]
async fn handle_fallback(
    State(app_state): State<AppStateType>,
    headers: HeaderMap
) -> axum::response::Response {
    handle_response(app_state, "/", headers).await
}

async fn build_file_list(relative_path: &str) -> Vec<Doclink> {
    let mut files = Vec::new();
    let relative_path = std::path::PathBuf::from(relative_path);
    let Some(relative_parent_path) = relative_path.parent() else {
        return files
    };
    let Some(original_file_name) = relative_path.file_name() else {
        return files
    };
    let abs_path = match tokio::fs::canonicalize(relative_parent_path).await {
        Ok(path) => path,
        Err(e) => {
            tracing::warn!("Could not get metadata for {relative_parent_path:?}: {e}");
            return files 
        }
    };
    tracing::debug!("Scanning for files in {abs_path:?}");
    if let Ok(mut read_dir) = tokio::fs::read_dir(abs_path.as_path()).await {
        while let Ok(entry_opt) = read_dir.next_entry().await {
            if let Some(entry) = entry_opt {
                tracing::trace!("Found file: {entry:?}");
                let path = entry.path();
                let file_name = entry.file_name();
                if let Some(extension) = path.extension() {
                    if extension.eq_ignore_ascii_case(OsStr::new("md")) && file_name.ne(original_file_name) {
                        let name_string = file_name.to_string_lossy().to_string();
                        files.push(Doclink{
                            anchor: urlencoding::encode(name_string.as_str()).into_owned(),
                            name: name_string,
                        });
                    }
                }
            }
            else {
                break;
            }
        }
    }
    files.sort_unstable_by(|a, b| {
        if a.name.eq_ignore_ascii_case("index.md") {
            Ordering::Less
        }
        else if b.name.eq_ignore_ascii_case("index.md") {
            Ordering::Greater
        }
        else {
            a.name.cmp(&b.name)
        }
    });
    files
}

fn has_extension(file_name: &str, match_ext: &str) -> bool {
    if let Some((_, ext)) = file_name.rsplit_once('.') {
        return ext.eq_ignore_ascii_case(match_ext);
    }
    false
}

fn add_anchors_to_headings(original_html: String, links: &[Doclink]) -> String {
    let num_links = links.len() - 1;
    if num_links == 0 {
        return original_html;
    }
    let mut link_index = 0;
    let mut new_html = String::with_capacity(original_html.len() * 11 / 10);
    let mut char_iter = original_html.char_indices();
    while let Some(ch) = char_iter.next() {
        let (i, c) = ch;
        if link_index < links.len() && c == '<' {
            if let Some(open_slice) = original_html.get(i..i+4) {
                let mut slit = open_slice.chars().skip(1);
                if slit.next() == Some('h') {
                    if let Some(heading_size) = slit.next() {
                        if slit.next() == Some('>') || slit.next() == Some(' ') {
                            let anchor = links[link_index].anchor.as_str();
                            tracing::trace!("Anchor: {anchor}");
                            new_html.push_str(format!("<h{heading_size} id=\"{anchor}\">").as_str());
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

async fn serve_markdown_file(
    app_state: AppStateType,
    path: &str,
) -> Result<axum::response::Response, ChimeraError> {
    tracing::debug!("Markdown request: {path:?}");
    let hb_modtime = get_modtime(std::path::PathBuf::from("templates/markdown.html").as_path()).await?;
    let modtimes = Modtimes::new(path, hb_modtime).await;
    {
        let state_reader = app_state.read().await;
        let cached_results = state_reader.cached_results.get(path);
        if let Some(results) = cached_results {
            if results.modtimes == modtimes {
                tracing::debug!("Returning cached response for {path}");
                return Ok((StatusCode::ACCEPTED, Html(results.html.clone())).into_response());
            }
        }
    };
    tracing::info!("Not cached, building: {path:?}");

    let md_content = tokio::fs::read_to_string(path).await?;
    let mut title_finder = DocumentScraper::new();
    let parser = pulldown_cmark::Parser::new_ext(
        md_content.as_str(), pulldown_cmark::Options::ENABLE_TABLES
    ).map(|ev| {
        title_finder.check_event(&ev);
        ev
    });
    let mut html_content = String::with_capacity(md_content.len() * 3 / 2);
    pulldown_cmark::html::push_html(&mut html_content, parser);
    let html_content = add_anchors_to_headings(html_content, &title_finder.doclinks);

    #[derive(Serialize)]
    struct HandlebarVars {
        body: String,
        title: String,
        doclinks: Vec<Doclink>,
        file_list: Vec<Doclink>,
        num_files: usize,
    }

    let file_list = build_file_list(path).await;

    // todo: the title fallback should be the file name
    let title = title_finder.title.unwrap_or("Chimera markdown".to_string());
    let mut state_writer = app_state.write().await;

    let vars = HandlebarVars{
        body: html_content,
        title,
        doclinks: title_finder.doclinks,
        num_files: file_list.len(),
        file_list,
    };

    let html = state_writer.handlebars.render("markdown", &vars)?;
    tracing::debug!("Generated fresh response for {path}");

    state_writer.cached_results.insert(path.to_string(), CachedResult {
        html: html.clone(),
        modtimes,
    });
    Ok((StatusCode::ACCEPTED, Html(html)).into_response())
}

async fn serve_static_file(
    _app_state: AppStateType,
    path: &str,
    headers: HeaderMap,
) -> Result<axum::response::Response, ChimeraError> {
    tracing::debug!("Static request: {path:?}");
    let mut req = Request::new(axum::body::Body::empty());
    *req.headers_mut() = headers;
    Ok(ServeDir::new(path).try_call(req).await?.into_response())
}

async fn get_response(
    app_state: AppStateType,
    path: &str,
    headers: HeaderMap
) -> Result<axum::response::Response, ChimeraError> {
    tracing::info!("Chimera request: {path:?}");
    let www_path = format!("www/{path}");
    if has_extension(www_path.as_str(), "md") {
        return serve_markdown_file(app_state, www_path.as_str()).await;
    }
    else {
        // is this a folder?
        let metadata_opt = tokio::fs::metadata(www_path.as_str()).await;
        if let Ok(metadata) = metadata_opt {
            if metadata.is_dir() && !www_path.ends_with('/') {
                let path_with_slash = format!("{path}/");
                return Ok(Redirect::permanent(path_with_slash.as_str()).into_response());
            }
            let path_with_index = format!("{www_path}index.md");
            if tokio::fs::metadata(path_with_index.as_str()).await.is_ok() {
                return serve_markdown_file(app_state, &path_with_index).await;
            }
        }
    }
    serve_static_file(app_state, www_path.as_str(), headers).await
}

async fn handle_response(
    app_state: AppStateType,
    path: &str,
    headers: HeaderMap,
) -> axum::response::Response {
    match get_response(app_state.clone(), path, headers).await {
        Ok(resp) => {
            let status = resp.status();
            tracing::info!("Response ok: {}", status);
            if status.is_success() || status.is_redirection() {
                resp.into_response()
            }
            else if status == StatusCode::NOT_FOUND {
                handle_404(app_state).await.into_response()
            }
            else {
                handle_err(app_state).await.into_response()
            }
        },
        Err(e) => {
            tracing::warn!("Error processing request: {e:?}");
            handle_err(app_state).await.into_response()
        }
    }
}

