mod chimera_error;
mod document_scraper;
mod full_text_index;
mod html_generator;
mod file_manager;

use std::{net::Ipv4Addr, path::PathBuf, sync::Arc};
use axum::{extract::State, http::{HeaderMap, Request, StatusCode}, response::{Html, IntoResponse, Redirect}, routing::get, Form, Router};
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use serde::Deserialize;
use clap::Parser;

#[allow(unused_imports)]
use axum::debug_handler;

use crate::file_manager::FileManager;
use crate::full_text_index::FullTextIndex;
use crate::html_generator::HtmlGenerator;
use crate::chimera_error::{ChimeraError, handle_404, handle_err};
use document_scraper::parse_markdown;

#[derive(Debug)]
enum CachedStatus {
    Cached,
    NotCached,
    StaticFile,
    Redirect,
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

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
    index_file: String,
    full_text_index: FullTextIndex,
    html_generator: HtmlGenerator,
    file_manager: FileManager,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self, ChimeraError> {
        tracing::debug!("Document root: {}", config.document_root);

        let template_root = PathBuf::from(config.template_root.as_str());
        let document_root = PathBuf::from(config.document_root.as_str());
        std::env::set_current_dir(document_root.as_path())?;

        let mut file_manager = FileManager::new().await?;
        file_manager.add_watch(document_root.as_path())?;
        file_manager.add_watch(template_root.as_path())?;

        let html_generator = HtmlGenerator::new(
            template_root.as_path(),
            config.site_title,
            VERSION,
            &mut file_manager)?;
        let mut full_text_index = FullTextIndex::new()?;
        full_text_index.scan_directory(document_root, &file_manager).await?;
    
        Ok(AppState {
            index_file: config.index_file,
            full_text_index,
            html_generator,
            file_manager,
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

    let mut app = Router::new()
        .route("/search", get(handle_search))
        .route("/*path", get(handle_path))
        .fallback_service(get(handle_fallback).with_state(state.clone()))
        .with_state(state);

    if cfg!(feature="response-timing") {
        tracing::info!("Response timing enabled");
        app = app.layer(tower_http::trace::TraceLayer::new_for_http()
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

fn has_extension(file_name: &str, match_ext: &str) -> bool {
    if let Some((_, ext)) = file_name.rsplit_once('.') {
        return ext.eq_ignore_ascii_case(match_ext);
    }
    false
}

async fn serve_markdown_file(
    app_state: AppStateType,
    path: &str,
) -> Result<(CachedStatus, axum::response::Response), ChimeraError> {
    tracing::debug!("Markdown request {path}");
    if let Some(result) = app_state.html_generator.get_cached_result(path).await {
        tracing::debug!("Returning cached response for {path}");
        return Ok((CachedStatus::Cached, (StatusCode::OK, Html(result)).into_response()));
    }
    tracing::debug!("Not cached, building {path}");
    let md_content = tokio::fs::read_to_string(path).await?;
    let (scraper, html_content) = parse_markdown(md_content.as_str());
    let peer_info = app_state.file_manager.find_peers(
        path,
        app_state.index_file.as_str()).await
        .unwrap_or_default();
    let html = app_state.html_generator.gen_markdown(
        path,
        html_content,
        scraper,
        peer_info,
    ).await?;
    Ok((CachedStatus::NotCached, (StatusCode::OK, Html(html)).into_response()))
}

async fn serve_static_file(
    path: &str,
    headers: HeaderMap,
) -> Result<(CachedStatus, axum::response::Response), ChimeraError> {
    tracing::debug!("Static request {path}");
    let mut req = Request::new(axum::body::Body::empty());
    *req.headers_mut() = headers;
    Ok((CachedStatus::StaticFile, ServeDir::new(path).try_call(req).await?.into_response()))
}

async fn get_response(
    app_state: AppStateType,
    path: &str,
    headers: HeaderMap
) -> Result<(CachedStatus, axum::response::Response), ChimeraError> {
    tracing::debug!("Chimera request {path}");
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
                tracing::debug!("Missing /, redirecting to {path_with_slash}");
                return Ok((CachedStatus::Redirect, Redirect::permanent(path_with_slash.as_str()).into_response()));
            }
            let path_with_index = format!("{path}{}", app_state.index_file.as_str());
            if tokio::fs::metadata(path_with_index.as_str()).await.is_ok() {
                tracing::debug!("No file specified, sending {path_with_index}");
                return serve_markdown_file(app_state, &path_with_index).await;
            }
        }
    }
    tracing::debug!("Not md or a dir {path}. Falling back to static routing");
    serve_static_file(path, headers).await
}

async fn handle_response(
    app_state: AppStateType,
    path: &str,
    headers: HeaderMap,
) -> axum::response::Response {
    match get_response(app_state.clone(), path, headers).await {
        Ok((cached, resp)) => {
            let status = resp.status();
            tracing::info!("{}: {} ({:?})", status, path, cached);
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
