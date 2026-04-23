use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(feature = "export-ts")]
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "export-ts", derive(TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "../../src/bindings/"))]
pub struct AppConfig {
    pub auto_start: bool,
    pub upload_dir: PathBuf,
    pub port: u16,
    pub password_enabled: bool,
    pub password: String,
    pub https_enabled: bool,
    pub bind_ipv6: bool,
    /// 磁盘最小保留空间（MB）：上传前若磁盘剩余空间低于 (size + disk_min_free_mb*1MB) 即拒绝。
    /// 0 表示不启用软限制。默认 500MB，避免写满硬盘把系统搞挂。
    #[serde(default = "default_disk_min_free_mb")]
    #[cfg_attr(feature = "export-ts", ts(type = "number"))]
    pub disk_min_free_mb: u64,
}

fn default_disk_min_free_mb() -> u64 {
    500
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
            disk_min_free_mb: default_disk_min_free_mb(),
        }
    }
}
