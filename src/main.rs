mod chimera_error;
mod document_scraper;
mod full_text_index;

use std::{cmp::Ordering, collections::BTreeMap, ffi::OsStr, net::Ipv4Addr, path::PathBuf, sync::Arc, time::Duration};
use axum::{extract::State, http::{HeaderMap, Request, StatusCode}, response::{Html, IntoResponse, Redirect}, routing::get, Form, Router};
use full_text_index::{FullTextIndex, SearchResult};
use tokio::sync::RwLock;
use tower_http::services::ServeDir;
use handlebars::{DirectorySourceOptions, Handlebars};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use serde::{Deserialize, Serialize};
use clap::Parser;
use async_watcher::{notify::{EventKind, RecursiveMode}, AsyncDebouncer};

#[allow(unused_imports)]
use axum::debug_handler;

use crate::chimera_error::{ChimeraError, handle_404, handle_err};
use crate::document_scraper::{Doclink, DocumentScraper};

struct CachedResult {
    html: String,
}

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
    handlebars: Handlebars<'static>,
    document_root: PathBuf,
    template_root: PathBuf,
    site_title: String,
    index_file: String,
    cached_results: RwLock<BTreeMap<String, CachedResult>>,
    full_text_index: FullTextIndex,
}

impl AppState {
    pub fn new(config: Config, full_text_index: FullTextIndex) -> Result<Self, ChimeraError> {
        let mut handlebars = Handlebars::new();
        handlebars.set_dev_mode(true);

        tracing::debug!("Document root: {}", config.document_root);
        let document_root = PathBuf::from(config.document_root);
        std::env::set_current_dir(document_root.as_path())?;

        let template_root = PathBuf::from(config.template_root.as_str());
        handlebars.register_templates_directory(template_root.as_path(), DirectorySourceOptions::default())?;

        // verify we have all the needed templates
        let required_templates = ["markdown", "error", "search"];
        for name in required_templates {
            if !handlebars.has_template(name) {
                let template_name = format!("{name}.hbs");
                tracing::error!("Missing required template: {template_name}");
                return Err(ChimeraError::MissingMarkdownTemplate(template_name));
            }
        }

        Ok(AppState {
            handlebars,
            document_root,
            template_root,
            site_title: config.site_title,
            index_file: config.index_file,
            cached_results: RwLock::new(BTreeMap::new()),
            full_text_index,
        })
    }

    async fn remove_cached_document(&self, path: &std::path::Path) {
        if let Ok(relative_path) = path.strip_prefix(self.document_root.as_path()) {
            tracing::info!("Relative path {}", relative_path.display());
            let path_string = relative_path.to_string_lossy();
            let path_string = path_string.into_owned();
            let mut map = self.cached_results.write().await;
            if map.remove(&path_string).is_some() {
                tracing::info!("Removed {path_string} from HTML cache");
            }
        }
    }

    async fn remove_all_cached_documents(&self) {
        let mut map = self.cached_results.write().await;
        map.clear();
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

    let mut full_text_index = FullTextIndex::new()?;
    full_text_index.scan_directory(config.document_root.as_str()).await?;

    let port = config.port;
    let state = Arc::new(AppState::new(config, full_text_index)?);

    tokio::spawn(directory_watcher(state.clone()));

    let app = Router::new()
        .route("/search", get(handle_search))
        .route("/*path", get(handle_path))
        .fallback_service(get(handle_fallback).with_state(state.clone()))
        .with_state(state)
        //.layer(TraceLayer::new_for_http()
            // .make_span_with(
            //     tower_http::trace::DefaultMakeSpan::new().include_headers(true)
            // )
            // .on_request(
            //     tower_http::trace::DefaultOnRequest::new().level(tracing::Level::INFO)
            // )
            // .on_response(
            // tower_http::trace::DefaultOnResponse::new()
            //     .level(tracing::Level::INFO)
            //     .latency_unit(tower_http::LatencyUnit::Micros)
            // )
        //)
        ;

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
    match app_state.full_text_index.search(search.query.as_str()).await {
        Ok(results) => {
            #[derive(Serialize)]
            struct HandlebarVars {
                site_title: String,
                query: String,
                num_results: usize,
                results: Vec<SearchResult>,
            }
            tracing::info!("Got {} search results", results.len());
            let vars = HandlebarVars {
                site_title: app_state.site_title.clone(),
                query: search.query,
                num_results: results.len(),
                results,
            };
            match app_state.handlebars.render("search", &vars) {
                Ok(html) => {
                    axum::response::Html(html).into_response()
                },
                Err(e) => {
                    tracing::warn!("Error processing request: {e:?}");
                    handle_err(app_state).await.into_response()
                }
            }
        },
        Err(_e) => {
            handle_err(app_state).await.into_response()
        }
    }
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
    if let Ok(mut read_dir) = tokio::fs::read_dir(relative_parent_path.as_path()).await {
        while let Ok(entry_opt) = read_dir.next_entry().await {
            if let Some(entry) = entry_opt {
                tracing::trace!("Found file: {entry:?}");
                let path = entry.path();
                let file_name = entry.file_name();
                if let Some(extension) = path.extension() {
                    if extension.eq_ignore_ascii_case(OsStr::new("md")) && file_name.ne(original_file_name) {
                        let name_string = file_name.to_string_lossy().to_string();
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

fn add_anchors_to_headings(original_html: String, links: &[Doclink]) -> String {
    let num_links = links.len() - 1;
    if num_links == 0 {
        return original_html;
    }
    let mut link_index = 0;
    let mut new_html = String::with_capacity(original_html.len() * 11 / 10);
    let mut char_iter = original_html.char_indices();
    while let Some((i, c)) = char_iter.next() {
        if link_index < links.len() && c == '<' {
            if let Some(open_slice) = original_html.get(i..i+4) {
                let mut slice_it = open_slice.chars().skip(1);
                if slice_it.next() == Some('h') {
                    if let Some(heading_size) = slice_it.next() {
                        if slice_it.next() == Some('>') {
                            let anchor = links[link_index].anchor.as_str();
                            tracing::debug!("Rewriting anchor: {anchor}");
                            new_html.push_str(format!("<h{heading_size} id=\"{anchor}\">").as_str());
                            link_index += 1;
                            for _ in 0..open_slice.len()-1 {
                                if char_iter.next().is_none() {
                                    return new_html;
                                }
                            }
                            continue;
                        }
                        else if slice_it.next() == Some(' ') {
                            // already has an id?
                            link_index += 1;
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
    let min_js_prefix = r#"<script src="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/languages/"#;
    let min_js_suffix = r#"".min.js"></script>
    "#;
    let min_jis_len = langs.iter().fold(0, |len, el| {
        len + el.len()
    });

    let style = r#"<link rel="stylesheet" href="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/styles/an-old-hope.min.css">
    "#;
    let highlight_js = r#"<script src="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/highlight.min.js"></script>
    "#;
    let invoke_js = r#"<script>hljs.highlightAll();</script>
    "#;
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
    {
        let cache = app_state.cached_results.read().await;
        let cached_results = cache.get(path);
        if let Some(results) = cached_results {
            tracing::debug!("Returning cached response for {path}");
            return Ok((StatusCode::ACCEPTED, Html(results.html.clone())).into_response());
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
        site_title: String,
        code_js: String,
        doclinks: Vec<Doclink>,
        file_list: Vec<Doclink>,
        num_files: usize,
    }

    let file_list = build_file_list(path, app_state.index_file.as_str()).await;

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

    let vars = HandlebarVars {
        body: html_content,
        title,
        site_title: app_state.site_title.clone(),
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
            html: html.clone()
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
    debouncer.watcher().watch(app_state.template_root.as_path(), RecursiveMode::Recursive)?;

    while let Some(Ok(events)) = file_events.recv().await {
        for e in events {
            tracing::debug!("File change event {e:?}");
            if let Some(ext) = e.path.extension() {
                if ext == OsStr::new("hbs") {
                    tracing::info!("Handlebars template {} changed. Discarding all cached results", e.path.display());
                    app_state.remove_all_cached_documents().await;
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
                            app_state.remove_cached_document(e.path.as_path()).await;
                        }
                    },
                    EventKind::Remove(f) => {
                        tracing::debug!("File change event: REMOVE - {f:?}, {:?}", e.path);
                        app_state.full_text_index.rescan_document(e.path.as_path()).await;
                        app_state.remove_cached_document(e.path.as_path()).await;
                    },
                    _ => {}
                };
            }
        }
    }
    Ok(())
}
