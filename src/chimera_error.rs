use axum::{http::StatusCode, response::IntoResponse};
use std::collections::BTreeMap;

use crate::AppStateType;

#[derive(Debug)]
pub enum ChimeraError {
    MissingMarkdownTemplate(String),
    TemplateRender(String),
    IOError(String),
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

impl IntoResponse for ChimeraError {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Last chance error handler tripped: {self:?}");
        (StatusCode::INTERNAL_SERVER_ERROR, "Chimera internal server error, and then a second failure attempting to render that error").into_response()
    }
}

pub async fn handle_404(
    app_state: AppStateType,
) -> Result<axum::response::Response, ChimeraError> {
    let vars = BTreeMap::from([
        ("error-code", "404: Not found"),
        ("heading", "Page not found"),
        ("message", "The page you are looking for does not exist or has been moved"),
    ]);
    let html = app_state.handlebars.render("error", &vars)?;
    Ok((StatusCode::NOT_FOUND, axum::response::Html(html)).into_response())
}

pub async fn handle_err(
    app_state: AppStateType,
) -> Result<axum::response::Response, ChimeraError> {
    let vars = BTreeMap::from([
        ("error-code", "500: Internal server error"),
        ("heading", "Internal server error"),
        ("message", "Chimera failed attempting to complete this request"),
    ]);
    let html = app_state.handlebars.render("error", &vars)?;
    Ok((StatusCode::NOT_FOUND, axum::response::Html(html)).into_response())
}
