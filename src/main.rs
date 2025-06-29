mod chimera_error;
mod toml_config;
mod document_scraper;
mod full_text_index;
mod html_generator;
mod file_manager;
mod result_cache;
mod perf_timer;
mod image_size_cache;
mod access_log_format;

use std::{borrow::Borrow, collections::HashMap, net::{Ipv4Addr, SocketAddr}, path::{self, PathBuf}, sync::Arc};
use axum::{body::HttpBody, extract::{ConnectInfo, State}, http::{Extensions, Request, StatusCode}, middleware::{self, Next}, response::{Html, IntoResponse, Redirect, Response}, routing::get, Form, Router};
use image_size_cache::ImageSizeCache;
use access_log_format::{log_access, AccessLogFormat};
use indexmap::IndexMap;
use tokio::signal;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};
use serde::Deserialize;
use clap::Parser;

#[allow(unused_imports)]
use axum::{debug_handler, debug_middleware};

use crate::file_manager::FileManager;
use crate::full_text_index::FullTextIndex;
use crate::html_generator::{HtmlGenerator, HtmlGeneratorCfg};
use crate::chimera_error::{ChimeraError, handle_404, handle_err};
use crate::document_scraper::parse_markdown;
use crate::result_cache::ResultCache;
use crate::perf_timer::PerfTimer;
use crate::toml_config::TomlConfig;

const SERVER_TIMING: &str = "server-timing";
const HOME_DIR: &str = "/home";

#[derive(Parser, Debug)]
#[command(about, author, version)]
struct Config {
    #[arg(long, env("CHIMERA_CONFIG_FILE"), default_value_t = String::from("/data/chimera.toml"))]
    config_file: String,
}

/// Application state containing all server components and configuration.
/// 
/// This struct holds all the core components needed to serve markdown files:
/// - File system monitoring and management
/// - HTML generation with templating
/// - Full-text search indexing
/// - Result caching for performance
/// - Configuration for routing and caching
struct AppState {
    /// Root directory for user-provided static files (CSS, images, etc.)
    user_web_root: PathBuf,
    /// Root directory for built-in static files (internal CSS, icons, etc.)
    internal_web_root: PathBuf,
    /// Default filename to serve for directory requests (e.g., "index.md")
    index_file: String,
    /// Whether to auto-generate index pages for directories without index files
    generate_index: bool,
    /// Full-text search engine for markdown content
    full_text_index: FullTextIndex,
    /// HTML generator using Tera templates
    html_generator: HtmlGenerator,
    /// File system watcher and peer file discovery
    file_manager: FileManager,
    /// URL redirect mappings from old to new paths
    known_redirects: HashMap<String, String>,
    /// HTTP cache control headers by content type (seconds)
    cache_control: IndexMap<String, usize>,
    /// In-memory cache for rendered HTML content
    result_cache: ResultCache,
}

impl AppState {
    /// Creates a new application state by initializing all server components.
    /// 
    /// This is an expensive async operation that:
    /// 1. Sets up file system watching for templates and documents
    /// 2. Initializes the full-text search index
    /// 3. Configures HTML template rendering
    /// 4. Sets up result caching with change invalidation
    /// 
    /// # Arguments
    /// * `chimera_root` - Base directory containing all server data (home/, www/, etc.)
    /// * `config` - Server configuration from TOML file
    /// 
    /// # Returns
    /// * `Ok(AppState)` - Fully initialized application state
    /// * `Err(ChimeraError)` - If any component fails to initialize
    /// 
    /// # Errors
    /// Can fail if:
    /// - Directory structure is invalid or inaccessible
    /// - Template compilation fails
    /// - Search index initialization fails
    /// - File watchers cannot be established
    pub async fn new(chimera_root: PathBuf, config: TomlConfig) -> Result<Self, ChimeraError> {
        let user_template_root = chimera_root.join("template");
        let internal_template_root = chimera_root.join("template-internal");
        let user_web_root = chimera_root.join("www");
        let internal_web_root = chimera_root.join("www-internal");
        let document_root = chimera_root.join("home");
        let search_index_dir = chimera_root.join("search");

        tracing::debug!("Document root: {}", document_root.display());
        if let Err(e) = std::env::set_current_dir(document_root.as_path()) {
            tracing::error!("Failed to set web root to {}: {e}", document_root.display());
        }

        let mut file_manager = FileManager::new(
            document_root.as_path(),
            config.index_file.as_str(),
        ).await?;
        tracing::debug!("Template roots: User: {}, Internal: {}", user_template_root.display(), internal_template_root.display());
        file_manager.add_watch(document_root.as_path());
        file_manager.add_watch(user_template_root.as_path());
        file_manager.add_watch(internal_template_root.as_path());

        let image_size_cache = config.image_size_file.map(|name| {
            let image_size_file = chimera_root.join(name.as_str());
            file_manager.add_watch(&image_size_file);
            let cache = ImageSizeCache::new(image_size_file);
            cache.listen_for_changes(&file_manager);
            cache
        });

        let result_cache = ResultCache::new(config.max_cache_size);
        result_cache.listen_for_changes(&file_manager);

        let cfg = HtmlGeneratorCfg {
            user_template_root,
            internal_template_root,
            site_title: config.site_title.as_str(),
            site_lang: config.site_lang.as_str(),
            highlight_style: config.highlight_style.as_str(),
            index_file: config.index_file.as_str(),
            menu: config.menu,
            file_manager: &file_manager,
            image_size_cache,
        };
        tracing::debug!("HtmlGenerator");
        let html_generator = HtmlGenerator::new(cfg)?;
        
        tracing::debug!("Full text index: {}", search_index_dir.to_string_lossy());
        let full_text_index = FullTextIndex::new(search_index_dir.as_path())?;
        full_text_index.scan_directory(document_root, search_index_dir, &file_manager).await?;

        Ok(AppState {
            index_file: config.index_file,
            generate_index: config.generate_index,
            user_web_root,
            internal_web_root,
            full_text_index,
            html_generator,
            file_manager,
            cache_control: config.cache_control,
            known_redirects: config.redirects,
            result_cache,
        })
    }
}

pub(crate) type AppStateType = Arc<AppState>;

#[tokio::main]
async fn run(toml_config: TomlConfig, chimera_root: PathBuf) -> Result<(), ChimeraError> {
    tracing::info!("Starting up Chimera MD server \"{}\" on port {}", toml_config.site_title, toml_config.port);
    let port = toml_config.port;
    let state = Arc::new(AppState::new(chimera_root, toml_config).await?);

    let app = Router::new()
        .route("/search", get(handle_search))
        .route(format!("{HOME_DIR}/{{*path}}").as_str(), get(handle_home))
        .route(format!("{HOME_DIR}/").as_str(), get(handle_home_folder))
        .route("/{*path}", get(handle_root_path))
        .route("/", get(handle_root))
        .fallback_service(get(handle_fallback).with_state(state.clone()))
        .with_state(state.clone())
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(middleware::from_fn_with_state(state, mw_headers))
        .layer(middleware::from_fn(mw_access_log))
        ;

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await.unwrap();
    let connect_wrapper = app.into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, connect_wrapper)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();

    Ok(())
}

fn main() -> Result<(), ChimeraError> {
    let config = Config::parse();
    let toml_config = TomlConfig::read_config(config.config_file.as_str())?;

    let chimera_root = path::absolute(toml_config.chimera_root.as_str())?;
    let log_dir = chimera_root.join("log");
    let tracing_level = toml_config.tracing_level();
    let file_appender = tracing_appender::rolling::daily(log_dir, "access_log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let trace_filter = tracing_subscriber::filter::Targets::new()
        .with_default(tracing_level);
    let file_layer = tracing_subscriber::fmt::layer()
        .without_time()
        .compact()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_line_number(false)
        .event_format(AccessLogFormat);
    let tty_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(true)
        .with_line_number(true)
        .with_filter(trace_filter);
    tracing_subscriber::registry()
        .with(file_layer)
        .with(tty_layer)
        .init();

    run(toml_config, chimera_root)
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

fn get_cache_duration(app_state: &AppState, content_type: Option<&str>) -> Option<usize> {
    if let Some(content_type) = content_type {
        for (k, v) in app_state.cache_control.iter() {
            if content_type.starts_with(k) {
                return Some(v.to_owned())
            }
        }
    }
    None
}

#[debug_middleware]
async fn mw_headers(
    State(app_state): State<AppStateType>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let start_time = std::time::Instant::now();
    let path = request.uri().path().to_string();
    let mut response = next.run(request).await;
    let status = response.status();
    let cache_check = response.extensions().get::<bool>().map(|b|b.to_owned());
    let headers = response.headers_mut();
    let content_type = headers.get(axum::http::header::CONTENT_TYPE);
    let content_type = content_type.and_then(|h|h.to_str().ok());
    let cache_duration = get_cache_duration(&app_state, content_type);
    if status.is_success() || status.is_redirection() {
        if let Some(cache_duration) = cache_duration {
            let cache_control_string = format!("public, max-age={cache_duration}");
            if let Ok(value) = axum::http::HeaderValue::from_str(cache_control_string.as_str()) {
                headers.insert(axum::http::header::CACHE_CONTROL, value);
            }
        }
    }

    if path.ends_with(".md") {
        if let Some(b) = cache_check {
            let cache_status = match b {
                true => "cached",
                false => "generated",
            };
            let elapsed = start_time.elapsed().as_micros() as f64 / 1000.0;
            let time_str = format!("total; dur={}; desc=\"total ({})\"", elapsed, cache_status);
            if let Ok(hval) = axum::http::HeaderValue::from_str(time_str.as_str()) {
                headers.append(SERVER_TIMING, hval);
            }
        }
    }

    response
}

//#[debug_middleware]
async fn mw_access_log(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    let method = request.method().to_owned();
    let version = request.version();
    let req_headers = request.headers();
    let user_agent = req_headers.get("user-agent").cloned();
    let referer = req_headers.get("referer").cloned();
    let forward_addr = req_headers.get("X-Forwarded-For").cloned();
    let addr = forward_addr.map_or(addr.ip().to_string(), |addr| {
        String::from_utf8_lossy(addr.as_bytes()).to_string()
    });

    let response = next.run(request).await;
    let status = response.status();
    let content_size = response.headers().get("content-length");
    let ext_size = response.extensions().get::<usize>();
    let size_hint = response.body().size_hint();
    let size_str = match size_hint.upper() {
        Some(hint) => hint.to_string(),
        None => {
            match ext_size {
                Some(size) => size.to_string(),
                None => {
                    match content_size {
                        Some(size) => {
                            size.to_str().unwrap_or("0").to_string()
                        },
                        None => "0".to_string(),
                    }
                },
            }
        },
    };
    log_access(
        status.as_u16(),
        method.as_str(),
        size_str.as_str(),
        version,
        path.as_str(),
        addr.as_str(),
        user_agent.map(|v|v.to_str().expect("user agent extract").to_string()),
        referer.map(|v|v.to_str().expect("referer extract").to_string()),
    );
    response
}

#[derive(Deserialize)]
struct SearchForm {
    query: Option<String>,
}

/// Handles full-text search requests via form submission.
/// 
/// This endpoint processes search queries and returns HTML pages with:
/// - Search results with snippets and links
/// - Empty search page if no query provided
/// - Error page if search fails
/// 
/// # Arguments
/// * `app_state` - Application state containing search index
/// * `search` - Form data containing the search query
/// 
/// # Returns
/// * HTML response with search results or search form
/// 
/// # Behavior
/// - Empty queries return the blank search page
/// - Valid queries are logged and processed by Tantivy
/// - Results are rendered using the search template
/// - Errors fall back to generic error page
//#[debug_handler]
async fn handle_search(
    State(app_state): State<AppStateType>,
    Form(search): Form<SearchForm>
) -> axum::response::Response {
    if let Some(query) = search.query {
        if !query.is_empty() {
            tracing::info!("Search for \"{}\"", query);
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

async fn handle_root_path(
    State(app_state): State<AppStateType>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> axum::response::Response {
    if let Some(redirect) = app_state.known_redirects.get(&path) {
        tracing::debug!("Known redirect: {path} => {redirect}");
        return Redirect::permanent(redirect).into_response()
    }
    let mut new_path = app_state.user_web_root.join(path.as_str());
    if !new_path.exists() {
        new_path = app_state.internal_web_root.join(path.as_str());
    }
    tracing::debug!("Root request {path} => {}", new_path.display());
    let req = Request::new(axum::body::Body::empty());
    match ServeDir::new(new_path.as_path()).try_call(req).await {
        Ok(resp) => {
            resp.into_response()
        },
        Err(e) => {
            tracing::warn!("Error serving file {}: {e}", new_path.display());
            handle_404(app_state).await.into_response()
        }
    }
}

async fn handle_home_folder(
    State(app_state): State<AppStateType>,
) -> axum::response::Response {
    let redirect_path = format!("{HOME_DIR}/{}", app_state.index_file);
    tracing::debug!("Redirecting /home/ => {redirect_path}");
    Redirect::permanent(redirect_path.as_str()).into_response()
}

//#[debug_handler]
async fn handle_home(
    State(mut app_state): State<AppStateType>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> axum::response::Response {
    tracing::debug!("handle_home: {path}");
    let path = PathBuf::from(path);
    match get_response(&mut app_state, path.as_path()).await {
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
    let redirect_path = format!("{HOME_DIR}/{}", app_state.index_file);
    tracing::debug!("Redirecting / => {redirect_path}");
    Redirect::permanent(redirect_path.as_str()).into_response()
}

//#[debug_handler]
async fn handle_fallback(
    State(app_state): State<AppStateType>,
    uri: axum::http::Uri,
) -> axum::response::Response {
    tracing::warn!("404: {uri}");
    handle_404(app_state).await.into_response()
}

fn has_extension(file_name: &std::path::Path, match_ext: &str) -> bool {
    if let Some(ext) = file_name.extension() {
        return ext.eq_ignore_ascii_case(match_ext);
    }
    false
}

/// Serves a markdown file as HTML with caching and performance timing.
/// 
/// This is the core function for processing markdown files. It handles:
/// - Cache lookup for previously rendered HTML
/// - Markdown parsing and HTML generation
/// - Peer file discovery for navigation
/// - Auto-generated index pages for missing files
/// - Performance timing via Server-Timing headers
/// 
/// # Arguments
/// * `app_state` - Mutable reference to application state for caching
/// * `path` - Filesystem path to the markdown file
/// 
/// # Returns
/// * `Ok(Response)` - HTML response with appropriate headers and timing
/// * `Err(ChimeraError)` - If file reading or processing fails
/// 
/// # Caching Behavior
/// - Cache hit: Returns stored HTML with timing header indicating cache status
/// - Cache miss: Processes file, stores result, returns new HTML
/// - Non-existent files: May generate index pages if enabled in config
/// 
/// # Performance Timing
/// Uses Server-Timing headers to report:
/// - File reading time
/// - Markdown parsing time  
/// - Peer discovery time
/// - HTML generation time
/// - Cache storage time
async fn serve_markdown_file(
    app_state: &mut AppStateType,
    path: &std::path::Path,
) -> Result<axum::response::Response, ChimeraError> {
    tracing::debug!("Markdown request {}", path.display());
    let mut headers = axum::http::header::HeaderMap::new();
    let mut ext = Extensions::new();
    let html = match app_state.result_cache.get(path).await {
        Some(html) => {
            ext.insert(true);
            html
        },
        None => {
            if path.exists() {
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
                ext.insert(false);
                html
            }
            else if path.ends_with(app_state.index_file.as_str()){
                let mut perf_timer = PerfTimer::new();
                tracing::debug!("Generating an index result at {}", path.display());
                let folder = match path.parent() {
                    Some(p) => p,
                    None => std::path::Path::new("/"),
                };
                let peers = match app_state.generate_index {
                    true => app_state.file_manager.find_peers_in_folder(folder, None),
                    false => None,
                };
                perf_timer.sample("find-peers", &mut headers);
                ext.insert(false);
                let html = app_state.html_generator.gen_index(path, peers).await?;
                perf_timer.sample("generate-html", &mut headers);
                app_state.result_cache.add(path, html.as_str()).await;
                perf_timer.sample("cache-results", &mut headers);
                html
            }
            else {
                return Ok(StatusCode::NOT_FOUND.into_response())
            }
        }
    };
    ext.insert(html.len());
    Ok((StatusCode::OK, headers, ext, Html(html)).into_response())
}

async fn serve_static_file(
    path: &std::path::Path,
) -> Result<axum::response::Response, ChimeraError> {
    tracing::debug!("Static request {}", path.display());
    let req = Request::new(axum::body::Body::empty());
    Ok(ServeDir::new(path).try_call(req).await?.into_response())
}

async fn get_response(
    app_state: &mut AppStateType,
    path: &std::path::Path,
) -> Result<axum::response::Response, ChimeraError> {
    tracing::debug!("Chimera request {}", path.display());
    if has_extension(path, "md") {
        return serve_markdown_file(app_state, path).await;
    }
    else if path.is_dir() { 
        let new_path = path::Path::new(HOME_DIR).join(path).join(app_state.index_file.as_str());
        tracing::debug!("Not a file. Redirecting to {}", new_path.display());
        return Ok(Redirect::permanent(new_path.to_string_lossy().borrow()).into_response());
    }
    tracing::debug!("Not md or a dir {}. Falling back to static routing", path.display());
    serve_static_file(path).await
}
