use super::files::{FileItem, Registry, TextItem};
use super::upload::UploadManager;
use crate::config::AppConfig;
use parking_lot::RwLock;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

/// 实时同步事件，WS 推送给所有在线客户端
/// 用 Arc 包裹 Item：每次 broadcast 与每个订阅者 clone 都只是 ref-count +1，
/// 避免高并发/多 WS 连接时出现大量结构体深拷贝。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SyncEvent {
    #[serde(rename_all = "camelCase")]
    FileAdded { file: Arc<FileItem> },
    #[serde(rename_all = "camelCase")]
    FileRemoved { id: String },
    #[serde(rename_all = "camelCase")]
    TextAdded { text: Arc<TextItem> },
    #[serde(rename_all = "camelCase")]
    TextRemoved { id: String },
    #[allow(dead_code)]
    Cleared,
}

pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub owner_token: String,
    pub started_at: i64,
    pub registry: Arc<Registry>,
    pub uploads: Arc<UploadManager>,
    pub events: broadcast::Sender<SyncEvent>,
    /// 当前会话的 JWT 签名密钥（支持热轮换：密码变化后旧 token 立即失效）
    pub jwt_secret: RwLock<String>,
    /// 访客密码的 argon2 hash（密码未启用或为空时为 None）
    pub password_hash: RwLock<Option<String>>,
    /// 上次重算 hash 时的 (password_enabled, password) 明文快照，用于幂等判定
    last_auth_spec: RwLock<(bool, String)>,
    /// 鉴权纪元号：每次 reload_auth 导致 JWT 轮换时 +1。
    /// WS 连接握手时快照一份，检测到变动即主动断连，让客户端重新登录。
    auth_epoch: AtomicU64,
}

impl AppState {
    /// 构造 AppState。`registry` 由调用方先打开（含 SQLite 持久化数据），这里仅持有。
    pub fn new(config: AppConfig, owner_token: String, registry: Arc<Registry>) -> Arc<Self> {
        // 容量从 128 调到 1024：在 burst 场景（大量文件批量导入/网络抖动时累积）下
        // 显著降低 Lagged 错误概率，避免 WS 全量 resync 风暴。
        let (tx, _) = broadcast::channel::<SyncEvent>(1024);
        let jwt_secret = super::auth::random_secret();
        let last_spec = (config.password_enabled, config.password.clone());
        let password_hash = compute_password_hash(&config);
        Arc::new(Self {
            config: Arc::new(RwLock::new(config)),
            owner_token,
            started_at: now_secs(),
            registry,
            uploads: UploadManager::new(),
            events: tx,
            jwt_secret: RwLock::new(jwt_secret),
            password_hash: RwLock::new(password_hash),
            last_auth_spec: RwLock::new(last_spec),
            auth_epoch: AtomicU64::new(0),
        })
    }

    pub fn broadcast(&self, event: SyncEvent) {
        let _ = self.events.send(event);
    }

    /// 当前是否需要访客鉴权
    pub fn password_required(&self) -> bool {
        let cfg = self.config.read();
        cfg.password_enabled && !cfg.password.trim().is_empty()
    }

    /// 获取当前 JWT 签名密钥的快照
    pub fn jwt_secret_snapshot(&self) -> String {
        self.jwt_secret.read().clone()
    }

    /// 热替换运行时 config（upload_dir、password 等立即生效；port 等启动时字段不生效）
    pub fn update_config(&self, new_config: AppConfig) {
        *self.config.write() = new_config;
    }

    /// 根据最新 config 重新计算 password_hash。
    /// 若密码启用状态或明文发生变化，轮换 jwt_secret，所有在线 JWT 即时失效。
    /// 返回 true 表示 JWT 已被轮换。
    pub fn reload_auth(&self) -> bool {
        let (new_enabled, new_pw) = {
            let cfg = self.config.read();
            (cfg.password_enabled, cfg.password.clone())
        };
        let changed = {
            let spec = self.last_auth_spec.read();
            spec.0 != new_enabled || spec.1 != new_pw
        };
        if !changed {
            return false;
        }
        *self.last_auth_spec.write() = (new_enabled, new_pw.clone());

        let new_hash = if new_enabled && !new_pw.trim().is_empty() {
            super::auth::hash_password(&new_pw).ok()
        } else {
            None
        };
        *self.password_hash.write() = new_hash;
        *self.jwt_secret.write() = super::auth::random_secret();
        self.auth_epoch.fetch_add(1, Ordering::Release);
        tracing::info!("auth reloaded: jwt secret rotated, passwordEnabled={new_enabled}");
        true
    }

    /// 当前鉴权纪元号快照（WS 用于检测 JWT 是否已被轮换）
    pub fn auth_epoch(&self) -> u64 {
        self.auth_epoch.load(Ordering::Acquire)
    }
}

fn compute_password_hash(cfg: &AppConfig) -> Option<String> {
    if !cfg.password_enabled || cfg.password.trim().is_empty() {
        return None;
    }
    super::auth::hash_password(&cfg.password).ok()
}

pub fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
