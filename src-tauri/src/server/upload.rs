use parking_lot::RwLock;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// 默认分片大小 4MB
pub const DEFAULT_CHUNK_SIZE: u64 = 4 * 1024 * 1024;
/// 单个分片体积上限 16MB（防止滥用）
pub const MAX_CHUNK_SIZE: u64 = 16 * 1024 * 1024;
/// 单个文件总大小上限 10GB
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadSession {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub chunk_size: u64,
    pub chunk_count: u32,
    pub uploader_ip: String,
    pub created_at: i64,
    /// 已完成的 chunk 索引（0-based），有序去重
    pub uploaded: Vec<u32>,
    /// 临时目录（序列化时不暴露）
    #[serde(skip)]
    pub tmp_dir: PathBuf,
}

impl UploadSession {
    pub fn signature(&self) -> String {
        // name + size 作为续传匹配 key；相同签名视为同一文件
        format!("{}::{}", self.name, self.size)
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

    pub fn chunk_path(&self, index: u32) -> PathBuf {
        self.tmp_dir.join(format!("chunk-{:06}", index))
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

    pub fn mark_chunk(&self, id: &str, index: u32) -> Option<UploadSession> {
        let mut g = self.sessions.write();
        let s = g.get_mut(id)?;
        s.mark_uploaded(index);
        Some(s.clone())
    }

    pub fn remove(&self, id: &str) -> Option<UploadSession> {
        let s = self.sessions.write().remove(id);
        if let Some(ref s) = s {
            self.by_signature.write().remove(&s.signature());
        }
        s
    }
}
