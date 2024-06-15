use axum::{http::StatusCode, response::IntoResponse};
use std::path::PathBuf;

use crate::AppStateType;

#[derive(Debug)]
pub enum ChimeraError {
    MissingMarkdownTemplate(String),
    TemplateRender(String),
    IOError(String),
    TantivyError(String),
    QueryError(String),
    TokioChannel(String),
    RwLock(String),
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
        ChimeraError::IOError(err.to_string())
    }
}

impl From<tantivy::TantivyError> for ChimeraError {
    fn from(err: tantivy::TantivyError) -> Self {
        ChimeraError::TantivyError(err.to_string())
    }
}

impl From<tokio::sync::mpsc::error::SendError<PathBuf>> for ChimeraError {
    fn from(err: tokio::sync::mpsc::error::SendError<PathBuf>) -> Self {
        ChimeraError::TokioChannel(err.to_string())
    }
}

impl From<tantivy::query::QueryParserError> for ChimeraError {
    fn from(err: tantivy::query::QueryParserError) -> Self {
        ChimeraError::QueryError(err.to_string())
    }
}

impl<T> From<std::sync::PoisonError<T>> for ChimeraError {
    fn from(err: std::sync::PoisonError<T>) -> Self {
        ChimeraError::RwLock(err.to_string())
    }
}

impl IntoResponse for ChimeraError {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Last chance error handler tripped: {self:?}");
        (StatusCode::INTERNAL_SERVER_ERROR, "Chimera internal server error, and then a second failure attempting to render that error").into_response()
    }
}

pub async fn handle_404(
    app_state: AppStateType,
) -> Result<axum::response::Response, ChimeraError> {
    let html = app_state.html_generator.gen_error(
        "404: Not found",
        "Page not found",
        "The page you are looking for does not exist or has been moved",
    )?;
    Ok((StatusCode::NOT_FOUND, axum::response::Html(html)).into_response())
}

pub async fn handle_err(
    app_state: AppStateType,
) -> Result<axum::response::Response, ChimeraError> {
    let html = app_state.html_generator.gen_error(
        "500: Internal server error",
        "Internal server error",
        "Chimera failed attempting to complete this request",
    )?;
    Ok((StatusCode::NOT_FOUND, axum::response::Html(html)).into_response())
}
