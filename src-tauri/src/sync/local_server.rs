// Axum's Response is a large struct, and every handler answers with it as
// the Err side; boxing it through every signature would cost more reading
// time than the lint buys.
#![allow(clippy::result_large_err)]

//! The sync server that ships inside the app.
//!
//! Donut Free is a local product: nothing of it lives in a vendor cloud, and
//! requiring a user to stand up the NestJS+MinIO compose stack
//! (`scripts/donut-sync`) before two copies of the app can share profiles
//! made "sync" a feature nobody could switch on. This module is a drop-in
//! replacement for that stack, served from the app's own process on loopback
//! and backed by a plain folder on disk: same routes, same request/response
//! shapes, same key layout (`profiles/{id}/…`, `tombstones/{kind}/{id}.json`,
//! `snapshots/…`) that `SyncClient` (`super::client`) already speaks.
//!
//! Wire contract, and where each piece comes from here:
//!
//! * `POST /v1/objects/{stat,presign-upload,presign-download,delete,list,
//!   presign-upload-batch,presign-download-batch,delete-prefix,
//!   snapshots/list,snapshots/create,snapshots/delete,snapshots/restore-files}`
//!   plus `GET /v1/objects/subscribe` (SSE) — the surface the reference
//!   server exposes and `tests/sync_e2e.rs` exercises, guarded by the same
//!   `Authorization: Bearer` header.
//! * Presigned URLs are HMAC-SHA256 over `method\nkey\nexpiry`, carried as
//!   S3-style `X-Amz-*` query parameters. The client treats the URL as
//!   opaque (it PUTs/GETs whatever the server handed it), so a compact
//!   signed form is wire-compatible without reimplementing SigV4.
//! * `x-amz-meta-*` headers on the presigned PUT are stored beside the
//!   object and echoed by `stat`, because sync conflict resolution reads
//!   `updated-at` from `stat` without downloading the body
//!   (`engine::UPDATED_AT_META_KEY`).
//!
//! Storage layout under the server root (default `<data>/sync-local`):
//!
//! * `objects/<key>` — object bodies, one file per key;
//! * `meta/<key>.json` — sidecar `{contentType, metadata, lastModified}`, a
//!   parallel tree so no key can ever collide with a sidecar name;
//! * `server.json` — the bearer token and URL signing key, generated once.
//!   A loopback-only server whose data already sits on the same disk gains
//!   nothing from a stronger store for this file: anything that can read it
//!   can read the objects directly.
//!
//! Snapshot copies live inside the object tree (`snapshots/{root}{id}/…`)
//! because the client downloads `snapshots/{root}{id}/manifest.json` through
//! the ordinary presigned-download path.

use axum::{
  body::Bytes,
  extract::{DefaultBodyLimit, Path, RawQuery, State},
  http::{HeaderMap, StatusCode},
  middleware::{self, Next},
  response::{
    sse::{Event, KeepAlive, KeepAliveStream, Sse},
    IntoResponse, Json, Response,
  },
  routing::{get, post},
  Router,
};
use ring::hmac;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};

use super::types::{
  DeletePrefixRequest, DeletePrefixResponse, DeleteRequest, DeleteResponse, ListObject,
  ListRequest, ListResponse, PresignDownloadBatchItemResponse, PresignDownloadBatchRequest,
  PresignDownloadBatchResponse, PresignDownloadRequest, PresignDownloadResponse,
  PresignUploadBatchItem, PresignUploadBatchItemResponse, PresignUploadBatchRequest,
  PresignUploadBatchResponse, PresignUploadRequest, PresignUploadResponse, StatRequest,
  StatResponse,
};

/// Presigned URLs live one hour by default, matching what `SyncClient` asks
/// for on every presign call.
const DEFAULT_EXPIRES_SECS: u64 = 3600;
/// S3 caps presigned URLs at seven days; do the same so a URL can never
/// outlive any reasonable rotation of the signing key.
const MAX_EXPIRES_SECS: u64 = 7 * 24 * 3600;
/// One object may be at most this large. Profile bundles run to tens of
/// megabytes; a gigabyte leaves room for the tarball of a heavy profile
/// while still bounding a rogue request.
const MAX_OBJECT_BYTES: usize = 1024 * 1024 * 1024;
/// Keys longer than this are refused before they touch the filesystem.
const MAX_KEY_LEN: usize = 1024;
/// The reference server keeps the newest three restore points of each kind.
const SNAPSHOTS_KEPT_PER_KIND: usize = 3;
/// Marker file inside a snapshot copy carrying its restore-point metadata.
/// Fixed name with a leading dot, so no profile file can be mistaken for it.
const SNAPSHOT_MARKER: &str = ".snapshot.json";
/// How often the subscription stream emits a keep-alive comment. The client
/// drops an SSE read that produces no bytes for 60 s, so this must stay
/// comfortably below that.
const SSE_KEEPALIVE_SECS: u64 = 20;

// ---------------------------------------------------------------------------
// Wire types the reference server owns but the client only speaks
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotsListRequest {
  profile_root: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotsCreateRequest {
  profile_root: String,
  kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotsTargetRequest {
  profile_root: String,
  id: String,
}

/// The restore-point record, field-for-field what
/// `super::restore_points::RestorePoint` deserializes.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RestorePointDto {
  id: String,
  created_at: String,
  kind: String,
  files: u64,
  bytes: u64,
}

/// What a sidecar records for one stored object.
#[derive(Debug, Serialize, Deserialize, Default)]
struct ObjectMeta {
  #[serde(
    rename = "contentType",
    default,
    skip_serializing_if = "Option::is_none"
  )]
  content_type: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  metadata: Option<HashMap<String, String>>,
  #[serde(rename = "lastModified", default)]
  last_modified: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ServerConfig {
  token: String,
  signing_key: String,
}

// ---------------------------------------------------------------------------
// Server lifecycle
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ServerState {
  /// Root of the `objects/` tree.
  objects: PathBuf,
  /// Root of the parallel sidecar tree.
  meta: PathBuf,
  token: String,
  signing_key: hmac::Key,
  /// What presigned URLs are built from: the loopback origin of this
  /// listener.
  public_base: String,
  /// One-sync-at-a-time for everything that changes the store. The sync
  /// engine drives several profiles in parallel; tmp-file, rename, list and
  /// prefix-delete steps interleaving across those runs is how objects go
  /// missing, so mutations serialize on this lock. Reads stay lock-free.
  writes: std::sync::Arc<std::sync::Mutex<()>>,
}

/// Run a store mutation alone. Blocking-lock scope is short (filesystem
/// work, no awaits inside), so a std mutex is right.
fn locked_write<T>(state: &ServerState, work: impl FnOnce() -> T) -> T {
  let _guard = state
    .writes
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner());
  work()
}

pub struct LocalSyncHandle {
  base_url: String,
  token: String,
  shutdown_tx: mpsc::Sender<()>,
  task: tokio::task::JoinHandle<()>,
}

impl LocalSyncHandle {
  pub fn base_url(&self) -> &str {
    &self.base_url
  }

  pub fn token(&self) -> &str {
    &self.token
  }

  /// Retire the server: close the listener and end the serve task.
  pub async fn stop(self) {
    let _ = self.shutdown_tx.send(()).await;
    self.task.abort();
  }
}

/// The app-wide instance, started once and left running for the lifetime of
/// the process: the store folder and token must be stable across every
/// engine that points at the server.
static RUNNING: Mutex<Option<LocalSyncHandle>> = Mutex::const_new(None);

/// Where the embedded server keeps its config and store.
fn default_root() -> PathBuf {
  crate::app_dirs::data_dir().join("sync-local")
}

/// Start (or return the already running) embedded sync server, backed by
/// `<data>/sync-local`. Returns the base URL to point `SyncClient` at and
/// the bearer token it must send.
pub async fn ensure_started() -> Result<(String, String), String> {
  let mut guard = RUNNING.lock().await;
  if let Some(handle) = guard.as_ref() {
    return Ok((handle.base_url().to_string(), handle.token().to_string()));
  }
  let handle = start(&default_root(), None).await?;
  let info = (handle.base_url().to_string(), handle.token().to_string());
  *guard = Some(handle);
  Ok(info)
}

/// Stop the app-wide instance (tests and shutdown paths).
pub async fn stop_local() {
  let mut guard = RUNNING.lock().await;
  if let Some(handle) = guard.take() {
    handle.stop().await;
  }
}

/// The bearer token persisted for the app-wide server, without starting it.
/// Subscription reconnects need the token alone and must not spin a server
/// up just to read it back.
pub fn stored_token() -> Result<Option<String>, String> {
  Ok(read_config(&default_root())?.map(|config| config.token))
}

/// Boot a standalone server on a random loopback port.
///
/// `fixed_token` pins the bearer token (the e2e harness passes the constant
/// its tests send); `None` loads or generates the persisted one.
pub async fn start(root: &FsPath, fixed_token: Option<&str>) -> Result<LocalSyncHandle, String> {
  let objects = root.join("objects");
  let meta = root.join("meta");
  std::fs::create_dir_all(&objects).map_err(|e| format!("sync store: {e}"))?;
  std::fs::create_dir_all(&meta).map_err(|e| format!("sync meta store: {e}"))?;

  let config = load_or_create_config(root, fixed_token)?;
  let signing_key = hmac::Key::new(hmac::HMAC_SHA256, config.signing_key.as_bytes());

  let listener = TcpListener::bind("127.0.0.1:0")
    .await
    .map_err(|e| format!("sync server bind: {e}"))?;
  let port = listener
    .local_addr()
    .map_err(|e| format!("sync server address: {e}"))?
    .port();

  let state = ServerState {
    objects,
    meta,
    token: config.token.clone(),
    signing_key,
    public_base: format!("http://127.0.0.1:{port}"),
    writes: std::sync::Arc::new(std::sync::Mutex::new(())),
  };

  let app = router(state.clone());

  let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
  let task = tokio::spawn(async move {
    let server = axum::serve(listener, app);
    tokio::select! {
      _ = server => {},
      _ = shutdown_rx.recv() => {},
    }
  });

  Ok(LocalSyncHandle {
    base_url: format!("http://127.0.0.1:{port}"),
    token: config.token,
    shutdown_tx,
    task,
  })
}

fn read_config(root: &FsPath) -> Result<Option<ServerConfig>, String> {
  match std::fs::read_to_string(root.join("server.json")) {
    Ok(text) => Ok(serde_json::from_str(&text).ok()),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
    Err(e) => Err(format!("sync server config: {e}")),
  }
}

fn random_hex(bytes: usize) -> String {
  use rand::Rng;
  let mut buf = vec![0u8; bytes];
  rand::rng().fill_bytes(&mut buf);
  buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn write_config(root: &FsPath, config: &ServerConfig) -> Result<(), String> {
  let path = root.join("server.json");
  let tmp = root.join(format!("server.json.{}", std::process::id()));
  let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
  std::fs::write(&tmp, text).map_err(|e| format!("sync server config write: {e}"))?;
  std::fs::rename(&tmp, &path).map_err(|e| format!("sync server config replace: {e}"))?;
  Ok(())
}

/// Load the persisted config, or create one. A fixed token (e2e harness)
/// overwrites whatever was stored; a generated token is 64 hex chars.
fn load_or_create_config(root: &FsPath, fixed_token: Option<&str>) -> Result<ServerConfig, String> {
  if let Some(mut config) = read_config(root)? {
    if config.signing_key.len() == 64 {
      if let Some(token) = fixed_token {
        if config.token != token {
          config.token = token.to_string();
          write_config(root, &config)?;
        }
      }
      if config.token.len() >= 24 {
        return Ok(config);
      }
    }
  }
  let config = ServerConfig {
    token: fixed_token
      .map(|t| t.to_string())
      .unwrap_or_else(|| random_hex(32)),
    signing_key: random_hex(32),
  };
  write_config(root, &config)?;
  Ok(config)
}

fn router(state: ServerState) -> Router {
  let objects = Router::new()
    .route("/stat", post(stat))
    .route("/presign-upload", post(presign_upload))
    .route("/presign-download", post(presign_download))
    .route("/presign-upload-batch", post(presign_upload_batch))
    .route("/presign-download-batch", post(presign_download_batch))
    .route("/delete", post(delete))
    .route("/delete-prefix", post(delete_prefix))
    .route("/list", post(list))
    .route("/snapshots/list", post(snapshots_list))
    .route("/snapshots/create", post(snapshots_create))
    .route("/snapshots/delete", post(snapshots_delete))
    .route("/snapshots/restore-files", post(snapshots_restore_files))
    .route("/subscribe", get(subscribe));

  let storage = get(storage_get)
    .put(storage_put)
    .layer(DefaultBodyLimit::max(MAX_OBJECT_BYTES));

  Router::new()
    .route("/health", get(health))
    .route("/readyz", get(readyz))
    .nest("/v1/objects", objects)
    .route("/storage/{*key}", storage)
    .layer(middleware::from_fn_with_state(
      state.clone(),
      auth_middleware,
    ))
    .with_state(state)
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// Constant-time byte-string equality, built on `ring`'s tag comparison so
/// no prefix of the secret is ever confirmed early.
fn constant_time_eq(a: &str, b: &str) -> bool {
  let key = hmac::Key::new(hmac::HMAC_SHA256, &[]);
  let tag = hmac::sign(&key, a.as_bytes());
  hmac::verify(&key, b.as_bytes(), tag.as_ref()).is_ok()
}

/// Bearer-token guard for the object API, answering in the shape the NestJS
/// reference does: `401 {"statusCode":401,"message":"Unauthorized"}`.
///
/// `/health`, `/readyz` and `/storage/*` are exempt: the first two are
/// pre-flight probes, storage requests authenticate by their URL signature.
async fn auth_middleware(
  State(state): State<ServerState>,
  request: axum::extract::Request,
  next: Next,
) -> Response {
  if request.uri().path().starts_with("/v1/objects") {
    let presented = request
      .headers()
      .get(axum::http::header::AUTHORIZATION)
      .and_then(|value| value.to_str().ok())
      .and_then(|value| value.strip_prefix("Bearer "));
    let authorized = presented.is_some_and(|token| constant_time_eq(token, &state.token));
    if !authorized {
      return unauthorized();
    }
  }
  next.run(request).await
}

fn unauthorized() -> Response {
  (
    StatusCode::UNAUTHORIZED,
    Json(serde_json::json!({ "statusCode": 401, "message": "Unauthorized" })),
  )
    .into_response()
}

fn bad_request(message: &str) -> Response {
  (
    StatusCode::BAD_REQUEST,
    Json(serde_json::json!({ "statusCode": 400, "message": message })),
  )
    .into_response()
}

fn forbidden() -> Response {
  (
    StatusCode::FORBIDDEN,
    Json(serde_json::json!({ "statusCode": 403, "message": "InvalidSignature" })),
  )
    .into_response()
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

/// Map an object key to a path under a store root, refusing anything that
/// could escape it. Per path component: no empty parts, no `.`/`..`, no
/// separators (`\`, NUL; `/` was the split), no Windows drive prefix.
fn object_path(root: &FsPath, key: &str) -> Option<PathBuf> {
  if key.is_empty() || key.len() > MAX_KEY_LEN || key.starts_with('/') {
    return None;
  }
  let mut path = root.to_path_buf();
  for part in key.split('/') {
    if !crate::app_dirs::is_plain_path_part(part) {
      return None;
    }
    path.push(part);
  }
  Some(path)
}

fn sidecar_path(meta_root: &FsPath, key: &str) -> Option<PathBuf> {
  let mut path = object_path(meta_root, key)?;
  path.set_extension("json");
  Some(path)
}

/// The key a store-relative file path spells, or `None` when any component
/// is not plain UTF-8.
fn key_for_path(objects_root: &FsPath, path: &FsPath) -> Option<String> {
  let rel = path.strip_prefix(objects_root).ok()?;
  let mut parts = Vec::new();
  for part in rel.components() {
    let std::path::Component::Normal(os) = part else {
      return None;
    };
    parts.push(os.to_str()?);
  }
  Some(parts.join("/"))
}

// ---------------------------------------------------------------------------
// Object helpers
// ---------------------------------------------------------------------------

fn now_rfc3339() -> String {
  chrono::Utc::now().to_rfc3339()
}

fn write_atomic(path: &FsPath, bytes: &[u8]) -> Result<(), String> {
  let parent = path
    .parent()
    .ok_or_else(|| "object path has no parent".to_string())?;
  std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
  let tmp = parent.join(format!(
    ".{}.{}.tmp",
    std::process::id(),
    blake3::hash(bytes).to_hex()
  ));
  std::fs::write(&tmp, bytes).map_err(|e| format!("write: {e}"))?;
  std::fs::rename(&tmp, path).map_err(|e| format!("replace: {e}"))?;
  Ok(())
}

fn read_sidecar(state: &ServerState, key: &str) -> ObjectMeta {
  sidecar_path(&state.meta, key)
    .and_then(|p| std::fs::read_to_string(p).ok())
    .and_then(|text| serde_json::from_str(&text).ok())
    .unwrap_or_default()
}

fn store_object(
  state: &ServerState,
  key: &str,
  body: &[u8],
  content_type: Option<String>,
  metadata: HashMap<String, String>,
) -> Result<(), String> {
  let path = object_path(&state.objects, key).ok_or("invalid key")?;
  write_atomic(&path, body)?;
  let meta = ObjectMeta {
    content_type,
    metadata: (!metadata.is_empty()).then_some(metadata),
    last_modified: now_rfc3339(),
  };
  let sidecar = sidecar_path(&state.meta, key).ok_or("invalid key")?;
  let text = serde_json::to_string(&meta).map_err(|e| e.to_string())?;
  write_atomic(&sidecar, text.as_bytes())?;
  Ok(())
}

/// Collect every stored key under `prefix`, sorted. Sidecars live in another
/// tree, so every file here is an object.
fn keys_under(state: &ServerState, raw_prefix: &str) -> Vec<String> {
  let prefix = normalized_prefix(raw_prefix);
  let Some(root) = object_path(&state.objects, prefix) else {
    return Vec::new();
  };
  let mut keys = Vec::new();
  let mut stack = vec![root];
  while let Some(dir) = stack.pop() {
    let Ok(entries) = std::fs::read_dir(&dir) else {
      continue;
    };
    for entry in entries.flatten() {
      let path = entry.path();
      if path.is_dir() {
        stack.push(path);
      } else if let Some(key) = key_for_path(&state.objects, &path) {
        keys.push(key);
      }
    }
  }
  keys.sort();
  keys
}

/// Remove one object and its sidecar, then any directories it leaves empty.
/// Profile churn would otherwise accumulate a directory per deleted profile
/// forever. Returns whether the object existed.
fn remove_object(state: &ServerState, key: &str) -> bool {
  let Some(path) = object_path(&state.objects, key) else {
    return false;
  };
  let existed = std::fs::remove_file(&path).is_ok();
  if let Some(sidecar) = sidecar_path(&state.meta, key) {
    let _ = std::fs::remove_file(sidecar);
  }
  let mut dir = path.parent().map(|p| p.to_path_buf());
  while let Some(current) = dir {
    if current == state.objects {
      break;
    }
    let empty = match std::fs::read_dir(&current) {
      Ok(mut entries) => entries.next().is_none(),
      Err(_) => break,
    };
    if !empty {
      break;
    }
    let _ = std::fs::remove_dir(&current);
    dir = current.parent().map(|p| p.to_path_buf());
  }
  existed
}

/// Write a tombstone object: presence plus `lastModified` is all conflict
/// resolution reads (`engine::tombstone_blocks`); the body is diagnostic.
fn write_tombstone(state: &ServerState, tombstone_key: &str, deleted_at: Option<&str>) -> bool {
  if tombstone_key.trim().is_empty() {
    return false;
  }
  let deleted_at = deleted_at.map(str::to_string).unwrap_or_else(now_rfc3339);
  let body = serde_json::json!({ "id": tombstone_key, "deletedAt": deleted_at });
  store_object(
    state,
    tombstone_key,
    body.to_string().as_bytes(),
    Some("application/json".to_string()),
    HashMap::new(),
  )
  .is_ok()
}

// ---------------------------------------------------------------------------
// Presigning
// ---------------------------------------------------------------------------

/// Sign `method\nkey\nexpiry` with the server's key. Hex, so the signature
/// needs no URL escaping.
fn sign_url(state: &ServerState, method: &str, key: &str, expires: u64) -> String {
  let payload = format!("{method}\n{key}\n{expires}");
  let tag = hmac::sign(&state.signing_key, payload.as_bytes());
  tag.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn clamped_expires(expires_in: Option<u64>) -> u64 {
  expires_in
    .unwrap_or(DEFAULT_EXPIRES_SECS)
    .clamp(1, MAX_EXPIRES_SECS)
}

fn build_presigned(
  state: &ServerState,
  method: &str,
  key: &str,
  expires_in: Option<u64>,
) -> String {
  let expires = chrono::Utc::now().timestamp() as u64 + clamped_expires(expires_in);
  let signature = sign_url(state, method, key, expires);
  let encoded = key
    .split('/')
    .map(|part| urlencoding::encode(part))
    .collect::<Vec<_>>()
    .join("/");
  format!(
    "{}/storage/{}?X-Amz-Algorithm=DONUT-LOCAL-V1&X-Amz-Expires={expires}&X-Amz-Signature={signature}",
    state.public_base, encoded
  )
}

fn expires_at_rfc3339(expires_in: Option<u64>) -> String {
  let secs = clamped_expires(expires_in) as i64;
  (chrono::Utc::now() + chrono::Duration::seconds(secs)).to_rfc3339()
}

#[derive(Debug, Default, Deserialize)]
struct StorageQuery {
  #[serde(rename = "X-Amz-Expires", default)]
  expires: Option<u64>,
  #[serde(rename = "X-Amz-Signature", default)]
  signature: Option<String>,
}

/// Check a storage request: right method, not expired, tag match.
fn storage_authorized(state: &ServerState, method: &str, key: &str, query: &StorageQuery) -> bool {
  let Some(signature) = query.signature.as_deref() else {
    return false;
  };
  let Some(expires) = query.expires else {
    return false;
  };
  if chrono::Utc::now().timestamp() as u64 > expires {
    return false;
  }
  constant_time_eq(&sign_url(state, method, key, expires), signature)
}

/// Minimal `a=1&b=2` parser for the three parameters a presigned URL carries.
fn parse_storage_query(query: &str) -> Option<StorageQuery> {
  let mut expires = None;
  let mut signature = None;
  for pair in query.split('&') {
    let (name, value) = pair.split_once('=')?;
    match name {
      "X-Amz-Expires" => expires = value.parse().ok(),
      "X-Amz-Signature" => signature = Some(value.to_string()),
      _ => {}
    }
  }
  Some(StorageQuery { expires, signature })
}

// ---------------------------------------------------------------------------
// Handlers — object API
// ---------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
  Json(serde_json::json!({ "status": "ok" }))
}

async fn readyz(State(state): State<ServerState>) -> Json<serde_json::Value> {
  Json(serde_json::json!({
    "status": "ready",
    "s3": true,
    "storageEndpoint": state.public_base,
  }))
}

async fn stat(
  State(state): State<ServerState>,
  Json(request): Json<StatRequest>,
) -> Result<Json<StatResponse>, Response> {
  let Some(path) = object_path(&state.objects, &request.key) else {
    return Err(bad_request("invalid key"));
  };
  let Ok(info) = std::fs::metadata(&path) else {
    return Ok(Json(StatResponse {
      exists: false,
      last_modified: None,
      size: None,
      metadata: None,
    }));
  };
  let meta = read_sidecar(&state, &request.key);
  let last_modified = if meta.last_modified.is_empty() {
    info
      .modified()
      .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
      .unwrap_or_default()
  } else {
    meta.last_modified
  };
  Ok(Json(StatResponse {
    exists: true,
    last_modified: Some(last_modified),
    size: Some(info.len()),
    metadata: meta.metadata,
  }))
}

async fn presign_upload(
  State(state): State<ServerState>,
  Json(request): Json<PresignUploadRequest>,
) -> Result<Json<PresignUploadResponse>, Response> {
  if object_path(&state.objects, &request.key).is_none() {
    return Err(bad_request("invalid key"));
  }
  Ok(Json(PresignUploadResponse {
    url: build_presigned(&state, "PUT", &request.key, request.expires_in),
    expires_at: expires_at_rfc3339(request.expires_in),
    metadata: request.metadata,
  }))
}

async fn presign_download(
  State(state): State<ServerState>,
  Json(request): Json<PresignDownloadRequest>,
) -> Result<Json<PresignDownloadResponse>, Response> {
  if object_path(&state.objects, &request.key).is_none() {
    return Err(bad_request("invalid key"));
  }
  Ok(Json(PresignDownloadResponse {
    url: build_presigned(&state, "GET", &request.key, request.expires_in),
    expires_at: expires_at_rfc3339(request.expires_in),
  }))
}

async fn presign_upload_batch(
  State(state): State<ServerState>,
  Json(request): Json<PresignUploadBatchRequest>,
) -> Result<Json<PresignUploadBatchResponse>, Response> {
  if request
    .items
    .iter()
    .any(|item| object_path(&state.objects, &item.key).is_none())
  {
    return Err(bad_request("invalid key"));
  }
  let items = request
    .items
    .into_iter()
    .map(
      |item: PresignUploadBatchItem| PresignUploadBatchItemResponse {
        url: build_presigned(&state, "PUT", &item.key, request.expires_in),
        expires_at: expires_at_rfc3339(request.expires_in),
        key: item.key,
      },
    )
    .collect();
  Ok(Json(PresignUploadBatchResponse { items }))
}

async fn presign_download_batch(
  State(state): State<ServerState>,
  Json(request): Json<PresignDownloadBatchRequest>,
) -> Result<Json<PresignDownloadBatchResponse>, Response> {
  if request
    .keys
    .iter()
    .any(|key| object_path(&state.objects, key).is_none())
  {
    return Err(bad_request("invalid key"));
  }
  let items = request
    .keys
    .into_iter()
    .map(|key| PresignDownloadBatchItemResponse {
      url: build_presigned(&state, "GET", &key, request.expires_in),
      expires_at: expires_at_rfc3339(request.expires_in),
      key,
    })
    .collect();
  Ok(Json(PresignDownloadBatchResponse { items }))
}

async fn delete(
  State(state): State<ServerState>,
  Json(request): Json<DeleteRequest>,
) -> Result<Json<DeleteResponse>, Response> {
  if object_path(&state.objects, &request.key).is_none() {
    return Err(bad_request("invalid key"));
  }
  let (deleted, tombstone_created) = locked_write(&state, || {
    let deleted = remove_object(&state, &request.key);
    let tombstone = write_tombstone(
      &state,
      request.tombstone_key.as_deref().unwrap_or(""),
      request.deleted_at.as_deref(),
    );
    (deleted, tombstone)
  });
  Ok(Json(DeleteResponse {
    deleted,
    tombstone_created,
  }))
}

/// Client prefixes name directories and so end with `/` (`profiles/{id}/`
/// everywhere in `engine`); the slash is a separator, not a component, so
/// strip it before validation instead of rejecting calls the reference
/// server accepts.
fn normalized_prefix(raw: &str) -> &str {
  raw.strip_suffix('/').unwrap_or(raw)
}

async fn delete_prefix(
  State(state): State<ServerState>,
  Json(request): Json<DeletePrefixRequest>,
) -> Result<Json<DeletePrefixResponse>, Response> {
  let prefix = normalized_prefix(&request.prefix);
  if !prefix.is_empty() && object_path(&state.objects, prefix).is_none() {
    return Err(bad_request("invalid prefix"));
  }
  let keys = keys_under(&state, prefix);
  let (deleted_count, tombstone_created) = locked_write(&state, || {
    let deleted = keys.iter().filter(|key| remove_object(&state, key)).count();
    let tombstone = write_tombstone(
      &state,
      request.tombstone_key.as_deref().unwrap_or(""),
      request.deleted_at.as_deref(),
    );
    (deleted, tombstone)
  });
  Ok(Json(DeletePrefixResponse {
    deleted_count: deleted_count as u32,
    tombstone_created,
  }))
}

async fn list(
  State(state): State<ServerState>,
  Json(request): Json<ListRequest>,
) -> Result<Json<ListResponse>, Response> {
  let prefix = normalized_prefix(&request.prefix);
  if !prefix.is_empty() && object_path(&state.objects, prefix).is_none() {
    return Err(bad_request("invalid prefix"));
  }
  let max_keys = request.max_keys.unwrap_or(1000).clamp(1, 1000) as usize;
  let from = request.continuation_token.as_deref();
  let mut keys = keys_under(&state, prefix)
    .into_iter()
    .filter(|key| from.is_none_or(|token| key.as_str() > token));
  let mut objects = Vec::new();
  let mut last_key = None;
  for key in keys.by_ref().take(max_keys) {
    let size = object_path(&state.objects, &key)
      .and_then(|p| std::fs::metadata(p).ok())
      .map_or(0, |m| m.len());
    let meta = read_sidecar(&state, &key);
    last_key = Some(key.clone());
    objects.push(ListObject {
      last_modified: if meta.last_modified.is_empty() {
        now_rfc3339()
      } else {
        meta.last_modified
      },
      key,
      size,
    });
  }
  let is_truncated = keys.next().is_some();
  Ok(Json(ListResponse {
    objects,
    is_truncated,
    next_continuation_token: if is_truncated { last_key } else { None },
  }))
}

// ---------------------------------------------------------------------------
// Handlers — snapshots
// ---------------------------------------------------------------------------

/// Normalize a client-supplied profile root to a trailing-slash prefix that
/// is a valid key prefix.
fn snapshot_profile_root(raw: &str) -> Option<String> {
  let trimmed = raw.trim().trim_matches('/');
  if trimmed.is_empty() || trimmed.len() > MAX_KEY_LEN {
    return None;
  }
  for part in trimmed.split('/') {
    if !crate::app_dirs::is_plain_path_part(part) {
      return None;
    }
  }
  Some(format!("{trimmed}/"))
}

fn snapshot_prefix(profile_root: &str, id: &str) -> Option<String> {
  if !crate::app_dirs::is_plain_path_part(id) {
    return None;
  }
  Some(format!("snapshots/{profile_root}{id}/"))
}

/// Copy every object under `from_prefix` into `to_prefix`, preserving
/// sidecars. Returns `(files, bytes)`. `skip_marker` drops the snapshot
/// marker on the restore-files path, where the copy becomes live data again.
fn copy_prefix(
  state: &ServerState,
  from_prefix: &str,
  to_prefix: &str,
  skip_marker: bool,
) -> (u64, u64) {
  let mut files = 0u64;
  let mut bytes = 0u64;
  for key in keys_under(state, from_prefix) {
    let rel = &key[from_prefix.len()..];
    if skip_marker && rel == SNAPSHOT_MARKER {
      continue;
    }
    let Some(source) = object_path(&state.objects, &key) else {
      continue;
    };
    let Ok(body) = std::fs::read(&source) else {
      continue;
    };
    let meta = read_sidecar(state, &key);
    let stored = store_object(
      state,
      &format!("{to_prefix}{rel}"),
      &body,
      meta.content_type,
      meta.metadata.unwrap_or_default(),
    );
    if stored.is_ok() {
      files += 1;
      bytes += body.len() as u64;
    }
  }
  (files, bytes)
}

/// Remove every object under `prefix`, returning how many went away.
fn clear_prefix(state: &ServerState, prefix: &str) -> u64 {
  keys_under(state, prefix)
    .iter()
    .filter(|key| remove_object(state, key))
    .count() as u64
}

fn read_snapshot_marker(state: &ServerState, marker_key: &str) -> Option<RestorePointDto> {
  let text = std::fs::read_to_string(object_path(&state.objects, marker_key)?).ok()?;
  serde_json::from_str(&text).ok()
}

/// Every restore point recorded under a profile root, newest first.
fn restore_points(state: &ServerState, profile_root: &str) -> Vec<RestorePointDto> {
  let mut points: Vec<RestorePointDto> = keys_under(state, &format!("snapshots/{profile_root}"))
    .into_iter()
    .filter_map(|key| read_snapshot_marker(state, &key))
    .collect();
  points.sort_by(|a, b| b.created_at.cmp(&a.created_at));
  points
}

async fn snapshots_list(
  State(state): State<ServerState>,
  Json(request): Json<SnapshotsListRequest>,
) -> Result<Json<serde_json::Value>, Response> {
  let Some(root) = snapshot_profile_root(&request.profile_root) else {
    return Err(bad_request("invalid profileRoot"));
  };
  Ok(Json(
    serde_json::json!({ "snapshots": restore_points(&state, &root) }),
  ))
}

async fn snapshots_create(
  State(state): State<ServerState>,
  Json(request): Json<SnapshotsCreateRequest>,
) -> Result<Json<RestorePointDto>, Response> {
  let Some(root) = snapshot_profile_root(&request.profile_root) else {
    return Err(bad_request("invalid profileRoot"));
  };
  let id = uuid::Uuid::new_v4().to_string();
  let copy_root = format!("snapshots/{root}{id}/");
  let point = locked_write(&state, || {
    let (files, bytes) = copy_prefix(&state, &root, &copy_root, false);
    let point = RestorePointDto {
      id: id.clone(),
      created_at: now_rfc3339(),
      kind: request.kind.clone(),
      files,
      bytes,
    };
    let marker_body = serde_json::to_string(&point).unwrap_or_default();
    store_object(
      &state,
      &format!("{copy_root}{SNAPSHOT_MARKER}"),
      marker_body.as_bytes(),
      Some("application/json".to_string()),
      HashMap::new(),
    )?;

    // Retention: newest three per kind, older ones pruned — under the same
    // lock, so a concurrent create cannot prune from a half-built list.
    let stale: Vec<String> = restore_points(&state, &root)
      .into_iter()
      .filter(|point| point.kind == request.kind)
      .skip(SNAPSHOTS_KEPT_PER_KIND)
      .map(|point| point.id)
      .collect();
    for stale_id in stale {
      if let Some(prefix) = snapshot_prefix(&root, &stale_id) {
        clear_prefix(&state, &prefix);
      }
    }
    Ok::<RestorePointDto, String>(point)
  });
  let point = point.map_err(|e| bad_request(&e))?;

  Ok(Json(point))
}

async fn snapshots_delete(
  State(state): State<ServerState>,
  Json(request): Json<SnapshotsTargetRequest>,
) -> Result<Json<serde_json::Value>, Response> {
  let Some(root) = snapshot_profile_root(&request.profile_root) else {
    return Err(bad_request("invalid profileRoot"));
  };
  let Some(prefix) = snapshot_prefix(&root, &request.id) else {
    return Err(bad_request("invalid id"));
  };
  if keys_under(&state, &prefix).is_empty() {
    return Err(bad_request("unknown snapshot"));
  }
  locked_write(&state, || clear_prefix(&state, &prefix));
  Ok(Json(serde_json::json!({ "deleted": true })))
}

async fn snapshots_restore_files(
  State(state): State<ServerState>,
  Json(request): Json<SnapshotsTargetRequest>,
) -> Result<Json<serde_json::Value>, Response> {
  let Some(root) = snapshot_profile_root(&request.profile_root) else {
    return Err(bad_request("invalid profileRoot"));
  };
  let Some(prefix) = snapshot_prefix(&root, &request.id) else {
    return Err(bad_request("invalid id"));
  };
  if keys_under(&state, &prefix).is_empty() {
    return Err(bad_request("unknown snapshot"));
  }
  locked_write(&state, || copy_prefix(&state, &prefix, &root, true));
  Ok(Json(serde_json::json!({ "restored": true })))
}

// ---------------------------------------------------------------------------
// Handlers — subscription and storage
// ---------------------------------------------------------------------------

type SubscribeStream = futures_util::stream::Pending<Result<Event, std::convert::Infallible>>;

/// An SSE stream that never carries an event: a single-machine server has no
/// second device to report. The keep-alive comment every 20 s holds the
/// connection open inside the client's 60 s read timeout and parses as no
/// event on the client side.
async fn subscribe() -> Sse<KeepAliveStream<SubscribeStream>> {
  Sse::new(futures_util::stream::pending()).keep_alive(
    KeepAlive::new()
      .interval(Duration::from_secs(SSE_KEEPALIVE_SECS))
      .text("keepalive"),
  )
}

async fn storage_put(
  State(state): State<ServerState>,
  Path(key): Path<String>,
  RawQuery(query): RawQuery,
  headers: HeaderMap,
  body: Bytes,
) -> Response {
  let Some(query) = query.as_deref().and_then(parse_storage_query) else {
    return forbidden();
  };
  if !storage_authorized(&state, "PUT", &key, &query) {
    return forbidden();
  }
  if object_path(&state.objects, &key).is_none() {
    return bad_request("invalid key");
  }
  if body.len() > MAX_OBJECT_BYTES {
    return (StatusCode::PAYLOAD_TOO_LARGE, "object exceeds size limit").into_response();
  }
  let content_type = headers
    .get(axum::http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .map(str::to_string);
  // Metadata headers are stored, not signed into the URL: the presign call
  // itself was authenticated, so recording what the PUT actually carried is
  // the honest record for later `stat` conflict checks.
  let mut metadata = HashMap::new();
  for (name, value) in headers.iter() {
    if let (Some(meta_key), Ok(text)) = (name.as_str().strip_prefix("x-amz-meta-"), value.to_str())
    {
      metadata.insert(meta_key.to_string(), text.to_string());
    }
  }
  let outcome = locked_write(&state, || {
    store_object(&state, &key, &body, content_type.clone(), metadata.clone())
  });
  match outcome {
    Ok(()) => (
      StatusCode::OK,
      [(
        axum::http::header::ETAG,
        format!("\"{}\"", blake3::hash(&body).to_hex()),
      )],
    )
      .into_response(),
    Err(e) => (
      StatusCode::INTERNAL_SERVER_ERROR,
      Json(serde_json::json!({ "statusCode": 500, "message": e })),
    )
      .into_response(),
  }
}

async fn storage_get(
  State(state): State<ServerState>,
  Path(key): Path<String>,
  RawQuery(query): RawQuery,
) -> Response {
  let Some(query) = query.as_deref().and_then(parse_storage_query) else {
    return forbidden();
  };
  if !storage_authorized(&state, "GET", &key, &query) {
    return forbidden();
  }
  let Some(path) = object_path(&state.objects, &key) else {
    return bad_request("invalid key");
  };
  let Ok(body) = std::fs::read(&path) else {
    return (
      StatusCode::NOT_FOUND,
      Json(serde_json::json!({ "statusCode": 404, "message": "NotFound" })),
    )
      .into_response();
  };
  let content_type = read_sidecar(&state, &key)
    .content_type
    .unwrap_or_else(|| "application/octet-stream".to_string());
  (
    StatusCode::OK,
    [
      (axum::http::header::CONTENT_TYPE, content_type),
      (
        axum::http::header::ETAG,
        format!("\"{}\"", blake3::hash(&body).to_hex()),
      ),
    ],
    body,
  )
    .into_response()
}
