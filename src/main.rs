mod chimera_error;
mod document_scraper;
mod full_text_index;
mod html_generator;

use std::{cmp::Ordering, ffi::OsStr, net::Ipv4Addr, path::PathBuf, sync::Arc, time::Duration};
use axum::{extract::State, http::{HeaderMap, Request, StatusCode}, response::{Html, IntoResponse, Redirect}, routing::get, Form, Router};
use full_text_index::FullTextIndex;
use html_generator::HtmlGenerator;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use serde::Deserialize;
use clap::Parser;
use async_watcher::{notify::{EventKind, RecursiveMode}, AsyncDebouncer};

#[allow(unused_imports)]
use axum::debug_handler;

use crate::chimera_error::{ChimeraError, handle_404, handle_err};
use crate::document_scraper::{Doclink, DocumentScraper};

#[derive(Parser, Debug)]
#[command(about, author, version)]
struct Config {
    #[arg(long, env("CHIMERA_DOCUMENT_ROOT"), default_value_t = String::from("/var/chimera-md/www"))]
    document_root: String,

    #[arg(long, env("CHIMERA_TEMPLATE_ROOT"), default_value_t = String::from("/var/chimera-md/template"))]
    template_root: String,

    #[arg(long, env("CHIMERA_SITE_TITLE"), default_value_t = String::from("Chimera-md"))]
    site_title: String,

    #[arg(long, env("CHIMERA_INDEX_FILE"), default_value_t = String::from("index.md"))]
    index_file: String,

    #[arg(long, env("CHIMERA_LOG_LEVEL"), value_enum)]
    log_level: Option<tracing::Level>,

    #[arg(long, env("CHIMERA_HTTP_PORT"), value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
}

struct AppState {
    document_root: PathBuf,
    index_file: String,
    full_text_index: FullTextIndex,
    html_generator: HtmlGenerator,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self, ChimeraError> {
        tracing::debug!("Document root: {}", config.document_root);

        let document_root = PathBuf::from(config.document_root.as_str());
        std::env::set_current_dir(document_root.as_path())?;

        let html_generator = HtmlGenerator::new(&config)?;
        let mut full_text_index = FullTextIndex::new()?;
        full_text_index.scan_directory(config.document_root.as_str()).await?;
    
        Ok(AppState {
            document_root,
            index_file: config.index_file,
            full_text_index,
            html_generator,
        })
    }
}

pub(crate) type AppStateType = Arc<AppState>;

#[tokio::main]
async fn main() -> Result<(), ChimeraError> {
    let config = Config::parse();
    let trace_filter = tracing_subscriber::filter::Targets::new()
        .with_default(config.log_level.unwrap_or(tracing::Level::INFO))
        .with_target("html5ever", tracing::Level::ERROR);
    let tracing_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_line_number(true);
    tracing_subscriber::registry()
        .with(tracing_layer)
        .with(trace_filter)
        .init();

    tracing::info!("Starting up Chimera MD server \"{}\" on port {}", config.site_title, config.port);

    let port = config.port;
    let state = Arc::new(AppState::new(config).await?);

    tokio::spawn(directory_watcher(state.clone()));

    let mut app = Router::new()
        .route("/search", get(handle_search))
        .route("/*path", get(handle_path))
        .fallback_service(get(handle_fallback).with_state(state.clone()))
        .with_state(state);

    if cfg!(response_timing) {
        app = app.layer(tower_http::trace::TraceLayer::new_for_http()
            .make_span_with(
                tower_http::trace::DefaultMakeSpan::new().include_headers(true)
            )
            .on_request(
                tower_http::trace::DefaultOnRequest::new().level(tracing::Level::INFO)
            )
            .on_response(
            tower_http::trace::DefaultOnResponse::new()
                .level(tracing::Level::INFO)
                .latency_unit(tower_http::LatencyUnit::Micros)
            )
        );
    }

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await.unwrap();
    axum::serve(listener, app).await.unwrap();
    Ok(())
}

#[derive(Deserialize)]
struct SearchForm {
    query: String,
}

//#[debug_handler]
async fn handle_search(
    State(app_state): State<AppStateType>,
    Form(search): Form<SearchForm>
) -> axum::response::Response {
    tracing::debug!("Search for {}", search.query);
    if let Ok(results) = app_state.full_text_index.search(search.query.as_str()).await {
        if let Ok(html) = app_state.html_generator.gen_search(search.query.as_str(), results) {
            return axum::response::Html(html).into_response();
        }
    }
    handle_err(app_state).await.into_response()
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
    tracing::debug!("Fallback handler");
    let index_file = app_state.index_file.clone();
    handle_response(app_state, index_file.as_str(), headers).await
}

async fn build_file_list(relative_path: &str, index_file: &str) -> Vec<Doclink> {
    let mut files = Vec::new();
    let relative_path = std::path::PathBuf::from(relative_path);
    tracing::debug!("Relative path: {}", relative_path.display());
    let mut relative_parent_path = match relative_path.parent() {
        Some(relative_parent_path) => relative_parent_path.to_path_buf(),
        None => return files
    };
    let osstr = relative_parent_path.as_mut_os_string();
    if osstr.is_empty() {
        osstr.push(".");
    }
    let Some(original_file_name) = relative_path.file_name() else {
        tracing::debug!("No filename found for {}", relative_path.display());
        return files
    };
    tracing::debug!("Relative path parent: {}", relative_parent_path.display());
    if let Ok(canon) = std::path::Path::canonicalize(&relative_parent_path) {
        tracing::debug!("Canonical: {}", canon.display());
    }
    if let Ok(mut read_dir) = tokio::fs::read_dir(relative_parent_path.as_path()).await {
        while let Ok(entry_opt) = read_dir.next_entry().await {
            if let Some(entry) = entry_opt {
                let path = entry.path();
                let file_name = entry.file_name();
                if let Some(extension) = path.extension() {
                    if extension.eq_ignore_ascii_case(OsStr::new("md")) && file_name.ne(original_file_name) {
                        let name_string = file_name.to_string_lossy().to_string();
                        tracing::debug!("Peer: {}", name_string);
                        files.push(Doclink {
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
        if a.name.eq_ignore_ascii_case(index_file) {
            Ordering::Less
        }
        else if b.name.eq_ignore_ascii_case(index_file) {
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

async fn serve_markdown_file(
    app_state: AppStateType,
    path: &str,
) -> Result<axum::response::Response, ChimeraError> {
    tracing::debug!("Markdown request: {path:?}");
    if let Some(result) = app_state.html_generator.get_cached_result(path).await {
        tracing::debug!("Returning cached response for {path}");
        return Ok((StatusCode::ACCEPTED, Html(result)).into_response());
    }
    tracing::info!("Not cached, building: {path:?}");

    let md_content = tokio::fs::read_to_string(path).await?;
    let mut scraper = DocumentScraper::new();
    let parser = pulldown_cmark::Parser::new_ext(
        md_content.as_str(), pulldown_cmark::Options::ENABLE_TABLES
    ).map(|ev| {
        scraper.check_event(&ev);
        ev
    });
    let mut html_content = String::with_capacity(md_content.len() * 3 / 2);
    pulldown_cmark::html::push_html(&mut html_content, parser);

    let peers = build_file_list(path, app_state.index_file.as_str()).await;

    let html = app_state.html_generator.gen_markdown(
        path,
        html_content,
        scraper,
        peers,
    ).await?;

    Ok((StatusCode::ACCEPTED, Html(html)).into_response())
}

async fn serve_static_file(
    _app_state: AppStateType,
    path: &str,
    headers: HeaderMap,
) -> Result<axum::response::Response, ChimeraError> {
    tracing::info!("Static request: {path:?}");
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
    if has_extension(path, "md") {
        return serve_markdown_file(app_state, path).await;
    }
    else {
        // is this a folder?
        let metadata_opt = tokio::fs::metadata(path).await;
        if let Ok(metadata) = metadata_opt {
            tracing::debug!("Metadata obtained for {path}");
            if metadata.is_dir() && !path.ends_with('/') {
                let path_with_slash = format!("{path}/");
                tracing::info!("Missing /, redirecting to {path_with_slash}");
                return Ok(Redirect::permanent(path_with_slash.as_str()).into_response());
            }
            let path_with_index = format!("{path}{}", app_state.index_file.as_str());
            if tokio::fs::metadata(path_with_index.as_str()).await.is_ok() {
                tracing::info!("No file specified, sending {path_with_index}");
                return serve_markdown_file(app_state, &path_with_index).await;
            }
        }
    }
    tracing::debug!("Not md or a dir {path}. Falling back to static routing");
    serve_static_file(app_state, path, headers).await
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
        Err(ChimeraError::IOError(e)) => {
            tracing::warn!("IOError processing request: {e:?}");
            handle_404(app_state).await.into_response()
        }
        Err(e) => {
            tracing::warn!("Error processing request: {e:?}");
            handle_err(app_state).await.into_response()
        }
    }
}

async fn directory_watcher(app_state: AppStateType) ->Result<(), async_watcher::error::Error> {
    let (mut debouncer, mut file_events) =
        AsyncDebouncer::new_with_channel(Duration::from_secs(1), Some(Duration::from_secs(1))).await?;
    debouncer.watcher().watch(app_state.document_root.as_path(), RecursiveMode::Recursive)?;
    debouncer.watcher().watch(app_state.html_generator.template_root.as_path(), RecursiveMode::Recursive)?;

    while let Some(Ok(events)) = file_events.recv().await {
        for e in events {
            tracing::debug!("File change event {e:?}");
            if let Some(ext) = e.path.extension() {
                if ext == OsStr::new("hbs") {
                    tracing::info!("Handlebars template {} changed. Discarding all cached results", e.path.display());
                    app_state.html_generator.clear_cached_results().await;
                }
                else if e.path.extension() != Some(OsStr::new("md")) {
                    continue;
                }
                match e.event.kind {
                    EventKind::Create(f) => {
                        tracing::debug!("File change event: CREATE - {f:?}, {:?}", e.path);
                        app_state.full_text_index.rescan_document(e.path.as_path()).await;
                    },
                    EventKind::Modify(f) => {
                        tracing::debug!("File change event: MODIFY - {f:?}, {:?}", e.event.paths);
                        for p in e.event.paths {
                            app_state.full_text_index.rescan_document(p.as_path()).await;
                            app_state.html_generator.remove_cached_result(e.path.as_path()).await;
                        }
                    },
                    EventKind::Remove(f) => {
                        tracing::debug!("File change event: REMOVE - {f:?}, {:?}", e.path);
                        app_state.full_text_index.rescan_document(e.path.as_path()).await;
                        app_state.html_generator.remove_cached_result(e.path.as_path()).await;
                    },
                    _ => {}
                };
            }
        }
    }
    Ok(())
}
