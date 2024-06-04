use std::{collections::BTreeMap, sync::Arc};

use axum::{
    debug_handler,
    extract::State, http::{HeaderMap, Request}, response::{Html, IntoResponse}, routing::get, Router
};
use markdown::{Options, CompileOptions};
use tokio::{sync::RwLock, fs};
use tower_http::services::ServeFile;
use handlebars::Handlebars;

struct AppState {
    handlebars: Handlebars<'static>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            handlebars: Handlebars::new(),
        }
    }
}

type AppStateType = Arc<RwLock<AppState>>;

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(serve_index))
        .route("/*path", get(serve_file))
        .with_state(Arc::new(RwLock::new(AppState::new())))
        //.route("/*path/*.md", get(markdown_file))
        //.route(path, method_router)
        ;

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

//#[debug_handler]
async fn serve_index(
    State(_app_state): State<AppStateType>,
    _headers: HeaderMap
) -> impl IntoResponse {
    let body = "<h1>Chimera root</h1>";
    Html(body)
}

#[debug_handler]
async fn serve_file(
    State(app_state): State<AppStateType>,
    axum::extract::Path(path): axum::extract::Path<String>,
    headers: HeaderMap
) -> impl IntoResponse {
    //println!("serve file called on {path}");
    if let Some((_, ext)) = path.rsplit_once('.') {
        if ext.eq_ignore_ascii_case("md") {
            match tokio::fs::read_to_string(path).await {
                Ok(md_content) => {
                    let md = markdown::to_html_with_options(md_content.as_str(),
                        &Options {
                            compile: CompileOptions {
                                allow_dangerous_html: true,
                                ..CompileOptions::default()
                            },
                            ..Options::default()
                        }
                    ).unwrap();
                    //println!("Markdown:\n {md}");
                    let mut map = BTreeMap::new();
                    map.insert("body".to_string(), md);
                    let mut state = app_state.write().await;
                    if state.handlebars.get_template("markdown.html").is_none() {
                        let file = fs::read_to_string("templates/markdown.html").await;
                        match file {
                            Ok(template_string) => {
                                state.handlebars.register_template_string("markdown.html", template_string).expect("failed to register template");
                            },
                            Err(e) => {
                                return Html(format!("Failed to get markdown html template: {e}")).into_response();
                            }
                        }
                    }
                    match state.handlebars.render("markdown.html", &map) {
                        Ok(html) => {
                            // todo: cache this result
                            return Html(html).into_response();
                        }
                        Err(e) => {
                            return Html(format!("Failed to render template: {e}")).into_response();
                        }
                    }
                },
                Err(msg) => {
                    eprintln!("Could not read source: {}", msg);
                    return Html("Failed to find md file").into_response();
                }
            }
        }
    }

    let mut req = Request::new(axum::body::Body::empty());
    *req.headers_mut() = headers;
    let resp = ServeFile::new(path).try_call(req).await.unwrap();
    resp.into_response()
}
