//! 触发 ts-rs 把关键 Rust 类型导出为 TypeScript 到 `../src/bindings/`。
//!
//! 用法：
//! ```bash
//! cd src-tauri
//! cargo test --features export-ts export_bindings
//! ```
//! 生成的文件：src/bindings/{AppConfig,FileItem,TextItem,AuditLog}.ts
//!
//! 仅在启用 `export-ts` feature 时编译，默认构建无开销（P3-16）。

#![cfg(feature = "export-ts")]

use fileshare_lib::config::AppConfig;
use fileshare_lib::server::files::{AuditLog, FileItem, TextItem};
use ts_rs::TS;

#[test]
fn export_bindings() {
    AppConfig::export_all().expect("export AppConfig");
    FileItem::export_all().expect("export FileItem");
    TextItem::export_all().expect("export TextItem");
    AuditLog::export_all().expect("export AuditLog");
    println!("✅ TS bindings written to ../src/bindings/");
}
