use parking_lot::RwLock;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// 默认分片大小 4MB
pub const DEFAULT_CHUNK_SIZE: u64 = 4 * 1024 * 1024;
/// 单个分片体积上限 16MB（防止滥用）
pub const MAX_CHUNK_SIZE: u64 = 16 * 1024 * 1024;
/// 单文件总大小理论上限（保留为常量仅作参考；分片上传路径已不再做强制校验）
#[allow(dead_code)]
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024 * 1024;

/// 上传会话：
/// - 不再使用 `.tmp/<uploadId>/chunk-xxx` 逐片落盘后再合并的策略
/// - init 时直接在最终目录下创建 `<name>.partial` 并 `set_len(size)` 预分配
/// - chunk 通过 `seek(index * chunk_size) + stream copy` 直接写入目标偏移
/// - complete 只做 `rename(.partial -> name)` 一次原子操作
/// 这样避免了一次完整的 "chunk → final" 二次拷贝，单文件 I/O 由 2x 降到 1x。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadSession {
    pub id: String,
    /// 完成后的最终 file_id（init 时就确定，complete 时直接用）
    pub file_id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub chunk_size: u64,
    pub chunk_count: u32,
    pub uploader_ip: String,
    pub created_at: i64,
    /// 已完成的 chunk 索引（0-based），有序去重
    pub uploaded: Vec<u32>,
    /// 上传中的占位文件（`<final_dir>/<name>.partial`），序列化时不暴露
    #[serde(skip)]
    pub partial_path: PathBuf,
    /// 完成后的最终路径（`<final_dir>/<name>`）
    #[serde(skip)]
    pub final_path: PathBuf,
}

impl UploadSession {
    /// 续传匹配键：name + size + uploader_ip
    ///
    /// 加入 IP 是为了避免不同客户端上传同名同大小文件时错误共享会话（P0-3 修复）。
    /// 这只是 LAN 场景下"尽量准"的启发式，完全精确的去重需要文件内容指纹（如前缀哈希）。
    pub fn signature(&self) -> String {
        format!("{}::{}::{}", self.name, self.size, self.uploader_ip)
    }

    pub fn is_complete(&self) -> bool {
        self.uploaded.len() as u32 == self.chunk_count
    }

    pub fn mark_uploaded(&mut self, index: u32) {
        if !self.uploaded.contains(&index) {
            self.uploaded.push(index);
            self.uploaded.sort_unstable();
        }
    }
}

/// 上传会话管理器：支持续传（相同签名返回既有会话）
pub struct UploadManager {
    /// uploadId -> session
    sessions: RwLock<HashMap<String, UploadSession>>,
    /// signature -> uploadId，用于续传查找
    by_signature: RwLock<HashMap<String, String>>,
}

impl UploadManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sessions: RwLock::new(HashMap::new()),
            by_signature: RwLock::new(HashMap::new()),
        })
    }

    pub fn get(&self, id: &str) -> Option<UploadSession> {
        self.sessions.read().get(id).cloned()
    }

    pub fn find_by_signature(&self, sig: &str) -> Option<UploadSession> {
        let id = self.by_signature.read().get(sig).cloned()?;
        self.get(&id)
    }

    pub fn insert(&self, session: UploadSession) {
        self.by_signature
            .write()
            .insert(session.signature(), session.id.clone());
        self.sessions.write().insert(session.id.clone(), session);
    }

    /// 标记分片完成并返回最新已上传列表（避免 clone 整个 session，P2-10 优化）
    pub fn mark_chunk(&self, id: &str, index: u32) -> Option<(Vec<u32>, u32)> {
        let mut g = self.sessions.write();
        let s = g.get_mut(id)?;
        s.mark_uploaded(index);
        Some((s.uploaded.clone(), s.chunk_count))
    }

    pub fn remove(&self, id: &str) -> Option<UploadSession> {
        let s = self.sessions.write().remove(id);
        if let Some(ref s) = s {
            self.by_signature.write().remove(&s.signature());
        }
        s
    }

    /// 遍历所有会话（只读快照），用于启动时清理孤儿 .partial 文件
    #[allow(dead_code)]
    pub fn snapshot(&self) -> Vec<UploadSession> {
        self.sessions.read().values().cloned().collect()
    }
}
