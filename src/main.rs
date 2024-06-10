mod chimera_error;
mod document_scraper;
mod cache_info;

use std::{cmp::Ordering, collections::BTreeMap, ffi::OsStr, net::Ipv4Addr, sync::Arc, path::PathBuf};
use axum::{
//    debug_handler,
    extract::State, http::{HeaderMap, Request, StatusCode}, response::{Html, IntoResponse, Redirect}, routing::get, Router
};
use tokio::sync::RwLock;
use tower_http::{services::ServeDir, trace::TraceLayer};
use handlebars::Handlebars;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use serde::Serialize;
use clap::Parser;

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

#[derive(Parser, Debug)]
#[command(version, about)]
struct Config {
    #[arg(long, env("CHIMERA_DOCUMENT_ROOT"), default_value_t = String::from("/var/chimera/www"))]
    document_root: String,

    #[arg(long, env("CHIMERA_TEMPLATE_ROOT"), default_value_t = String::from("/var/chimera/template"))]
    template_root: String,

    #[arg(long, env("CHIMERA_SITE_TITLE"), default_value_t = String::from("Chimera Markdown Server"))]
    site_title: String,

    #[arg(long, env("CHIMERA_INDEX_FILE"), default_value_t = String::from("index.md"))]
    index_file: String,

    #[arg(long, env("CHIMERA_LOG_LEVEL"), value_enum)]
    log_level: Option<tracing::Level>,

    #[arg(long, env("CHIMERA_HTTP_PORT"), value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
}
// https://docs.docker.com/compose/environment-variables/env-file/
// https://stackoverflow.com/questions/73528645/how-to-extract-config-value-from-env-variable-with-clap-derive

struct AppState {
    handlebars: Handlebars<'static>,
    document_root: PathBuf,
    markdown_template: PathBuf,
    site_title: String,
    index_file: String,
    cached_results: RwLock<BTreeMap<String, CachedResult>>,
}

impl AppState {
    pub fn new(config: Config, ) -> Result<Self, ChimeraError> {
        let mut handlebars = Handlebars::new();
        handlebars.set_dev_mode(true);

        tracing::info!("Document root: {}", config.document_root);
        let document_root = PathBuf::from(config.document_root);
        std::env::set_current_dir(document_root.as_path())?;

        let mut markdown_template = PathBuf::from(config.template_root.as_str());
        markdown_template.push("markdown.html");
        tracing::debug!("Markdown template file: {}", markdown_template.display());
        handlebars.register_template_file("markdown", markdown_template.to_string_lossy().into_owned())?;

        let mut error_template = PathBuf::from(config.template_root.as_str());
        error_template.push("error.html");
        tracing::debug!("Error template file: {}", error_template.display());
        handlebars.register_template_file("error", error_template.to_string_lossy().into_owned())?;
        Ok(AppState{
            handlebars,
            document_root,
            markdown_template,
            site_title: config.site_title,
            index_file: config.index_file,
            cached_results: RwLock::new(BTreeMap::new()),
        })
    }
}

pub(crate) type AppStateType = Arc<AppState>;

#[tokio::main]
async fn main() -> Result<(), ChimeraError> {
    let config = Config::parse();
    let trace_filter = tracing_subscriber::filter::Targets::new()
        .with_default(config.log_level.unwrap_or(tracing::Level::INFO));
    let tracing_layer = tracing_subscriber::fmt::layer();
    tracing_subscriber::registry()
        .with(tracing_layer)
        .with(trace_filter)
        .init();

    let port = config.port;
    let state = Arc::new(AppState::new(config)?);
    let app = Router::new()
        .route("/*path", get(handle_path))
        .fallback_service(get(handle_fallback).with_state(state.clone()))
        .with_state(state)
        .layer(TraceLayer::new_for_http()
    );

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await.unwrap();
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
    let index_file = app_state.index_file.clone();
    handle_response(app_state, index_file.as_str(), headers).await
}

async fn build_file_list(relative_path: &str) -> Vec<Doclink> {
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
    if let Ok(mut read_dir) = tokio::fs::read_dir(relative_parent_path.as_path()).await {
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

fn get_language_blob(langs: &[&str]) -> String {
    let min_js_prefix = "<script src=\"https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/languages/";
    let min_js_suffix = ".min.js\"></script>\n";
    let min_jis_len = langs.iter().fold(0, |len, el| {
        len + el.len()
    });

    let style = "<link rel=\"stylesheet\" href=\"https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/styles/an-old-hope.min.css\">\n";
    let highlight_js = "<script src=\"https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/highlight.min.js\"></script>\n";
    let invoke_js = "<script>hljs.highlightAll();</script>\n";
    let mut buffer = String::with_capacity(
        style.len() +
        highlight_js.len() +
        min_jis_len +
        invoke_js.len());
    buffer.push_str(style);
    buffer.push_str(highlight_js);
    for lang in langs {
        buffer.push_str(min_js_prefix);
        buffer.push_str(lang);
        buffer.push_str(min_js_suffix);
    }
    buffer.push_str(invoke_js);
    buffer
}

async fn serve_markdown_file(
    app_state: AppStateType,
    path: &str,
) -> Result<axum::response::Response, ChimeraError> {
    tracing::debug!("Markdown request: {path:?}");
    let hb_modtime = get_modtime(app_state.markdown_template.as_path()).await?;
    let modtimes = Modtimes::new(path, hb_modtime).await;
    {
        let cache = app_state.cached_results.read().await;
        let cached_results = cache.get(path);
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
        code_js: String,
        doclinks: Vec<Doclink>,
        file_list: Vec<Doclink>,
        num_files: usize,
    }

    let file_list = build_file_list(path).await;

    let title = title_finder.title.unwrap_or_else(||{
        if let Some((_, slashpos)) = path.rsplit_once('/') {
            slashpos.to_string()
        }
        else {
            path.to_string()
        }
    });

    let code_js = if title_finder.has_code_blocks {
        get_language_blob(&title_finder.code_languages)
    }
    else {
        String::new()
    };

    let vars = HandlebarVars{
        body: html_content,
        title,
        code_js,
        doclinks: title_finder.doclinks,
        num_files: file_list.len(),
        file_list,
    };

    let html = app_state.handlebars.render("markdown", &vars)?;
    tracing::debug!("Generated fresh response for {path}");

    {
        let mut cache = app_state.cached_results.write().await;
        cache.insert(path.to_string(), CachedResult {
            html: html.clone(),
            modtimes,
        });
    }
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
    //let maybe_slash = if path.ends_with('/') {""} else {"/"};
    // let abs_path = {
    //     let read_lock = app_state.read().await;
    //     format!("{}{}{}", read_lock.config.document_root, maybe_slash, path)
    // };
    //let abs = format!("www{slash}{path}");
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
            let path_with_index = format!("{path}index.md");
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
        Err(e) => {
            tracing::warn!("Error processing request: {e:?}");
            handle_err(app_state).await.into_response()
        }
    }
}

