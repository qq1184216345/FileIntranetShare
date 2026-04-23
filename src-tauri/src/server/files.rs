use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(feature = "export-ts")]
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "export-ts", derive(TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "../../src/bindings/"))]
pub struct FileItem {
    pub id: String,
    pub name: String,
    #[cfg_attr(feature = "export-ts", ts(type = "number"))]
    pub size: u64,
    pub mime: String,
    pub uploader_ip: String,
    pub created_at: i64,
    /// 服务器本地绝对路径（不下发给客户端）
    #[serde(skip)]
    #[cfg_attr(feature = "export-ts", ts(skip))]
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "export-ts", derive(TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "../../src/bindings/"))]
pub struct TextItem {
    pub id: String,
    pub content: String,
    pub uploader_ip: String,
    pub created_at: i64,
}

/// 文件 / 文本的内存清单，同时持久化到 SQLite（可选）。
///
/// 设计要点（P1-5 重构）：
/// - 内部用 `IndexMap<String, Arc<Item>>` 同时获得 O(1) 按 id 查找与插入顺序遍历
/// - 对外 list_* 只 clone `Arc`（8 字节），不 clone 整个结构体
/// - 写操作先写内存再 best-effort 写 DB
pub struct Registry {
    files: RwLock<IndexMap<String, Arc<FileItem>>>,
    texts: RwLock<IndexMap<String, Arc<TextItem>>>,
    db: Option<Mutex<Connection>>,
}

/// 审计日志单条记录（P3-18）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "export-ts", derive(TS))]
#[cfg_attr(feature = "export-ts", ts(export, export_to = "../../src/bindings/"))]
pub struct AuditLog {
    pub id: i64,
    pub ts: i64,
    pub kind: String,
    pub ip: String,
    pub detail: String,
}

/// 审计日志最多保留条数（超出按时间倒序裁剪）
const AUDIT_KEEP: i64 = 2000;

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
CREATE TABLE IF NOT EXISTS audit_logs (
    id     INTEGER PRIMARY KEY AUTOINCREMENT,
    ts     INTEGER NOT NULL,
    kind   TEXT NOT NULL,
    ip     TEXT NOT NULL,
    detail TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_logs(ts DESC);
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
            files: RwLock::new(IndexMap::new()),
            texts: RwLock::new(IndexMap::new()),
            db: None,
        }
    }

    fn try_open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)?;
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        conn.execute_batch(SCHEMA)?;

        let mut files: IndexMap<String, Arc<FileItem>> = {
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
            let mut map = IndexMap::new();
            for item in rows.filter_map(Result::ok) {
                map.insert(item.id.clone(), Arc::new(item));
            }
            map
        };

        // Reconcile：磁盘上找不到的文件直接从记录里剔除
        let missing: Vec<String> = files
            .iter()
            .filter_map(|(id, it)| if it.path.exists() { None } else { Some(id.clone()) })
            .collect();
        for id in &missing {
            files.shift_remove(id);
        }
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

        let texts: IndexMap<String, Arc<TextItem>> = {
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
            let mut map = IndexMap::new();
            for item in rows.filter_map(Result::ok) {
                map.insert(item.id.clone(), Arc::new(item));
            }
            map
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

    pub fn add_file(&self, item: FileItem) -> Arc<FileItem> {
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
        let arc = Arc::new(item);
        self.files.write().insert(arc.id.clone(), arc.clone());
        arc
    }

    /// 返回所有文件的 Arc 视图（廉价 clone：只 clone 指针计数）
    pub fn list_files(&self) -> Vec<Arc<FileItem>> {
        self.files.read().values().cloned().collect()
    }

    pub fn get_file(&self, id: &str) -> Option<Arc<FileItem>> {
        self.files.read().get(id).cloned()
    }

    pub fn remove_file(&self, id: &str) -> Option<Arc<FileItem>> {
        let removed = self.files.write().shift_remove(id);
        if removed.is_some() {
            if let Some(db) = &self.db {
                if let Err(e) = db.lock().execute("DELETE FROM files WHERE id = ?1", [id]) {
                    tracing::warn!("persist remove_file failed: {e}");
                }
            }
        }
        removed
    }

    /// 返回所有文件记录持有的 path 快照（用于扫描孤儿物理文件，P1-6）
    pub fn known_file_paths(&self) -> Vec<PathBuf> {
        self.files.read().values().map(|f| f.path.clone()).collect()
    }

    // ========== Texts ==========

    pub fn add_text(&self, item: TextItem) -> Arc<TextItem> {
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
        let arc = Arc::new(item);
        self.texts.write().insert(arc.id.clone(), arc.clone());
        arc
    }

    pub fn list_texts(&self) -> Vec<Arc<TextItem>> {
        self.texts.read().values().cloned().collect()
    }

    pub fn remove_text(&self, id: &str) -> Option<Arc<TextItem>> {
        let removed = self.texts.write().shift_remove(id);
        if removed.is_some() {
            if let Some(db) = &self.db {
                if let Err(e) = db.lock().execute("DELETE FROM texts WHERE id = ?1", [id]) {
                    tracing::warn!("persist remove_text failed: {e}");
                }
            }
        }
        removed
    }

    // ========== Audit Log (P3-18) ==========

    /// 追加一条审计日志；DB 不可用时静默忽略。
    /// 写入后按阈值做一次软性 prune（删除超出 AUDIT_KEEP 的旧记录）。
    pub fn log_audit(&self, kind: &str, ip: &str, detail: &str) {
        let Some(db) = &self.db else { return };
        let ts = super::state::now_secs();
        let conn = db.lock();
        if let Err(e) = conn.execute(
            "INSERT INTO audit_logs(ts, kind, ip, detail) VALUES(?1, ?2, ?3, ?4)",
            params![ts, kind, ip, detail],
        ) {
            tracing::debug!("audit log insert failed: {e}");
            return;
        }
        // 低成本 prune：每 200 次写入执行一次，避免每次都 DELETE
        // 注意：bundled rusqlite 未开启 SQLITE_ENABLE_UPDATE_DELETE_LIMIT，
        // 所以 DELETE 不能用 LIMIT/OFFSET；改成按 id 上界批删。
        let last_id = conn.last_insert_rowid();
        if last_id % 200 == 0 {
            let _ = conn.execute(
                "DELETE FROM audit_logs WHERE id <= (\
                    SELECT id FROM audit_logs ORDER BY id DESC LIMIT 1 OFFSET ?1)",
                params![AUDIT_KEEP],
            );
        }
    }

    /// 查询最近的审计日志（按 ts 倒序）
    pub fn list_audit(&self, limit: i64, offset: i64) -> Vec<AuditLog> {
        let Some(db) = &self.db else { return Vec::new() };
        let conn = db.lock();
        let limit = limit.clamp(1, 500);
        let offset = offset.max(0);
        let Ok(mut stmt) = conn.prepare(
            "SELECT id, ts, kind, ip, detail FROM audit_logs \
             ORDER BY id DESC LIMIT ?1 OFFSET ?2",
        ) else {
            return Vec::new();
        };
        let rows = stmt.query_map(params![limit, offset], |row| {
            Ok(AuditLog {
                id: row.get(0)?,
                ts: row.get(1)?,
                kind: row.get(2)?,
                ip: row.get(3)?,
                detail: row.get(4)?,
            })
        });
        match rows {
            Ok(it) => it.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// 清空审计日志（供 Host 端按钮调用）
    pub fn clear_audit(&self) {
        if let Some(db) = &self.db {
            let _ = db.lock().execute("DELETE FROM audit_logs", []);
        }
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
