use super::auth;
use super::files::{FileItem, TextItem};
use super::state::{now_secs, AppState, SyncEvent};
use super::upload::{UploadSession, DEFAULT_CHUNK_SIZE, MAX_CHUNK_SIZE, MAX_FILE_SIZE};
use axum::{
    body::{Body, Bytes},
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
use futures_util::{SinkExt, StreamExt};
use nanoid::nanoid;
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio_util::io::ReaderStream;
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
    files: Vec<FileItem>,
    texts: Vec<TextItem>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadResponse {
    uploaded: Vec<FileItem>,
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
        // 对外声明单文件上限（分片通道生效）
        max_upload_size: MAX_FILE_SIZE as usize,
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
    Json(body): Json<LoginBody>,
) -> Result<Json<LoginResp>, (StatusCode, String)> {
    // 未开启密码 → 直接签发临时 token（前端统一代码路径）
    if !state.password_required() {
        let token = auth::issue_token(&state.jwt_secret_snapshot(), "guest")
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
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
        return Err((StatusCode::UNAUTHORIZED, "密码错误".into()));
    }
    let token = auth::issue_token(&state.jwt_secret_snapshot(), "guest")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
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
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, (StatusCode, String)> {
    let upload_dir = state.config.read().upload_dir.clone();
    if upload_dir.as_os_str().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "uploadDir 未配置".into()));
    }
    fs::create_dir_all(&upload_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir: {e}")))?;

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
        state.registry.add_file(item.clone());
        state.broadcast(SyncEvent::FileAdded { file: item.clone() });
        uploaded.push(item);
    }

    Ok(Json(UploadResponse { uploaded }))
}

async fn download_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let item = state.registry.get_file(&id).ok_or(StatusCode::NOT_FOUND)?;

    let metadata = fs::metadata(&item.path).await.map_err(|_| StatusCode::NOT_FOUND)?;
    let total_size = metadata.len();

    // Parse Range: bytes=START-END
    let range = parse_range_header(headers.get(header::RANGE), total_size);

    let encoded_name = urlencoding::encode(&item.name);
    let content_disposition = format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        item.name.replace('"', ""),
        encoded_name
    );

    match range {
        Some((start, end)) if start <= end && end < total_size => {
            let length = end - start + 1;
            let mut file = fs::File::open(&item.path).await.map_err(|_| StatusCode::NOT_FOUND)?;
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let stream = ReaderStream::new(file.take(length));
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
            let file = fs::File::open(&item.path).await.map_err(|_| StatusCode::NOT_FOUND)?;
            let stream = ReaderStream::new(file);
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
    if let Some(item) = state.registry.remove_file(&id) {
        if let Some(parent) = item.path.parent() {
            let _ = fs::remove_dir_all(parent).await;
        } else {
            let _ = fs::remove_file(&item.path).await;
        }
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
) -> Result<Json<TextItem>, (StatusCode, String)> {
    if body.content.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "content 不能为空".into()));
    }
    if body.content.len() > 1024 * 1024 {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "文本过大（>1MB）".into()));
    }
    let item = TextItem {
        id: nanoid!(12),
        content: body.content,
        uploader_ip: client_ip(&addr),
        created_at: now_secs(),
    };
    state.registry.add_text(item.clone());
    state.broadcast(SyncEvent::TextAdded { text: item.clone() });
    Ok(Json(item))
}

async fn delete_text(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    require_owner(&headers, &state.owner_token)?;
    if state.registry.remove_text(&id).is_some() {
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
    file: FileItem,
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
    if body.size > MAX_FILE_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("文件过大，最大 {} GB", MAX_FILE_SIZE / 1024 / 1024 / 1024),
        ));
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

    // 续传匹配：同 name + size 已存在进行中的会话
    let sig = format!("{}::{}", safe_name, body.size);
    if let Some(existing) = state.uploads.find_by_signature(&sig) {
        // 校验 chunk_size 一致，否则放弃续传（极端情况）
        if existing.chunk_size == chunk_size && existing.chunk_count == chunk_count {
            return Ok(Json(InitUploadResp {
                upload_id: existing.id,
                chunk_size: existing.chunk_size,
                chunk_count: existing.chunk_count,
                uploaded: existing.uploaded,
                resumed: true,
            }));
        }
    }

    let upload_dir = state.config.read().upload_dir.clone();
    if upload_dir.as_os_str().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "uploadDir 未配置".into()));
    }
    let id = nanoid!(16);
    let tmp_dir = upload_dir.join(".tmp").join(&id);
    fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir tmp: {e}")))?;

    let session = UploadSession {
        id: id.clone(),
        name: safe_name,
        size: body.size,
        mime,
        chunk_size,
        chunk_count,
        uploader_ip: client_ip(&addr),
        created_at: now_secs(),
        uploaded: Vec::new(),
        tmp_dir,
    };
    state.uploads.insert(session.clone());
    Ok(Json(InitUploadResp {
        upload_id: id,
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
        let _ = fs::remove_dir_all(&s.tmp_dir).await;
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn upload_chunk(
    State(state): State<Arc<AppState>>,
    Path((id, index)): Path<(String, u32)>,
    body: Bytes,
) -> Result<Json<ChunkUploadResp>, (StatusCode, String)> {
    let session = state
        .uploads
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, "upload 不存在或已取消".into()))?;

    if index >= session.chunk_count {
        return Err((StatusCode::BAD_REQUEST, "index 越界".into()));
    }
    // 校验块体积
    let expected = if index + 1 == session.chunk_count {
        session.size - (session.chunk_size * index as u64)
    } else {
        session.chunk_size
    };
    if body.len() as u64 != expected {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("块大小不匹配：期望 {}，实际 {}", expected, body.len()),
        ));
    }
    if body.len() as u64 > MAX_CHUNK_SIZE {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "块过大".into()));
    }

    // 落盘到临时文件
    let path = session.chunk_path(index);
    let mut f = fs::File::create(&path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("create chunk: {e}")))?;
    f.write_all(&body)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("write chunk: {e}")))?;
    f.flush().await.ok();
    drop(f);

    let updated = state
        .uploads
        .mark_chunk(&id, index)
        .ok_or((StatusCode::NOT_FOUND, "upload 已被清理".into()))?;
    let complete = updated.is_complete();

    Ok(Json(ChunkUploadResp {
        received: index,
        uploaded: updated.uploaded,
        complete,
    }))
}

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

    let upload_dir = state.config.read().upload_dir.clone();
    let file_id = nanoid!(16);
    let file_dir = upload_dir.join(&file_id);
    fs::create_dir_all(&file_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir final: {e}")))?;
    let final_path = file_dir.join(&session.name);

    // 按 index 顺序拼接所有分片到目标文件
    let mut out = fs::File::create(&final_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("create final: {e}")))?;
    let mut total: u64 = 0;
    for i in 0..session.chunk_count {
        let path = session.chunk_path(i);
        let mut input = fs::File::open(&path).await.map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("open chunk {i}: {e}"))
        })?;
        let mut buf = vec![0u8; 1024 * 512];
        loop {
            let n = input.read(&mut buf).await.map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("read chunk {i}: {e}"))
            })?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).await.map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("write merge: {e}"))
            })?;
            total = total.saturating_add(n as u64);
        }
    }
    out.flush().await.ok();
    drop(out);

    if total != session.size {
        let _ = fs::remove_dir_all(&file_dir).await;
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("合并尺寸不一致：期望 {}，实际 {}", session.size, total),
        ));
    }

    // 清理临时目录 & 会话
    let _ = fs::remove_dir_all(&session.tmp_dir).await;
    state.uploads.remove(&id);

    let item = FileItem {
        id: file_id,
        name: session.name.clone(),
        size: session.size,
        mime: session.mime.clone(),
        uploader_ip: session.uploader_ip.clone(),
        created_at: now_secs(),
        path: final_path,
    };
    state.registry.add_file(item.clone());
    state.broadcast(SyncEvent::FileAdded { file: item.clone() });

    Ok(Json(CompleteUploadResp { file: item }))
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

    // 服务端推送循环
    let push_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if let Ok(text) = serde_json::to_string(&ev) {
                        if sender.send(Message::Text(text)).await.is_err() {
                            break;
                        }
                    }
                }
                // 接收者滞后，事件被覆盖：提示客户端全量重拉
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let _ = sender
                        .send(Message::Text("{\"type\":\"resync\"}".into()))
                        .await;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        // 主动关闭
        let _ = sender.close().await;
    });

    // 客户端消息循环（心跳/断开检测）
    let recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Close(_)) => break,
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Text(_)) | Ok(Message::Binary(_)) => {}
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

/// 去除文件名中的路径分隔符与非法字符，防止路径穿越
fn sanitize_filename(name: &str) -> String {
    let bad = ['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\0'];
    let cleaned: String = name.chars().map(|c| if bad.contains(&c) { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned.chars().take(200).collect()
    }
}
