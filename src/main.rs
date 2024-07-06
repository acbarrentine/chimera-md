mod chimera_error;
mod document_scraper;
mod full_text_index;
mod html_generator;
mod file_manager;
mod result_cache;

use std::{net::Ipv4Addr, path::PathBuf, sync::Arc};
use axum::{extract::State, http::{HeaderMap, Request, StatusCode}, response::{Html, IntoResponse, Redirect}, routing::get, Form, Router};
use html_generator::HtmlGeneratorCfg;
use result_cache::ResultCache;
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
const HOME_DIR: &str = "/home/";

#[derive(Parser, Debug)]
#[command(about, author, version)]
struct Config {
    #[arg(long, env("CHIMERA_DOCUMENT_ROOT"), default_value_t = String::from("/data/www"))]
    document_root: String,

    #[arg(long, env("CHIMERA_TEMPLATE_ROOT"), default_value_t = String::from("/data/templates"))]
    template_root: String,

    #[arg(long, env("CHIMERA_STYLE_ROOT"), default_value_t = String::from("/data/style"))]
    style_root: String,

    #[arg(long, env("CHIMERA_SEARCH_INDEX_DIR"), default_value_t = String::from("/data/search"))]
    search_index_dir: String,

    #[arg(long, env("CHIMERA_SITE_TITLE"), default_value_t = String::from("Chimera-md"))]
    site_title: String,

    #[arg(long, env("CHIMERA_INDEX_FILE"), default_value_t = String::from("index.md"))]
    index_file: String,

    #[arg(long, env("CHIMERA_HIGHLIGHT_STYLE"), default_value_t = String::from("a11y-dark"))]
    highlight_style: String,

    #[arg(long, env("CHIMERA_LANG"), default_value_t = String::from("en"))]
    site_lang: String,

    #[arg(long, env("CHIMERA_GENERATE_INDEX"))]
    generate_index: Option<bool>,

    #[arg(long, env("CHIMERA_LOG_LEVEL"), value_enum)]
    log_level: Option<tracing::Level>,

    #[arg(long, env("CHIMERA_MAX_CACHE_SIZE"), default_value_t = 50 * 1024 * 1024)]
    max_cache_size: usize,

    #[arg(long, env("CHIMERA_HTTP_PORT"), value_parser = clap::value_parser!(u16).range(1..), default_value_t = 8080)]
    port: u16,
}

struct AppState {
    index_file: String,
    style_root: PathBuf,
    generate_index: bool,
    full_text_index: FullTextIndex,
    html_generator: HtmlGenerator,
    file_manager: FileManager,
    result_cache: ResultCache,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self, ChimeraError> {
        tracing::debug!("Document root: {}", config.document_root);

        let template_root = PathBuf::from(config.template_root.as_str());
        let document_root = PathBuf::from(config.document_root.as_str());
        let search_index_dir = PathBuf::from(config.search_index_dir.as_str());
        std::env::set_current_dir(document_root.as_path())?;

        let mut file_manager = FileManager::new().await?;
        file_manager.add_watch(document_root.as_path());
        file_manager.add_watch(template_root.as_path());

        let result_cache = ResultCache::new(config.max_cache_size);

        let cfg = HtmlGeneratorCfg {
            template_root,
            site_title: config.site_title,
            index_file: config.index_file.as_str(),
            site_lang: config.site_lang,
            highlight_style: config.highlight_style,
            version: VERSION,
            result_cache: result_cache.clone(),
            file_manager: &mut file_manager,
        };
        let html_generator = HtmlGenerator::new(cfg)?;
        
        let mut full_text_index = FullTextIndex::new(search_index_dir.as_path())?;
        full_text_index.scan_directory(document_root, search_index_dir, &file_manager).await?;

        let generate_index = config.generate_index.map_or(false, |v| v);
        Ok(AppState {
            index_file: config.index_file,
            style_root: PathBuf::from(config.style_root),
            generate_index,
            full_text_index,
            html_generator,
            file_manager,
            result_cache,
        })
    }
}

pub(crate) type AppStateType = Arc<AppState>;

#[tokio::main]
async fn main() -> Result<(), ChimeraError> {
    let config = Config::parse();
    let trace_filter = tracing_subscriber::filter::Targets::new()
        .with_default(config.log_level.unwrap_or(tracing::Level::INFO));
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
        .route("/style/*path", get(handle_style))
        .route("/", get(handle_root))
        .route(HOME_DIR, get(handle_root))
        .route(format!("{HOME_DIR}*path").as_str(), get(handle_path))
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
    query: Option<String>,
}

//#[debug_handler]
async fn handle_search(
    State(app_state): State<AppStateType>,
    Form(search): Form<SearchForm>
) -> axum::response::Response {
    if let Some(query) = search.query {
        if !query.is_empty() {
            tracing::debug!("Search for {}", query);
            if let Ok(results) = app_state.full_text_index.search(query.as_str()) {
                if let Ok(html) = app_state.html_generator.gen_search(query.as_str(), results) {
                    return axum::response::Html(html).into_response();
                }
            }
        }
    }
    if let Ok(html) = app_state.html_generator.gen_search_blank() {
        return axum::response::Html(html).into_response();
    }    
    handle_err(app_state).await.into_response()
}

//#[debug_handler]
async fn handle_style(
    State(app_state): State<AppStateType>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap
) -> axum::response::Response {
    let new_path = app_state.style_root.join(path.as_str());
    tracing::debug!("Style request {path} => {}", new_path.display());
    let mut req = Request::new(axum::body::Body::empty());
    *req.headers_mut() = headers;
    match ServeDir::new(new_path.as_path()).try_call(req).await {
        Ok(resp) => {
            tracing::info!("{}: {}", path, resp.status());
            resp.into_response()
        },
        Err(e) => {
            tracing::warn!("Error serving style {}: {e}", new_path.display());
            handle_404(app_state).await.into_response()
        }
    }
}

//#[debug_handler]
async fn handle_path(
    State(app_state): State<AppStateType>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap
) -> axum::response::Response {
    let path = PathBuf::from(path);
    handle_response(app_state, path.as_path(), headers).await
}

async fn handle_root(
    State(app_state): State<AppStateType>,
) -> axum::response::Response {
    let redirect_path = format!("{HOME_DIR}{}", app_state.index_file);
    tracing::info!("Redirecting / => {redirect_path}");
    Redirect::permanent(redirect_path.as_str()).into_response()
}

//#[debug_handler]
async fn handle_fallback(
    State(app_state): State<AppStateType>,
    uri: axum::http::Uri,
) -> axum::response::Response {
    tracing::warn!("404 Not found: {uri}");
    handle_404(app_state).await.into_response()
}

fn has_extension(file_name: &std::path::Path, match_ext: &str) -> bool {
    if let Some(ext) = file_name.extension() {
        return ext.eq_ignore_ascii_case(match_ext);
    }
    false
}

async fn serve_markdown_file(
    app_state: &AppStateType,
    path: &std::path::Path,
) -> Result<(CachedStatus, axum::response::Response), ChimeraError> {
    tracing::debug!("Markdown request {}", path.display());
    if let Some(result) = app_state.result_cache.get(path).await {
        tracing::debug!("Returning cached response for {}", path.display());
        return Ok((CachedStatus::Cached, (StatusCode::OK, Html(result)).into_response()));
    }
    tracing::debug!("Not cached, building {}", path.display());
    let md_content = tokio::fs::read_to_string(path).await?;
    let (scraper, html_content) = parse_markdown(md_content.as_str());
    let peers = match app_state.generate_index {
        true => {
            app_state.file_manager.find_peers(
                path,
                app_state.index_file.as_str()).await
        },
        false => {
            None
        }
    };
    let html = app_state.html_generator.gen_markdown(
        path,
        html_content,
        scraper,
        peers,
    ).await?;
    Ok((CachedStatus::NotCached, (StatusCode::OK, Html(html)).into_response()))
}

async fn serve_static_file(
    path: &std::path::Path,
    headers: HeaderMap,
) -> Result<(CachedStatus, axum::response::Response), ChimeraError> {
    tracing::debug!("Static request {}", path.display());
    let mut req = Request::new(axum::body::Body::empty());
    *req.headers_mut() = headers;
    Ok((CachedStatus::StaticFile, ServeDir::new(path).try_call(req).await?.into_response()))
}

async fn get_response(
    app_state: &AppStateType,
    path: &std::path::Path,
    headers: HeaderMap
) -> Result<(CachedStatus, axum::response::Response), ChimeraError> {
    tracing::debug!("Chimera request {}", path.display());
    if has_extension(path, "md") {
        return serve_markdown_file(app_state, path).await;
    }
    else if path.is_dir() { 
        // is this a folder?
        let path_str = path.to_string_lossy();
        if !path_str.ends_with('/') {
            let path_with_slash = format!("{}/", path_str);
            tracing::debug!("Missing /, redirecting to {path_with_slash}");
            return Ok((CachedStatus::Redirect, Redirect::permanent(path_with_slash.as_str()).into_response()));
        }
        let path_with_index = path.join(app_state.index_file.as_str());
        if path_with_index.exists() {
            tracing::debug!("No file specified, sending {}", path_with_index.display());
            return serve_markdown_file(app_state, &path_with_index).await;
        }
        else if app_state.generate_index {
            if let Some(result) = app_state.result_cache.get(path).await {
                tracing::debug!("Returning cached index for {}", path.display());
                return Ok((CachedStatus::Cached, (StatusCode::OK, Html(result)).into_response()));
            }
            tracing::info!("No file specified. Generating an index result at {}", path.display());
            let links = app_state.file_manager.find_files_in_directory(path, None).await;
            let html = app_state.html_generator.gen_index(path, links).await?;
            return Ok((CachedStatus::NotCached, Html(html).into_response()));
        }
    }
    tracing::debug!("Not md or a dir {}. Falling back to static routing", path.display());
    let path = PathBuf::from(path);
    serve_static_file(path.as_path(), headers).await
}

async fn handle_response(
    app_state: AppStateType,
    path: &std::path::Path,
    headers: HeaderMap,
) -> axum::response::Response {
    match get_response(&app_state, path, headers).await {
        Ok((cached, resp)) => {
            let status = resp.status();
            tracing::info!("{}: {} ({:?})", status, path.display(), cached);
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
            tracing::warn!("IOError processing request for {}: {e:?}", path.display());
            handle_404(app_state).await.into_response()
        }
        Err(e) => {
            tracing::warn!("Error processing request for {}: {e:?}", path.display());
            handle_err(app_state).await.into_response()
        }
    }
}
