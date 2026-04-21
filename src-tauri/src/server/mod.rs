pub mod auth;
pub mod files;
mod firewall;
mod routes;
pub mod state;
pub mod upload;

use crate::config::AppConfig;
use anyhow::{Context, Result};
use files::Registry;
use nanoid::nanoid;
use state::AppState;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// 服务器运行时句柄：停机信号 + JoinHandle + 状态。
pub struct ServerHandle {
    shutdown_tx: Option<oneshot::Sender<()>>,
    join_handle: JoinHandle<()>,
    pub addr: SocketAddr,
    pub owner_token: String,
    pub state: Arc<AppState>,
}

impl ServerHandle {
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.join_handle.await;
    }
}

/// 启动 HTTP 服务，返回句柄。
///
/// `db_path` 为 SQLite 持久化文件路径；传 `None` 则降级为纯内存 Registry，
/// 服务照常运行但记录不会在重启后保留。
pub async fn start(config: AppConfig, db_path: Option<PathBuf>) -> Result<ServerHandle> {
    let port = config.port;
    let bind_ip: IpAddr = if config.bind_ipv6 {
        IpAddr::V6(Ipv6Addr::UNSPECIFIED) // [::]
    } else {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED) // 0.0.0.0
    };
    let addr = SocketAddr::new(bind_ip, port);

    // 防火墙规则（Windows 首次会触发 UAC 弹窗）
    firewall::ensure_rule(port);

    // 打开（或创建）SQLite 并加载历史记录、执行 reconcile
    let registry = Registry::open(db_path.as_deref());

    let owner_token = nanoid!(32);
    let state = AppState::new(config, owner_token.clone(), registry);
    let app = routes::build_router(state.clone());

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr} failed"))?;
    let actual_addr = listener.local_addr().unwrap_or(addr);

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let join_handle = tokio::spawn(async move {
        tracing::info!("HTTP server listening on {actual_addr}");
        let result = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
            tracing::info!("graceful shutdown signal received");
        })
        .await;
        if let Err(e) = result {
            tracing::error!("HTTP server error: {e}");
        }
    });

    Ok(ServerHandle {
        shutdown_tx: Some(shutdown_tx),
        join_handle,
        addr: actual_addr,
        owner_token,
        state,
    })
}
