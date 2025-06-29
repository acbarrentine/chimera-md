use std::collections::HashMap;
use indexmap::IndexMap;
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
    #[serde(default = "default_chimera_root")]
    pub chimera_root: String,

    #[serde(default = "default_site_title")]
    pub site_title: String,

    #[serde(default = "default_index_file")]
    pub index_file: String,

    #[serde(default = "default_highlight_style")]
    pub highlight_style: String,

    #[serde(default = "default_site_lang")]
    pub site_lang: String,

    pub image_size_file: Option<String>,

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

    #[serde(default)]
    pub menu: IndexMap<String, String>,

    #[serde(default)]
    pub cache_control: IndexMap<String, usize>,
}

fn default_chimera_root() -> String { "/data".to_string() }
fn default_site_title() -> String { "Chimera-md".to_string() }
fn default_index_file() -> String { "index.md".to_string() }
fn default_highlight_style() -> String { "an-old-hope".to_string() }
fn default_site_lang() -> String { "en".to_string() }
fn default_log_level() -> LogLevel { LogLevel::Info }
fn default_max_cache_size() -> usize { 50 * 1024 * 1024 }
fn default_port() -> u16 { 8080 }

impl TomlConfig {
    /// Reads and parses a TOML configuration file.
    /// 
    /// # Arguments
    /// * `config_file` - Path to the TOML configuration file
    /// 
    /// # Returns
    /// * `Ok(TomlConfig)` - Parsed configuration with defaults applied
    /// * `Err(ChimeraError)` - If file cannot be read or parsed
    /// 
    /// # Configuration Fields
    /// * `chimera_root` - Base directory for all server data (default: "/data")
    /// * `site_title` - Website title displayed in templates (default: "My documents") 
    /// * `site_lang` - HTML language attribute (default: "en")
    /// * `index_file` - Default file for directory requests (default: "index.md")
    /// * `port` - Server port (default: 8080)
    /// * `generate_index` - Auto-generate directory indexes (default: false)
    /// * `highlight_style` - Code syntax highlighting theme (default: "an-old-hope")
    /// * `max_cache_size` - Result cache size limit in bytes (default: 50MB)
    /// * `image_size_file` - Optional image dimensions cache file
    /// * `redirects` - URL redirect mappings (old_path -> new_path)
    /// * `menu` - Navigation menu items (label -> URL)
    /// * `cache_control` - HTTP cache durations by content type (mime_type -> seconds)
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
