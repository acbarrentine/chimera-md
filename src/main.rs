mod chimera_error;
mod document_scraper;
mod full_text_index;
mod html_generator;
mod file_manager;
mod result_cache;

use std::{net::Ipv4Addr, path::PathBuf, sync::Arc, time::Instant};
use axum::{extract::State, http::{HeaderMap, Request, StatusCode}, middleware::{self, Next}, response::{Html, IntoResponse, Redirect, Response}, routing::get, Form, Router};
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use serde::Deserialize;
use clap::Parser;

#[allow(unused_imports)]
use axum::debug_handler;

use crate::file_manager::{FileManager, PeerInfo};
use crate::full_text_index::FullTextIndex;
use crate::html_generator::{HtmlGenerator, HtmlGeneratorCfg};
use crate::chimera_error::{ChimeraError, handle_404, handle_err};
use crate::document_scraper::parse_markdown;
use crate::result_cache::ResultCache;

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

    #[arg(long, env("CHIMERA_ICON_ROOT"), default_value_t = String::from("/data/icon"))]
    icon_root: String,

    #[arg(long, env("CHIMERA_SEARCH_INDEX_DIR"), default_value_t = String::from("/data/search"))]
    search_index_dir: String,

    #[arg(long, env("CHIMERA_SITE_TITLE"), default_value_t = String::from("Chimera-md"))]
    site_title: String,

    #[arg(long, env("CHIMERA_INDEX_FILE"), default_value_t = String::from("index.md"))]
    index_file: String,

    #[arg(long, env("CHIMERA_HIGHLIGHT_STYLE"), default_value_t = String::from("an-old-hope"))]
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
    icon_root: PathBuf,
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

        let mut file_manager = FileManager::new(
            document_root.as_path(),
            config.index_file.as_str(),
        ).await?;
        file_manager.add_watch(document_root.as_path());
        file_manager.add_watch(template_root.as_path());

        let result_cache = ResultCache::new(config.max_cache_size);
        result_cache.listen_for_changes(&file_manager);

        let cfg = HtmlGeneratorCfg {
            template_root,
            site_title: config.site_title,
            index_file: config.index_file.as_str(),
            site_lang: config.site_lang,
            highlight_style: config.highlight_style,
            version: VERSION,
        };
        let html_generator = HtmlGenerator::new(cfg)?;
        
        let full_text_index = FullTextIndex::new(search_index_dir.as_path())?;
        full_text_index.scan_directory(document_root, search_index_dir, &file_manager).await?;

        let generate_index = config.generate_index.map_or(false, |v| v);
        Ok(AppState {
            index_file: config.index_file,
            style_root: PathBuf::from(config.style_root),
            icon_root: PathBuf::from(config.icon_root),
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

    let app = Router::new()
        .route("/search", get(handle_search))
        .route("/style/*path", get(handle_style))
        .route("/icon/*path", get(handle_icon))
        .route("/", get(handle_root))
        .route(HOME_DIR, get(handle_root))
        .route(format!("{HOME_DIR}*path").as_str(), get(handle_path))
        .route("/*path", get(handle_path))
        .fallback_service(get(handle_fallback).with_state(state.clone()))
        .with_state(state)
        .layer(middleware::from_fn(mw_response_time));

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await.unwrap();
    axum::serve(listener, app).await.unwrap();
    Ok(())
}

async fn mw_response_time(request: axum::extract::Request, next: Next) -> Response {
    let start_time = Instant::now();
    let path = match request.uri().path_and_query() {
        Some(p_and_q) => { p_and_q.as_str().to_owned() },
        None => { request.uri().path().to_string() }
    };
    let response = next.run(request).await;
    let headers = response.headers();
    let cached_status = match headers.contains_key("cached") {
        true => " (cached)",
        false => "",
    };
    let elapsed = start_time.elapsed().as_micros();
    if elapsed > 1000 {
        tracing::info!("{}: {} in {:.3} ms{}", response.status().as_u16(), path, start_time.elapsed().as_micros() as f64 / 1000.0, cached_status);
    }
    else {
        tracing::info!("{}: {} in {} µs{}", response.status().as_u16(), path, start_time.elapsed().as_micros(), cached_status);
    }
    response
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

async fn handle_internal_file(
    app_state: AppStateType,
    path: PathBuf,
    headers: HeaderMap) -> axum::response::Response {
    let mut req = Request::new(axum::body::Body::empty());
    *req.headers_mut() = headers;
    match ServeDir::new(path.as_path()).try_call(req).await {
        Ok(resp) => {
            resp.into_response()
        },
        Err(e) => {
            tracing::warn!("Error serving style {}: {e}", path.display());
            handle_404(app_state).await.into_response()
        }
    }
}

async fn handle_icon(
    State(app_state): State<AppStateType>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap
) -> axum::response::Response {
    let new_path = app_state.icon_root.join(path.as_str());
    tracing::debug!("Icon request {path} => {}", new_path.display());
    handle_internal_file(app_state, new_path, headers).await
}

//#[debug_handler]
async fn handle_style(
    State(app_state): State<AppStateType>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap
) -> axum::response::Response {
    let new_path = app_state.style_root.join(path.as_str());
    tracing::debug!("Style request {path} => {}", new_path.display());
    handle_internal_file(app_state, new_path, headers).await
}

//#[debug_handler]
async fn handle_path(
    State(mut app_state): State<AppStateType>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap
) -> axum::response::Response {
    let path = PathBuf::from(path);
    match get_response(&mut app_state, path.as_path(), headers).await {
        Ok(resp) => {
            let status = resp.status();
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

async fn handle_root(
    State(app_state): State<AppStateType>,
) -> axum::response::Response {
    let redirect_path = format!("{HOME_DIR}{}", app_state.index_file);
    tracing::debug!("Redirecting / => {redirect_path}");
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
    app_state: &mut AppStateType,
    path: &std::path::Path,
) -> Result<axum::response::Response, ChimeraError> {
    tracing::debug!("Markdown request {}", path.display());
    let (html, cached) = match app_state.result_cache.get(path) {
        Some(html) => { (html, true) },
        None => {
            let md_content = tokio::fs::read_to_string(path).await?;
            let (body, scraper) = parse_markdown(md_content.as_str());
            let peers = match app_state.generate_index {
                true => {
                    app_state.file_manager.find_peers(
                        path).await
                },
                false => { PeerInfo::default() }
            };
            let html = app_state.html_generator.gen_markdown(
                path,
                body,
                scraper,
                peers,
            ).await?;
            app_state.result_cache.add(path, html.as_str()).await;
            (html, false)
        }
    };
    if cached {
        Ok((StatusCode::OK, [("cached", "yes")], Html(html)).into_response())
    }
    else {
        Ok((StatusCode::OK, Html(html)).into_response())
    }
}

async fn serve_static_file(
    path: &std::path::Path,
    headers: HeaderMap,
) -> Result<axum::response::Response, ChimeraError> {
    tracing::debug!("Static request {}", path.display());
    let mut req = Request::new(axum::body::Body::empty());
    *req.headers_mut() = headers;
    Ok(ServeDir::new(path).try_call(req).await?.into_response())
}

async fn get_response(
    app_state: &mut AppStateType,
    path: &std::path::Path,
    headers: HeaderMap
) -> Result<axum::response::Response, ChimeraError> {
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
            return Ok(Redirect::permanent(path_with_slash.as_str()).into_response());
        }
        let path_with_index = path.join(app_state.index_file.as_str());
        if path_with_index.exists() {
            tracing::debug!("No file specified, sending {}", path_with_index.display());
            return serve_markdown_file(app_state, &path_with_index).await;
        }
        else if app_state.generate_index {
            tracing::debug!("No file specified. Generating an index result at {}", path.display());
            let links = if let Ok(abs_path) = path.canonicalize() {
                app_state.file_manager.find_files_in_directory(abs_path.as_path(), None).await
            }
            else {
                app_state.file_manager.find_files_in_directory(path, None).await
            };
            let html = app_state.html_generator.gen_index(path, links).await?;
            return Ok(Html(html).into_response());
        }
    }
    tracing::debug!("Not md or a dir {}. Falling back to static routing", path.display());
    serve_static_file(path, headers).await
}
