use axum::{http::StatusCode, response::IntoResponse};

use crate::AppStateType;

#[derive(Debug)]
pub enum ChimeraError {
    MissingMarkdownTemplate,
    TemplateRender,
    IOError(String),
    TantivyError,
    QueryError,
    TokioChannel,
    RwLock,
    NotifyError,
}

impl From<handlebars::TemplateError> for ChimeraError {
    fn from(err: handlebars::TemplateError) -> Self {
        tracing::warn!("handlebars::TemplateError: {err}");
        ChimeraError::MissingMarkdownTemplate
    }
}

impl From<handlebars::RenderError> for ChimeraError {
    fn from(err: handlebars::RenderError) -> Self {
        tracing::warn!("handlebars::RenderError: {err}");
        ChimeraError::TemplateRender
    }
}

impl From<std::io::Error> for ChimeraError {
    fn from(err: std::io::Error) -> Self {
        ChimeraError::IOError(err.to_string())
    }
}

impl From<tantivy::TantivyError> for ChimeraError {
    fn from(err: tantivy::TantivyError) -> Self {
        tracing::warn!("tantivy::TantivyError: {err}");
        ChimeraError::TantivyError
    }
}

impl From<tantivy::directory::error::OpenDirectoryError> for ChimeraError {
    fn from(err: tantivy::directory::error::OpenDirectoryError) -> Self {
        tracing::warn!("tantivy::OpenDirectoryError: {err}");
        ChimeraError::TantivyError
    }
}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for ChimeraError {
    fn from(err: tokio::sync::mpsc::error::SendError<T>) -> Self {
        tracing::warn!("tokio::sync::mpsc::error::SendError: {err}");
        ChimeraError::TokioChannel
    }
}

impl<T> From<tokio::sync::broadcast::error::SendError<T>> for ChimeraError {
    fn from(err: tokio::sync::broadcast::error::SendError<T>) -> Self {
        tracing::warn!("tokio::sync::broadcast::error::SendError: {err}");
        ChimeraError::TokioChannel
    }
}

impl From<tantivy::query::QueryParserError> for ChimeraError {
    fn from(err: tantivy::query::QueryParserError) -> Self {
        tracing::warn!("tantivy::query::QueryParserError: {err}");
        ChimeraError::QueryError
    }
}

impl<T> From<std::sync::PoisonError<T>> for ChimeraError {
    fn from(err: std::sync::PoisonError<T>) -> Self {
        tracing::warn!("std::sync::PoisonError: {err}");
        ChimeraError::RwLock
    }
}

impl From<async_watcher::error::Error> for ChimeraError {
    fn from(err: async_watcher::error::Error) -> Self {
        tracing::warn!("async_watcher::error::Error: {err}");
        ChimeraError::NotifyError
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
    Ok((StatusCode::INTERNAL_SERVER_ERROR, axum::response::Html(html)).into_response())
}
