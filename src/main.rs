//use tokio::fs;
use axum::{
    //debug_handler,
    http::StatusCode, response::{Html, IntoResponse}, routing::get, Router
};

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/*path", get(index))
        //.route(path, method_router)
        ;

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// PathExtractor
// Path(path): Path<String>
// ServeFile::new("assets/index.html")
async fn index(axum::extract::Path(path): axum::extract::Path<String>) -> (StatusCode, impl IntoResponse) {
    if let Some((directory, filename)) = path.rsplit_once('/') {
        let body = format!("<h1>Chimera:</h1>\n<div>Directory: {directory}</div>\n<div>File: {filename}</div>");
        return (StatusCode::OK, Html(body));
    }

    let body = format!("<h1>Chimera: Unknown {path}</h1>");
    (StatusCode::OK, Html(body))
}

//     match fs::read_to_string("examples/documentation.md").await {
//         Ok(md_content) => {
//             let md = markdown::to_html(md_content.as_str());
//             stdout().write_all(md.as_bytes()).await.unwrap();
//         },
//         Err(msg) => {
//             eprintln!("Could not read source: {}", msg);
//         }
//     }
// }
