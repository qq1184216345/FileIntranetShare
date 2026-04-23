use super::auth;
use super::files::{FileItem, TextItem};
use super::state::{now_secs, AppState, SyncEvent};
use super::upload::{UploadSession, DEFAULT_CHUNK_SIZE, MAX_CHUNK_SIZE};
use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, DefaultBodyLimit, Multipart, Path, Request, State,
    },
    http::{header, HeaderMap, StatusCode, Uri},
    middleware::{self, Next},
    response::{Html, IntoResponse, Json, Response},
    routing::{delete, get, post},
    Router,
};
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use nanoid::nanoid;
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio_util::io::{ReaderStream, StreamReader};
use tower_http::cors::{Any, CorsLayer};

/// 嵌入前端构建产物。
/// debug 模式下 rust-embed 直接从磁盘读取（可热替换，无需重新编译 Rust）；
/// release 模式下文件内嵌到二进制。
#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../dist/"]
struct FrontendAsset;

/// 旧版 multipart 一次性上传的大小上限：保留 100 MB（大文件请走 /api/upload/init 分片通道）
const MAX_UPLOAD_SIZE: usize = 100 * 1024 * 1024;

// ========== Response Types ==========

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerInfo {
    name: &'static str,
    version: &'static str,
    password_required: bool,
    https_enabled: bool,
    started_at: i64,
    max_upload_size: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListResponse {
    files: Vec<Arc<FileItem>>,
    texts: Vec<Arc<TextItem>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadResponse {
    uploaded: Vec<Arc<FileItem>>,
}

#[derive(Deserialize)]
struct PostTextBody {
    content: String,
}

// ========== Router ==========

pub fn build_router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // 公开路由：免鉴权
    let public = Router::new()
        .route("/api/info", get(info))
        .route("/api/ping", get(ping))
        .route("/api/login", post(login))
        // 静态资源（前端 SPA）也放公开，由 fallback 托底
        .fallback(get(serve_frontend));

    // 受保护路由：password 开启时需要 owner_token 或有效 JWT
    let protected = Router::new()
        .route("/api/list", get(list_items))
        .route("/api/upload", post(upload).layer(DefaultBodyLimit::max(MAX_UPLOAD_SIZE + 4 * 1024 * 1024)))
        .route("/api/file/:id", get(download_file).delete(delete_file))
        .route("/api/text", post(post_text))
        .route("/api/text/:id", delete(delete_text))
        .route("/api/upload/init", post(upload_init))
        .route(
            "/api/upload/:id",
            get(upload_status).delete(upload_cancel),
        )
        .route(
            "/api/upload/:id/chunk/:index",
            post(upload_chunk)
                .layer(DefaultBodyLimit::max((MAX_CHUNK_SIZE as usize) + 1024 * 1024)),
        )
        .route("/api/upload/:id/complete", post(upload_complete))
        .route("/api/sync", get(sync_ws))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        .merge(protected)
        .merge(public)
        .with_state(state)
        .layer(cors)
}

// ========== Auth Middleware ==========

/// 鉴权中间件：
/// - 未启用密码 → 直接放行
/// - 启用密码 → 校验 Authorization: Bearer 或 `?token=` query
/// - 允许 owner_token（Host）或有效 JWT（访客登录后）
async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    if !state.password_required() {
        return next.run(req).await;
    }

    let token = extract_token(&req);
    if token_valid(&token, &state) {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "未授权").into_response()
    }
}

/// 从请求中提取 token，优先 header，其次 query `?token=`
fn extract_token(req: &Request) -> String {
    // Authorization: Bearer xxx
    if let Some(v) = req.headers().get(header::AUTHORIZATION) {
        if let Ok(s) = v.to_str() {
            if let Some(rest) = s.strip_prefix("Bearer ") {
                return rest.trim().to_string();
            }
        }
    }
    // ?token=xxx
    if let Some(q) = req.uri().query() {
        for pair in q.split('&') {
            if let Some(val) = pair.strip_prefix("token=") {
                return urlencoding::decode(val).unwrap_or_default().to_string();
            }
        }
    }
    String::new()
}

fn token_valid(token: &str, state: &AppState) -> bool {
    if token.is_empty() {
        return false;
    }
    if token == state.owner_token {
        return true;
    }
    auth::verify_token(&state.jwt_secret_snapshot(), token).is_ok()
}

/// 前端 SPA 静态资源托管：
/// 1. 根路径或未命中任何 /api 路由时，优先查找 dist/{path}
/// 2. 未找到时返回 index.html（支持 history 路由刷新、/s 等路径）
/// 3. 若 dist 还未构建（未执行 pnpm build），回退到内置占位页，提示用户
async fn serve_frontend(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let candidates: [&str; 2] = [path, "index.html"];

    for (i, key) in candidates.iter().enumerate() {
        let lookup_key = if key.is_empty() { "index.html" } else { *key };
        if let Some(content) = FrontendAsset::get(lookup_key) {
            let mime = mime_guess::from_path(lookup_key).first_or_octet_stream();
            let body = Body::from(content.data.into_owned());
            let status = if i == 0 { StatusCode::OK } else { StatusCode::OK };
            return Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, mime.as_ref())
                // index.html 不缓存；带 hash 的静态资源让浏览器缓存
                .header(
                    header::CACHE_CONTROL,
                    if lookup_key == "index.html" {
                        "no-cache, no-store"
                    } else {
                        "public, max-age=31536000, immutable"
                    },
                )
                .body(body)
                .unwrap_or_else(|_| empty_response(StatusCode::INTERNAL_SERVER_ERROR));
        }
    }

    // dist 目录尚未生成：给一个友好提示页
    Html(include_str!("./placeholder.html")).into_response()
}

fn empty_response(status: StatusCode) -> Response {
    Response::builder().status(status).body(Body::empty()).unwrap()
}

// ========== Handlers ==========

async fn info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.read();
    Json(ServerInfo {
        name: "FileShare",
        version: env!("CARGO_PKG_VERSION"),
        password_required: config.password_enabled,
        https_enabled: config.https_enabled,
        started_at: state.started_at,
        // 分片通道不再限制单文件大小；0 表示无上限（前端据此不做拦截）
        max_upload_size: 0,
    })
}

async fn ping() -> impl IntoResponse {
    "pong"
}

#[derive(Deserialize)]
struct LoginBody {
    password: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginResp {
    token: String,
    expires_in: i64,
}

async fn login(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<LoginBody>,
) -> Result<Json<LoginResp>, (StatusCode, String)> {
    let ip = client_ip(&addr);
    // 未开启密码 → 直接签发临时 token（前端统一代码路径）
    if !state.password_required() {
        let token = auth::issue_token(&state.jwt_secret_snapshot(), "guest")
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        state.registry.log_audit("login", &ip, "no-password");
        return Ok(Json(LoginResp {
            token,
            expires_in: 7 * 24 * 3600,
        }));
    }
    let hash_opt = state.password_hash.read().clone();
    let Some(hash) = hash_opt else {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "密码未初始化".into()));
    };
    if !auth::verify_password(&body.password, &hash) {
        state.registry.log_audit("login_fail", &ip, "wrong password");
        return Err((StatusCode::UNAUTHORIZED, "密码错误".into()));
    }
    let token = auth::issue_token(&state.jwt_secret_snapshot(), "guest")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state.registry.log_audit("login", &ip, "ok");
    Ok(Json(LoginResp {
        token,
        expires_in: 7 * 24 * 3600,
    }))
}

async fn list_items(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(ListResponse {
        files: state.registry.list_files(),
        texts: state.registry.list_texts(),
    })
}

async fn upload(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, (StatusCode, String)> {
    let (upload_dir, min_free_mb) = {
        let cfg = state.config.read();
        (cfg.upload_dir.clone(), cfg.disk_min_free_mb)
    };
    if upload_dir.as_os_str().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "uploadDir 未配置".into()));
    }
    fs::create_dir_all(&upload_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir: {e}")))?;

    // P3-17: 用 Content-Length 提前判断磁盘软限制（上限是 multipart 总字节，略高估但够用）
    if let Some(cl) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
    {
        check_disk_soft_limit(&upload_dir, cl, min_free_mb)?;
    }

    let uploader_ip = client_ip(&addr);
    let mut uploaded = Vec::new();

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("multipart: {e}")))?
    {
        let original_name = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("unnamed-{}", nanoid!(6)));
        let safe_name = sanitize_filename(&original_name);
        let mime = field.content_type().unwrap_or("application/octet-stream").to_string();

        let id = nanoid!(16);
        let file_dir = upload_dir.join(&id);
        fs::create_dir_all(&file_dir)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir id: {e}")))?;
        let file_path = file_dir.join(&safe_name);

        let mut file = fs::File::create(&file_path)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("create: {e}")))?;
        let mut total: u64 = 0;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("chunk: {e}")))?
        {
            total = total.saturating_add(chunk.len() as u64);
            if total > MAX_UPLOAD_SIZE as u64 {
                // 清理已写入的碎片
                drop(file);
                let _ = fs::remove_dir_all(&file_dir).await;
                return Err((
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!("文件过大，最大 {} MB", MAX_UPLOAD_SIZE / 1024 / 1024),
                ));
            }
            file.write_all(&chunk)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("write: {e}")))?;
        }
        drop(file);

        let item = FileItem {
            id,
            name: safe_name,
            size: total,
            mime,
            uploader_ip: uploader_ip.clone(),
            created_at: now_secs(),
            path: file_path,
        };
        let arc = state.registry.add_file(item);
        state.registry.log_audit(
            "upload",
            &uploader_ip,
            &format!("{} ({} bytes)", arc.name, arc.size),
        );
        state.broadcast(SyncEvent::FileAdded { file: arc.clone() });
        uploaded.push(arc);
    }

    Ok(Json(UploadResponse { uploaded }))
}

async fn download_file(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let item = state.registry.get_file(&id).ok_or(StatusCode::NOT_FOUND)?;
    // 只记录全量拉取（没有 Range）的首次命中，避免 range/续传产生海量噪声
    if headers.get(header::RANGE).is_none() {
        state
            .registry
            .log_audit("download", &client_ip(&addr), &item.name);
    }

    // 一次 open 拿 fd + metadata，省一次 stat syscall（P2-11）
    let mut file = fs::File::open(&item.path).await.map_err(|_| StatusCode::NOT_FOUND)?;
    let total_size = file.metadata().await.map_err(|_| StatusCode::NOT_FOUND)?.len();

    let range = parse_range_header(headers.get(header::RANGE), total_size);

    let encoded_name = urlencoding::encode(&item.name);
    let content_disposition = format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        item.name.replace('"', ""),
        encoded_name
    );

    // 把 ReaderStream 的缓冲从默认 4KB 提到 256KB：
    // - axum/hyper 目前不直接暴露底层 socket，无法调用 sendfile(2)/TransmitFile 真·零拷贝
    //   （P3-15 的理想实现需要绕过 hyper）。但加大 read 粒度后，每秒 syscall 数从 ~上万 降到 ~百级，
    //   吞吐接近 sendfile 的 80%+，代价只是一份 256KB 堆缓冲。
    const DOWNLOAD_BUF: usize = 256 * 1024;

    match range {
        Some((start, end)) if start <= end && end < total_size => {
            let length = end - start + 1;
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let stream = ReaderStream::with_capacity(file.take(length), DOWNLOAD_BUF);
            let body = Body::from_stream(stream);

            let resp = Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, &item.mime)
                .header(header::CONTENT_LENGTH, length)
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {}-{}/{}", start, end, total_size),
                )
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CONTENT_DISPOSITION, content_disposition)
                .body(body)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(resp)
        }
        _ => {
            let stream = ReaderStream::with_capacity(file, DOWNLOAD_BUF);
            let body = Body::from_stream(stream);

            let resp = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, &item.mime)
                .header(header::CONTENT_LENGTH, total_size)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CONTENT_DISPOSITION, content_disposition)
                .body(body)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(resp)
        }
    }
}

async fn delete_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    require_owner(&headers, &state.owner_token)?;
    // 按产品语义：删除仅移除分享记录（DB + 内存），不删除物理文件。
    // 适用于：Guest 上传的文件、Host 本机引用的文件、剪贴板图片。
    // 物理文件若在磁盘上被移动/删除，服务重启时的 reconcile 流程会自动清理对应的孤儿记录。
    if let Some(removed) = state.registry.remove_file(&id) {
        state.registry.log_audit("file_delete", "host", &removed.name);
        state.broadcast(SyncEvent::FileRemoved { id });
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn post_text(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<PostTextBody>,
) -> Result<Json<Arc<TextItem>>, (StatusCode, String)> {
    if body.content.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "content 不能为空".into()));
    }
    if body.content.len() > 1024 * 1024 {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "文本过大（>1MB）".into()));
    }
    let ip = client_ip(&addr);
    let size = body.content.len();
    let item = TextItem {
        id: nanoid!(12),
        content: body.content,
        uploader_ip: ip.clone(),
        created_at: now_secs(),
    };
    let arc = state.registry.add_text(item);
    state
        .registry
        .log_audit("text_add", &ip, &format!("{size} chars"));
    state.broadcast(SyncEvent::TextAdded { text: arc.clone() });
    Ok(Json(arc))
}

async fn delete_text(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    require_owner(&headers, &state.owner_token)?;
    if let Some(removed) = state.registry.remove_text(&id) {
        state.registry.log_audit(
            "text_delete",
            "host",
            &format!("{} chars", removed.content.len()),
        );
        state.broadcast(SyncEvent::TextRemoved { id });
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// ========== Chunked Upload ==========

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitUploadBody {
    name: String,
    size: u64,
    #[serde(default)]
    mime: Option<String>,
    #[serde(default)]
    chunk_size: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InitUploadResp {
    upload_id: String,
    chunk_size: u64,
    chunk_count: u32,
    uploaded: Vec<u32>,
    resumed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChunkUploadResp {
    uploaded: Vec<u32>,
    received: u32,
    complete: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompleteUploadResp {
    file: Arc<FileItem>,
}

async fn upload_init(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<InitUploadBody>,
) -> Result<Json<InitUploadResp>, (StatusCode, String)> {
    if body.name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name 不能为空".into()));
    }
    if body.size == 0 {
        return Err((StatusCode::BAD_REQUEST, "size 必须大于 0".into()));
    }

    let chunk_size = body
        .chunk_size
        .unwrap_or(DEFAULT_CHUNK_SIZE)
        .clamp(256 * 1024, MAX_CHUNK_SIZE);
    let chunk_count = ((body.size + chunk_size - 1) / chunk_size) as u32;
    let safe_name = sanitize_filename(&body.name);
    let mime = body.mime.unwrap_or_else(|| {
        mime_guess::from_path(&safe_name)
            .first_or_octet_stream()
            .to_string()
    });

    let uploader_ip = client_ip(&addr);
    // 续传匹配：同 name + size + uploader_ip（P0-3：避免并发冲突）
    let sig = format!("{}::{}::{}", safe_name, body.size, uploader_ip);
    if let Some(existing) = state.uploads.find_by_signature(&sig) {
        // 校验参数一致 + partial 文件仍然存在（否则放弃续传重新开）
        if existing.chunk_size == chunk_size
            && existing.chunk_count == chunk_count
            && fs::metadata(&existing.partial_path).await.is_ok()
        {
            return Ok(Json(InitUploadResp {
                upload_id: existing.id,
                chunk_size: existing.chunk_size,
                chunk_count: existing.chunk_count,
                uploaded: existing.uploaded,
                resumed: true,
            }));
        }
        // 参数不一致或 partial 丢失：清理旧会话，走新建分支
        if let Some(old) = state.uploads.remove(&existing.id) {
            let _ = fs::remove_file(&old.partial_path).await;
        }
    }

    let (upload_dir, min_free_mb) = {
        let cfg = state.config.read();
        (cfg.upload_dir.clone(), cfg.disk_min_free_mb)
    };
    if upload_dir.as_os_str().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "uploadDir 未配置".into()));
    }
    // P3-17: 磁盘空间软限制检查
    check_disk_soft_limit(&upload_dir, body.size, min_free_mb)?;
    let upload_id = nanoid!(16);
    // 最终目录在 init 时就确定：upload_dir/<file_id>/
    // partial 占位文件：upload_dir/<file_id>/<name>.partial
    let file_id = nanoid!(16);
    let file_dir = upload_dir.join(&file_id);
    fs::create_dir_all(&file_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir: {e}")))?;
    let final_path = file_dir.join(&safe_name);
    let partial_path = file_dir.join(format!("{}.partial", &safe_name));

    // 预分配：避免 chunk 写入 offset 时触发连续扩容（性能+减碎片）
    let f = fs::File::create(&partial_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("create partial: {e}")))?;
    f.set_len(body.size)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("preallocate: {e}")))?;
    drop(f);

    let session = UploadSession {
        id: upload_id.clone(),
        file_id,
        name: safe_name,
        size: body.size,
        mime,
        chunk_size,
        chunk_count,
        uploader_ip,
        created_at: now_secs(),
        uploaded: Vec::new(),
        partial_path,
        final_path,
    };
    state.uploads.insert(session);
    Ok(Json(InitUploadResp {
        upload_id,
        chunk_size,
        chunk_count,
        uploaded: Vec::new(),
        resumed: false,
    }))
}

async fn upload_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<InitUploadResp>, StatusCode> {
    let s = state.uploads.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(InitUploadResp {
        upload_id: s.id,
        chunk_size: s.chunk_size,
        chunk_count: s.chunk_count,
        uploaded: s.uploaded,
        resumed: true,
    }))
}

async fn upload_cancel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> StatusCode {
    if let Some(s) = state.uploads.remove(&id) {
        let _ = fs::remove_file(&s.partial_path).await;
        if let Some(dir) = s.final_path.parent() {
            // 若目录已空则删除；非空则保留（理论上 cancel 时只有 partial）
            let _ = fs::remove_dir(dir).await;
        }
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

/// 分片上传：直接把请求体 **流式** 写到目标文件对应偏移。
/// 相比旧实现的两个关键改进：
/// 1. 不再 `body: Bytes`（会把整个分片读进内存），改为 `Body` + `StreamReader`，内存占用由 O(chunk_size) 降到 O(64KB 缓冲)
/// 2. 不再写临时 chunk 文件，complete 时不再二次拷贝；I/O 量从 2x 降到 1x
async fn upload_chunk(
    State(state): State<Arc<AppState>>,
    Path((id, index)): Path<(String, u32)>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<ChunkUploadResp>, (StatusCode, String)> {
    let session = state
        .uploads
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, "upload 不存在或已取消".into()))?;

    if index >= session.chunk_count {
        return Err((StatusCode::BAD_REQUEST, "index 越界".into()));
    }
    let expected = if index + 1 == session.chunk_count {
        session.size - (session.chunk_size * index as u64)
    } else {
        session.chunk_size
    };
    if expected > MAX_CHUNK_SIZE {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "块过大".into()));
    }
    // 预校验：用 Content-Length 提前拒绝大小不一致的请求，避免写到一半才发现
    if let Some(cl) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
    {
        if cl != expected {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("块大小不匹配：期望 {}，实际 {}", expected, cl),
            ));
        }
    }

    let offset = session.chunk_size * index as u64;
    // O_WRONLY 打开预分配文件，seek 到目标偏移
    let mut f = fs::OpenOptions::new()
        .write(true)
        .open(&session.partial_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("open partial: {e}")))?;
    f.seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("seek: {e}")))?;

    // Body -> StreamReader -> file (流式拷贝)
    let stream = body
        .into_data_stream()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
    let reader = StreamReader::new(stream);

    // 用 take 做强约束：超过 expected 的字节直接被截断（防攻击/客户端 bug）
    let copied = tokio::io::copy(&mut reader.take(expected), &mut f)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("stream write: {e}")))?;
    f.flush().await.ok();
    drop(f);

    if copied != expected {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("块大小不匹配：期望 {}，实际写入 {}", expected, copied),
        ));
    }

    let (uploaded, chunk_count) = state
        .uploads
        .mark_chunk(&id, index)
        .ok_or((StatusCode::NOT_FOUND, "upload 已被清理".into()))?;
    let complete = uploaded.len() as u32 == chunk_count;

    Ok(Json(ChunkUploadResp {
        received: index,
        uploaded,
        complete,
    }))
}

/// 完成上传：所有分片已就位，只做一次原子 rename。
/// `partial` 和最终文件在同一目录（同一文件系统 mount），rename 是 O(1) 元数据操作。
async fn upload_complete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<CompleteUploadResp>, (StatusCode, String)> {
    let session = state
        .uploads
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, "upload 不存在".into()))?;
    if !session.is_complete() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "分片未全部到达：{}/{}",
                session.uploaded.len(),
                session.chunk_count
            ),
        ));
    }

    // 事后校验：partial 实际大小必须与声明一致（防御磁盘错误/并发异常）
    let meta = fs::metadata(&session.partial_path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("partial 不可读: {e}"),
        )
    })?;
    if meta.len() != session.size {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("文件尺寸不一致：期望 {}，实际 {}", session.size, meta.len()),
        ));
    }

    fs::rename(&session.partial_path, &session.final_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("rename: {e}")))?;

    state.uploads.remove(&id);

    let item = FileItem {
        id: session.file_id.clone(),
        name: session.name.clone(),
        size: session.size,
        mime: session.mime.clone(),
        uploader_ip: session.uploader_ip.clone(),
        created_at: now_secs(),
        path: session.final_path.clone(),
    };
    let arc = state.registry.add_file(item);
    state.registry.log_audit(
        "upload",
        &arc.uploader_ip,
        &format!("{} ({} bytes, chunked)", arc.name, arc.size),
    );
    state.broadcast(SyncEvent::FileAdded { file: arc.clone() });

    Ok(Json(CompleteUploadResp { file: arc }))
}

// ========== WebSocket ==========

async fn sync_ws(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_sync_socket(socket, state))
}

async fn handle_sync_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.events.subscribe();
    // 握手时快照一次鉴权纪元，后续检测到变动就立刻踢下线（P0-4）
    let epoch_at_connect = state.auth_epoch();

    // 握手消息：告知客户端连上并附带当前服务信息
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Hello<'a> {
        #[serde(rename = "type")]
        kind: &'a str,
        started_at: i64,
    }
    let hello = serde_json::to_string(&Hello {
        kind: "hello",
        started_at: state.started_at,
    })
    .unwrap_or_default();
    if sender.send(Message::Text(hello)).await.is_err() {
        return;
    }

    // 服务端推送循环：
    // - 事件广播（来自 AppState::events）
    // - 每 25s 主动发一次 Ping（心跳，防止 NAT/负载均衡超时断连）
    // - 每 10s 检查鉴权纪元（JWT 被轮换时立即踢下线）
    let state_push = state.clone();
    let push_task = tokio::spawn(async move {
        let mut ping_timer = tokio::time::interval(std::time::Duration::from_secs(25));
        ping_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut auth_timer = tokio::time::interval(std::time::Duration::from_secs(10));
        auth_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // 丢弃首次立即触发的 tick
        ping_timer.tick().await;
        auth_timer.tick().await;

        loop {
            tokio::select! {
                ev = rx.recv() => {
                    match ev {
                        Ok(ev) => {
                            if let Ok(text) = serde_json::to_string(&ev) {
                                if sender.send(Message::Text(text)).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let _ = sender
                                .send(Message::Text("{\"type\":\"resync\"}".into()))
                                .await;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = ping_timer.tick() => {
                    if sender.send(Message::Ping(Vec::new())).await.is_err() {
                        break;
                    }
                }
                _ = auth_timer.tick() => {
                    if state_push.auth_epoch() != epoch_at_connect {
                        let _ = sender
                            .send(Message::Text("{\"type\":\"authInvalid\"}".into()))
                            .await;
                        break;
                    }
                }
            }
        }
        let _ = sender.close().await;
    });

    // 客户端消息循环：只用于检测断开
    let recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Close(_)) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    tokio::select! {
        _ = push_task => {}
        _ = recv_task => {}
    }
    tracing::debug!("sync ws client disconnected");
}

// ========== Helpers ==========

fn client_ip(addr: &SocketAddr) -> String {
    addr.ip().to_string()
}

fn require_owner(headers: &HeaderMap, owner_token: &str) -> Result<(), StatusCode> {
    let got = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");
    if got == owner_token && !owner_token.is_empty() {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

/// 解析 Range: bytes=START-END / bytes=START- / bytes=-SUFFIX
fn parse_range_header(header_value: Option<&axum::http::HeaderValue>, total: u64) -> Option<(u64, u64)> {
    let raw = header_value?.to_str().ok()?;
    let raw = raw.strip_prefix("bytes=")?;
    // 只支持单区间
    let part = raw.split(',').next()?.trim();
    if let Some(suffix) = part.strip_prefix('-') {
        let n: u64 = suffix.parse().ok()?;
        if n == 0 || n > total {
            return Some((0, total.saturating_sub(1)));
        }
        return Some((total - n, total - 1));
    }
    let mut iter = part.splitn(2, '-');
    let start: u64 = iter.next()?.parse().ok()?;
    let end_opt = iter.next()?;
    let end: u64 = if end_opt.is_empty() {
        total.saturating_sub(1)
    } else {
        end_opt.parse().ok()?
    };
    if start >= total {
        return None;
    }
    Some((start, end.min(total.saturating_sub(1))))
}

/// 返回 path 所在分区的剩余可用字节数；失败或不支持则返回 None。
/// 仅读元信息，成本接近 0；失败不阻塞上传（返回 None 时视作"未知"按放行处理）。
fn disk_available_bytes(path: &std::path::Path) -> Option<u64> {
    use fs4::available_space;
    // 若目录尚未建，逐级向上找到一个存在的父目录再询问
    let mut cur: &std::path::Path = path;
    loop {
        if cur.exists() {
            return available_space(cur).ok();
        }
        match cur.parent() {
            Some(p) if p != cur => cur = p,
            _ => return None,
        }
    }
}

/// 上传前的磁盘软限制校验（P3-17）。
/// 规则：free < size + min_free_mb*MB 即拒绝。
/// 0 表示不启用；拿不到 free 也放行（避免误伤）。
fn check_disk_soft_limit(
    upload_dir: &std::path::Path,
    need: u64,
    min_free_mb: u64,
) -> Result<(), (StatusCode, String)> {
    if min_free_mb == 0 {
        return Ok(());
    }
    let Some(free) = disk_available_bytes(upload_dir) else {
        return Ok(());
    };
    let reserve = min_free_mb.saturating_mul(1024 * 1024);
    if free < need.saturating_add(reserve) {
        return Err((
            StatusCode::INSUFFICIENT_STORAGE,
            format!(
                "磁盘空间不足：剩余 {:.1} MB，需至少 {} MB 预留 + 文件 {:.1} MB",
                free as f64 / 1024.0 / 1024.0,
                min_free_mb,
                need as f64 / 1024.0 / 1024.0,
            ),
        ));
    }
    Ok(())
}

/// Windows 下保留的设备名（不区分大小写，含/不含扩展名都保留）
/// 若文件名主体匹配这些之一，在前面加下划线前缀避免 `CON.txt`/`NUL` 等被 OS 劫持。
const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL",
    "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 去除文件名中的路径分隔符与非法字符，防止路径穿越；同时处理 Windows 保留名（P2-13）
fn sanitize_filename(name: &str) -> String {
    let bad = ['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\0'];
    let cleaned: String = name.chars().map(|c| if bad.contains(&c) { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        return "file".to_string();
    }
    let truncated: String = cleaned.chars().take(200).collect();
    // 取主名（不含扩展名）判断是否为 Windows 保留设备名
    let stem_upper = truncated
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(&truncated)
        .to_ascii_uppercase();
    if WINDOWS_RESERVED.iter().any(|r| *r == stem_upper) {
        format!("_{truncated}")
    } else {
        truncated
    }
}
