use std::collections::HashMap;
use serde::Deserialize;
use crate::chimera_error::ChimeraError;

#[derive(Deserialize, Debug)]
enum LogLevel {
    Trace,
    Debug,
    Info,
    Warning,
    Error,
} 

#[derive(Deserialize, Debug)]
pub struct TomlConfig {
    #[serde(default = "default_document_root")]
    pub document_root: String,

    #[serde(default = "default_template_root")]
    pub template_root: String,

    #[serde(default = "default_style_root")]
    pub style_root: String,

    #[serde(default = "default_icon_root")]
    pub icon_root: String,

    #[serde(default = "default_search_index_dir")]
    pub search_index_dir: String,

    #[serde(default = "default_site_title")]
    pub site_title: String,

    #[serde(default = "default_index_file")]
    pub index_file: String,

    #[serde(default = "default_highlight_style")]
    pub highlight_style: String,

    #[serde(default = "default_site_lang")]
    pub site_lang: String,

    #[serde(default)]
    pub generate_index: bool,

    #[serde(default = "default_log_level")]
    log_level: LogLevel,

    #[serde(default = "default_max_cache_size")]
    pub max_cache_size: usize,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default)]
    pub redirects: HashMap<String, String>,
}

fn default_document_root() -> String { "/data/www".to_string() }
fn default_template_root() -> String { "/data/templates".to_string() }
fn default_style_root() -> String { "/data/style".to_string() }
fn default_icon_root() -> String { "/data/icon".to_string() }
fn default_search_index_dir() -> String { "/data/search".to_string() }
fn default_site_title() -> String { "Chimera-md".to_string() }
fn default_index_file() -> String { "index.md".to_string() }
fn default_highlight_style() -> String { "an-old-hope".to_string() }
fn default_site_lang() -> String { "en".to_string() }
fn default_log_level() -> LogLevel { LogLevel::Info }
fn default_max_cache_size() -> usize { 50 * 1024 * 1024 }
fn default_port() -> u16 { 8080 }

impl TomlConfig {
    pub fn read_config(config_file: &str) -> Result<TomlConfig, ChimeraError> {
        let config_file_data = match std::fs::read_to_string(config_file) {
            Ok(config_file_data) => config_file_data,
            Err(e) => {
                if let Ok(cwd) = std::env::current_dir() {
                    tracing::debug!("CWD: {}", cwd.display());
                }
                tracing::error!("Failed reading {config_file}");
                return Err(ChimeraError::from(e));
            },
        };
        tracing::debug!("Toml config file: {config_file_data}");
        let config_data: TomlConfig = toml::from_str(config_file_data.as_str())?;
        Ok(config_data)
    }

    pub fn tracing_level(&self) -> tracing::Level {
        match self.log_level {
            LogLevel::Trace => tracing::Level::TRACE,
            LogLevel::Debug => tracing::Level::DEBUG,
            LogLevel::Info => tracing::Level::INFO,
            LogLevel::Warning => tracing::Level::WARN,
            LogLevel::Error => tracing::Level::ERROR,
        }
    }
}