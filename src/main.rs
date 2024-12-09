mod chimera_error;
mod toml_config;
mod document_scraper;
mod full_text_index;
mod html_generator;
mod file_manager;
mod result_cache;
mod perf_timer;
mod image_size_cache;

use std::{collections::HashMap, net::{Ipv4Addr, SocketAddr}, path::{self, PathBuf}, sync::Arc, time::Instant};
use axum::{extract::{ConnectInfo, State}, http::{HeaderMap, Request, StatusCode}, middleware::{self, Next}, response::{Html, IntoResponse, Redirect, Response}, routing::get, Form, Router};
use image_size_cache::ImageSizeCache;
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
const CACHED_HEADER: &str = "cached";
const HOME_DIR: &str = "/home";

#[derive(Parser, Debug)]
#[command(about, author, version)]
struct Config {
    #[arg(long, env("CHIMERA_CONFIG_FILE"), default_value_t = String::from("/data/chimera.toml"))]
    config_file: String,
}

struct AppState {
    user_web_root: PathBuf,
    internal_web_root: PathBuf,
    index_file: String,
    generate_index: bool,
    full_text_index: FullTextIndex,
    html_generator: HtmlGenerator,
    file_manager: FileManager,
    known_redirects: HashMap<String, String>,
    result_cache: ResultCache,
}

impl AppState {
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
            //tracing::debug!("Image size file cache: {}", image_size_file.display());
            ImageSizeCache::new(image_size_file)
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
    let chimera_root = path::absolute(toml_config.chimera_root.as_str())?;
    let log_dir = chimera_root.join("log");
    let tracing_level = toml_config.tracing_level();
    let file_appender = tracing_appender::rolling::daily(log_dir, "chimera.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let trace_filter = tracing_subscriber::filter::Targets::new()
        .with_default(tracing_level);
    let file_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_line_number(false)
        .with_filter(trace_filter.clone());
    let tty_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_ansi(true)
        .with_line_number(true)
        .with_filter(trace_filter);
    tracing_subscriber::registry()
        .with(file_layer)
        .with(tty_layer)
        .init();

    tracing::info!("Starting up Chimera MD server \"{}\" on port {}", toml_config.site_title, toml_config.port);
    let port = toml_config.port;
    let state = Arc::new(AppState::new(chimera_root, toml_config).await?);

    let app = Router::new()
        .route("/search", get(handle_search))
        .route(format!("{HOME_DIR}/*path").as_str(), get(handle_home))
        .route(format!("{HOME_DIR}/").as_str(), get(handle_home_folder))
        .route("/*path", get(handle_root_path))
        .route("/", get(handle_root))
        .fallback_service(get(handle_fallback).with_state(state.clone()))
        .with_state(state)
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(middleware::from_fn(mw_response_time));

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await.unwrap();
    let connect_wrapper = app.into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, connect_wrapper)
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

#[debug_middleware]
async fn mw_response_time(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let start_time = Instant::now();
    let path = match request.uri().path_and_query() {
        Some(p_and_q) => { p_and_q.as_str().to_owned() },
        None => { request.uri().path().to_string() }
    };

    let req_headers = request.headers();
    let user_agent = req_headers.get("user-agent").cloned();
    let referer = req_headers.get("referer").cloned();
    let forward_addr = req_headers.get("X-Forwarded-For").cloned();
    let addr = forward_addr.map_or(addr.ip().to_string(), |addr| {
        String::from_utf8_lossy(addr.as_bytes()).to_string()
    });

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
                true => {
                    if let Ok(value) = axum::http::HeaderValue::from_str("public, max-age=360") {
                        headers.insert(axum::http::header::CACHE_CONTROL, value);
                    }
                    tracing::info!("{}: {path} in {elapsed} ms ({cached_status}), user_agent: {user_agent:?}, referer: {referer:?}, addr: {addr}", response.status().as_u16())
                },
                false => tracing::warn!("{}: {path} in {elapsed} ms ({cached_status}), user_agent: {user_agent:?}, referer: {referer:?}, addr: {addr}", response.status().as_u16())
            }
        },
        false => {
            let elapsed = start_time.elapsed().as_micros() as f64 / 1000.0;
            match status.is_success()  || status.is_redirection() {
                true => {
                    if let Ok(value) = axum::http::HeaderValue::from_str("public, max-age=28800") {
                        headers.insert(axum::http::header::CACHE_CONTROL, value);
                    }
                    tracing::debug!("{}: {path} in {elapsed} ms", response.status().as_u16())
                },
                false => tracing::warn!("{}: {path} in {elapsed} ms, user_agent: {user_agent:?}, addr: {addr}", response.status().as_u16())
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

async fn handle_root_path(
    State(app_state): State<AppStateType>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap
) -> axum::response::Response {
    let mut new_path = app_state.user_web_root.join(path.as_str());
    if !new_path.exists() {
        new_path = app_state.internal_web_root.join(path.as_str());
    }
    tracing::debug!("Root request {path} => {}", new_path.display());
    let mut req = Request::new(axum::body::Body::empty());
    *req.headers_mut() = headers;
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
    headers: HeaderMap
) -> axum::response::Response {
    tracing::debug!("handle_home: {path}");
    if let Some(redirect) = app_state.known_redirects.get(&path) {
        tracing::debug!("Known redirect: {path} => {redirect}");
        return Redirect::permanent(redirect).into_response()
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
                app_state.file_manager.find_peers_in_folder(abs_path.as_path(), None)
            }
            else {
                app_state.file_manager.find_peers_in_folder(path, None)
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
    headers: HeaderMap,
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
