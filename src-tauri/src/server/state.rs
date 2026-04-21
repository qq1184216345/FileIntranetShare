use super::files::{FileItem, Registry, TextItem};
use super::upload::UploadManager;
use crate::config::AppConfig;
use parking_lot::RwLock;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::broadcast;

/// 实时同步事件，WS 推送给所有在线客户端
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SyncEvent {
    #[serde(rename_all = "camelCase")]
    FileAdded { file: FileItem },
    #[serde(rename_all = "camelCase")]
    FileRemoved { id: String },
    #[serde(rename_all = "camelCase")]
    TextAdded { text: TextItem },
    #[serde(rename_all = "camelCase")]
    TextRemoved { id: String },
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
}

impl AppState {
    /// 构造 AppState。`registry` 由调用方先打开（含 SQLite 持久化数据），这里仅持有。
    pub fn new(config: AppConfig, owner_token: String, registry: Arc<Registry>) -> Arc<Self> {
        let (tx, _) = broadcast::channel::<SyncEvent>(128);
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
        tracing::info!("auth reloaded: jwt secret rotated, passwordEnabled={new_enabled}");
        true
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
