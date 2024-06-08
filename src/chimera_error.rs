use axum::{http::StatusCode, response::IntoResponse};

#[derive(Debug)]
pub enum ChimeraError {
    MissingMarkdownTemplate(String),
    TemplateRender(String),
    MarkdownFileNotFound(String),
    //StaticFileNotFound(String),
    //StaticFile(String),
}

impl From<handlebars::TemplateError> for ChimeraError {
    fn from(err: handlebars::TemplateError) -> Self {
        ChimeraError::MissingMarkdownTemplate(err.to_string())
    }
}

impl From<handlebars::RenderError> for ChimeraError {
    fn from(err: handlebars::RenderError) -> Self {
        ChimeraError::TemplateRender(err.to_string())
    }
}

impl From<std::io::Error> for ChimeraError {
    fn from(err: std::io::Error) -> Self {
        ChimeraError::MarkdownFileNotFound(err.to_string())
    }
}

//impl From<tower_http::

impl IntoResponse for ChimeraError {
    fn into_response(self) -> axum::response::Response {
        match self {
            ChimeraError::MissingMarkdownTemplate(e) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to load templates/markdown.html: {e}")).into_response()
            },
            ChimeraError::TemplateRender(e) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to render the Handlebars template: {e}")).into_response()
            },
            ChimeraError::MarkdownFileNotFound(e) => {
                (StatusCode::NOT_FOUND, format!("Failed to load file: {e}")).into_response()
            },
            // ChimeraError::StaticFileNotFound(e) => {
            //     (StatusCode::NOT_FOUND, format!("Failed to load file: {e}")).into_response()
            // },
        }
    }
}
