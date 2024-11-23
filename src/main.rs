mod chimera_error;
mod toml_config;
mod document_scraper;
mod full_text_index;
mod html_generator;
mod file_manager;
mod result_cache;
mod perf_timer;

use std::{collections::HashMap, net::Ipv4Addr, path::PathBuf, sync::Arc, time::Instant};
use axum::{extract::State, http::{HeaderMap, Request, StatusCode}, middleware::{self, Next}, response::{Html, IntoResponse, Redirect, Response}, routing::get, Form, Router};
use tokio::signal;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use serde::Deserialize;
use clap::Parser;

#[allow(unused_imports)]
use axum::debug_handler;

use crate::file_manager::FileManager;
use crate::full_text_index::FullTextIndex;
use crate::html_generator::{HtmlGenerator, HtmlGeneratorCfg};
use crate::chimera_error::{ChimeraError, handle_404, handle_err};
use crate::document_scraper::parse_markdown;
use crate::result_cache::ResultCache;
use crate::perf_timer::PerfTimer;
use crate::toml_config::TomlConfig;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const HOME_DIR: &str = "/home/";
const SERVER_TIMING: &str = "server-timing";
const CACHED_HEADER: &str = "cached";

#[derive(Parser, Debug)]
#[command(about, author, version)]
struct Config {
    #[arg(long, env("CHIMERA_CONFIG_FILE"), default_value_t = String::from("/data/chimera.toml"))]
    config_file: String,
}

struct AppState {
    index_file: String,
    style_root: PathBuf,
    icon_root: PathBuf,
    generate_index: bool,
    full_text_index: FullTextIndex,
    html_generator: HtmlGenerator,
    file_manager: FileManager,
    known_redirects: HashMap<String, String>,
    result_cache: ResultCache,
}

impl AppState {
    pub async fn new(config: TomlConfig) -> Result<Self, ChimeraError> {
        let template_root = PathBuf::from(config.template_root.as_str());
        let document_root = PathBuf::from(config.document_root.as_str());
        let search_index_dir = PathBuf::from(config.search_index_dir.as_str());

        tracing::debug!("Document root: {}", document_root.to_string_lossy());
        if let Err(e) = std::env::set_current_dir(document_root.as_path()) {
            tracing::error!("Failed to set document root to {}: {e}", document_root.display());
        }

        let mut file_manager = FileManager::new(
            document_root.as_path(),
            config.index_file.as_str(),
        ).await?;
        tracing::debug!("Template root: {}", template_root.to_string_lossy());
        file_manager.add_watch(document_root.as_path());
        file_manager.add_watch(template_root.as_path());

        let result_cache = ResultCache::new(config.max_cache_size);
        result_cache.listen_for_changes(&file_manager);

        let cfg = HtmlGeneratorCfg {
            template_root: config.template_root.as_str(),
            site_title: config.site_title,
            index_file: config.index_file.as_str(),
            site_lang: config.site_lang,
            highlight_style: config.highlight_style,
            version: VERSION,
        };
        tracing::debug!("HtmlGenerator");
        let html_generator = HtmlGenerator::new(cfg)?;
        
        tracing::debug!("Full text index: {}", search_index_dir.to_string_lossy());
        let full_text_index = FullTextIndex::new(search_index_dir.as_path())?;
        full_text_index.scan_directory(document_root, search_index_dir, &file_manager).await?;

        Ok(AppState {
            index_file: config.index_file,
            style_root: PathBuf::from(config.style_root),
            icon_root: PathBuf::from(config.icon_root),
            generate_index: config.generate_index,
            full_text_index,
            html_generator,
            file_manager,
            known_redirects: config.redirects,
            result_cache,
        })
    }
}

pub(crate) type AppStateType = Arc<AppState>;

#[tokio::main]
async fn main() -> Result<(), ChimeraError> {
    let config = Config::parse();
    let toml_config = TomlConfig::read_config(config.config_file.as_str())?;
    let tracing_level = toml_config.tracing_level();
    let trace_filter = tracing_subscriber::filter::Targets::new()
        .with_default(tracing_level);
    let tracing_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_line_number(true);
    tracing_subscriber::registry()
        .with(tracing_layer)
        .with(trace_filter)
        .init();
    let toml_config = TomlConfig::read_config(config.config_file.as_str())?;

    tracing::info!("Starting up Chimera MD server \"{}\" on port {}", toml_config.site_title, toml_config.port);
    let port = toml_config.port;
    let state = Arc::new(AppState::new(toml_config).await?);

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
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(middleware::from_fn(mw_response_time));

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Ctrl-c detected. Shutting down");
        },
        _ = terminate => {
            tracing::info!("Signal detected. Shutting down");
        },
    }
}

async fn mw_response_time(request: axum::extract::Request, next: Next) -> Response {
    let start_time = Instant::now();
    let path = match request.uri().path_and_query() {
        Some(p_and_q) => { p_and_q.as_str().to_owned() },
        None => { request.uri().path().to_string() }
    };
    let mut response = next.run(request).await;
    let status = response.status();
    let headers = response.headers_mut();
    match path.ends_with(".md") {
        true => {
            let cached_status = match headers.remove(CACHED_HEADER) {
                Some(status) => {
                    match status.to_str() {
                        Ok(str) => str.to_string(),
                        Err(_) => "err".to_string(),
                    }
                },
                None => "static".to_string(),
            };
            let elapsed = start_time.elapsed().as_micros() as f64 / 1000.0;
            let time_str = format!("total; dur={}; desc=\"total ({})\"", elapsed, cached_status);
            if let Ok(hval) = axum::http::HeaderValue::from_str(time_str.as_str()) {
                headers.append(SERVER_TIMING, hval);
            }
            match status.is_success() || status.is_redirection() {
                true => tracing::info!("{}: {path} in {elapsed} ms ({cached_status})", response.status().as_u16()),
                false => tracing::warn!("{}: {path} in {elapsed} ms ({cached_status})", response.status().as_u16())
            }
        },
        false => {
            let elapsed = start_time.elapsed().as_micros() as f64 / 1000.0;
            match status.is_success()  || status.is_redirection() {
                true => tracing::debug!("{}: {path} in {elapsed} ms", response.status().as_u16()),
                false => tracing::warn!("{}: {path} in {elapsed} ms", response.status().as_u16())
            }
        },
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
    if let Some(redirect) = app_state.known_redirects.get(&path) {
        tracing::debug!("Known redirect: {path} => {redirect}");
        return Redirect::temporary(redirect).into_response()
    }

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
    let mut headers = axum::http::header::HeaderMap::new();
    let html = match app_state.result_cache.get(path).await {
        Some(html) => {
            if let Ok(hval) = axum::http::HeaderValue::from_str("cached") {
                headers.append(CACHED_HEADER, hval);
            }
            html
        },
        None => {
            let mut perf_timer = PerfTimer::new();
            let md_content = tokio::fs::read_to_string(path).await?;
            perf_timer.sample("read-file", &mut headers);
            let (body, scraper) = parse_markdown(md_content.as_str());
            perf_timer.sample("parse-markdown", &mut headers);
            let peers = match app_state.generate_index {
                true => app_state.file_manager.find_peers(path),
                false => None,
            };
            perf_timer.sample("find-peers", &mut headers);
            let html = app_state.html_generator.gen_markdown(path, body, scraper, peers)?;
            perf_timer.sample("generate-html", &mut headers);
            app_state.result_cache.add(path, html.as_str()).await;
            perf_timer.sample("cache-results", &mut headers);
            if let Ok(hval) = axum::http::HeaderValue::from_str("generated") {
                headers.append(CACHED_HEADER, hval);
            }
            html
        }
    };
    Ok((StatusCode::OK, headers, Html(html)).into_response())
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

async fn serve_index(
    app_state: &mut AppStateType,
    path: &std::path::Path,
) -> Result<axum::response::Response, ChimeraError> {
    let mut headers = axum::http::header::HeaderMap::new();
    let html = match app_state.result_cache.get(path).await {
        Some(html) => {
            if let Ok(hval) = axum::http::HeaderValue::from_str("cached") {
                headers.append(CACHED_HEADER, hval);
            }
            html
        },
        None => {
            tracing::debug!("No file specified. Generating an index result at {}", path.display());
            let peers = if let Ok(abs_path) = path.canonicalize() {
                app_state.file_manager.find_files_in_directory(abs_path.as_path(), None)
            }
            else {
                app_state.file_manager.find_files_in_directory(path, None)
            };
            if let Ok(hval) = axum::http::HeaderValue::from_str("generated") {
                headers.append(CACHED_HEADER, hval);
            }
            app_state.html_generator.gen_index(path, peers).await?
        }
    };
    Ok((StatusCode::OK, headers, Html(html)).into_response())
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
            return serve_index(app_state, path).await;
        }
    }
    tracing::debug!("Not md or a dir {}. Falling back to static routing", path.display());
    serve_static_file(path, headers).await
}
