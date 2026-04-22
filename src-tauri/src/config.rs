use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub auto_start: bool,
    pub upload_dir: PathBuf,
    pub port: u16,
    pub password_enabled: bool,
    pub password: String,
    pub https_enabled: bool,
    pub bind_ipv6: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            auto_start: false,
            upload_dir: PathBuf::new(),
            port: 18888,
            password_enabled: false,
            password: String::new(),
            https_enabled: false,
            bind_ipv6: false,
        }
    }
}
