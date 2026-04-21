use parking_lot::{Mutex, RwLock};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileItem {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub uploader_ip: String,
    pub created_at: i64,
    /// 服务器本地绝对路径（不下发给客户端）
    #[serde(skip)]
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextItem {
    pub id: String,
    pub content: String,
    pub uploader_ip: String,
    pub created_at: i64,
}

/// 文件 / 文本的内存清单，同时持久化到 SQLite（可选）。
///
/// - 写操作：先写内存，再异步地 best-effort 写 DB（写失败仅 warn 不阻塞主流程）。
/// - 读操作：直接读内存 Vec，零 I/O。
/// - DB 不可用时 `db = None`，此时退化为纯内存 Registry，服务照常运行，仅失去持久化。
pub struct Registry {
    files: RwLock<Vec<FileItem>>,
    texts: RwLock<Vec<TextItem>>,
    db: Option<Mutex<Connection>>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS files (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    size        INTEGER NOT NULL,
    mime        TEXT NOT NULL,
    uploader_ip TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    path        TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS texts (
    id          TEXT PRIMARY KEY,
    content     TEXT NOT NULL,
    uploader_ip TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_files_created ON files(created_at);
CREATE INDEX IF NOT EXISTS idx_texts_created ON texts(created_at);
"#;

impl Registry {
    /// 打开（或创建）指定路径的 SQLite 数据库并加载现有记录到内存。
    /// 启动时会校验磁盘上的文件是否仍存在，不在则清理对应记录。
    /// 传入 `None` 则不做持久化，纯内存运行（用于测试或 DB 不可用时回退）。
    pub fn open(db_path: Option<&Path>) -> Arc<Self> {
        let Some(path) = db_path else {
            return Arc::new(Self::memory_only());
        };
        match Self::try_open(path) {
            Ok(reg) => Arc::new(reg),
            Err(e) => {
                tracing::warn!(
                    "open registry db at {:?} failed: {e:#}; fallback to memory-only",
                    path
                );
                Arc::new(Self::memory_only())
            }
        }
    }

    fn memory_only() -> Self {
        Self {
            files: RwLock::new(Vec::new()),
            texts: RwLock::new(Vec::new()),
            db: None,
        }
    }

    fn try_open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)?;
        // 使用 WAL 提升并发 / 容灾
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        conn.execute_batch(SCHEMA)?;

        // 加载文件
        let mut files: Vec<FileItem> = {
            let mut stmt = conn.prepare(
                "SELECT id, name, size, mime, uploader_ip, created_at, path \
                 FROM files ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(FileItem {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    size: row.get::<_, i64>(2)? as u64,
                    mime: row.get(3)?,
                    uploader_ip: row.get(4)?,
                    created_at: row.get(5)?,
                    path: PathBuf::from(row.get::<_, String>(6)?),
                })
            })?;
            rows.filter_map(Result::ok).collect()
        };

        // Reconcile：磁盘上找不到的文件直接从记录里剔除
        let mut missing: Vec<String> = Vec::new();
        files.retain(|f| {
            if f.path.exists() {
                true
            } else {
                missing.push(f.id.clone());
                false
            }
        });
        if !missing.is_empty() {
            let mut stmt = conn.prepare("DELETE FROM files WHERE id = ?1")?;
            for id in &missing {
                let _ = stmt.execute([id]);
            }
            tracing::info!(
                "registry reconcile: removed {} stale file records (files missing on disk)",
                missing.len()
            );
        }

        // 加载文本
        let texts: Vec<TextItem> = {
            let mut stmt = conn.prepare(
                "SELECT id, content, uploader_ip, created_at FROM texts ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(TextItem {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    uploader_ip: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?;
            rows.filter_map(Result::ok).collect()
        };

        tracing::info!(
            "registry loaded: {} files, {} texts from {:?}",
            files.len(),
            texts.len(),
            path
        );

        Ok(Self {
            files: RwLock::new(files),
            texts: RwLock::new(texts),
            db: Some(Mutex::new(conn)),
        })
    }

    // ========== Files ==========

    pub fn add_file(&self, item: FileItem) {
        if let Some(db) = &self.db {
            let res = db.lock().execute(
                "INSERT OR REPLACE INTO files(id, name, size, mime, uploader_ip, created_at, path) \
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    item.id,
                    item.name,
                    item.size as i64,
                    item.mime,
                    item.uploader_ip,
                    item.created_at,
                    item.path.to_string_lossy().to_string(),
                ],
            );
            if let Err(e) = res {
                tracing::warn!("persist add_file failed: {e}");
            }
        }
        self.files.write().push(item);
    }

    pub fn list_files(&self) -> Vec<FileItem> {
        self.files.read().clone()
    }

    pub fn get_file(&self, id: &str) -> Option<FileItem> {
        self.files.read().iter().find(|f| f.id == id).cloned()
    }

    pub fn remove_file(&self, id: &str) -> Option<FileItem> {
        let removed = {
            let mut g = self.files.write();
            g.iter()
                .position(|f| f.id == id)
                .map(|idx| g.remove(idx))
        };
        if removed.is_some() {
            if let Some(db) = &self.db {
                if let Err(e) = db.lock().execute("DELETE FROM files WHERE id = ?1", [id]) {
                    tracing::warn!("persist remove_file failed: {e}");
                }
            }
        }
        removed
    }

    // ========== Texts ==========

    pub fn add_text(&self, item: TextItem) {
        if let Some(db) = &self.db {
            let res = db.lock().execute(
                "INSERT OR REPLACE INTO texts(id, content, uploader_ip, created_at) \
                 VALUES(?1, ?2, ?3, ?4)",
                params![item.id, item.content, item.uploader_ip, item.created_at],
            );
            if let Err(e) = res {
                tracing::warn!("persist add_text failed: {e}");
            }
        }
        self.texts.write().push(item);
    }

    pub fn list_texts(&self) -> Vec<TextItem> {
        self.texts.read().clone()
    }

    pub fn remove_text(&self, id: &str) -> Option<TextItem> {
        let removed = {
            let mut g = self.texts.write();
            g.iter()
                .position(|t| t.id == id)
                .map(|idx| g.remove(idx))
        };
        if removed.is_some() {
            if let Some(db) = &self.db {
                if let Err(e) = db.lock().execute("DELETE FROM texts WHERE id = ?1", [id]) {
                    tracing::warn!("persist remove_text failed: {e}");
                }
            }
        }
        removed
    }

    #[allow(dead_code)]
    pub fn clear_all(&self) {
        self.files.write().clear();
        self.texts.write().clear();
        if let Some(db) = &self.db {
            if let Err(e) = db.lock().execute_batch("DELETE FROM files; DELETE FROM texts;") {
                tracing::warn!("persist clear_all failed: {e}");
            }
        }
    }
}
