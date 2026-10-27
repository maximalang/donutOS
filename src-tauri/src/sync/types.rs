use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatRequest {
  pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatResponse {
  pub exists: bool,
  #[serde(rename = "lastModified")]
  pub last_modified: Option<String>,
  pub size: Option<u64>,
  /// User-defined S3 object metadata (`x-amz-meta-*`), lowercased keys without
  /// the prefix. `None` from older servers that don't return it. Used to read
  /// `updated-at` for sync conflict resolution without downloading the body.
  #[serde(default)]
  pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadRequest {
  pub key: String,
  #[serde(rename = "contentType")]
  pub content_type: Option<String>,
  #[serde(rename = "expiresIn")]
  pub expires_in: Option<u64>,
  /// Object metadata to sign into the presigned PUT (stored as `x-amz-meta-*`).
  #[serde(skip_serializing_if = "Option::is_none")]
  pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadResponse {
  pub url: String,
  #[serde(rename = "expiresAt")]
  pub expires_at: String,
  /// The metadata the server actually signed into the URL. The client must send
  /// exactly these as `x-amz-meta-*` headers on the PUT or S3 rejects it. `None`
  /// from older servers → client sends no metadata headers (body-GET fallback).
  #[serde(default)]
  pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignDownloadRequest {
  pub key: String,
  #[serde(rename = "expiresIn")]
  pub expires_in: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignDownloadResponse {
  pub url: String,
  #[serde(rename = "expiresAt")]
  pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteRequest {
  pub key: String,
  #[serde(rename = "tombstoneKey")]
  pub tombstone_key: Option<String>,
  #[serde(rename = "deletedAt")]
  pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteResponse {
  pub deleted: bool,
  #[serde(rename = "tombstoneCreated")]
  pub tombstone_created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRequest {
  pub prefix: String,
  #[serde(rename = "maxKeys")]
  pub max_keys: Option<u32>,
  #[serde(rename = "continuationToken")]
  pub continuation_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListObject {
  pub key: String,
  #[serde(rename = "lastModified")]
  pub last_modified: String,
  pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
  pub objects: Vec<ListObject>,
  #[serde(rename = "isTruncated")]
  pub is_truncated: bool,
  #[serde(rename = "nextContinuationToken")]
  pub next_continuation_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tombstone {
  pub id: String,
  pub deleted_at: String,
}

// Batch presign types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadBatchItem {
  pub key: String,
  #[serde(rename = "contentType")]
  pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadBatchRequest {
  pub items: Vec<PresignUploadBatchItem>,
  #[serde(rename = "expiresIn")]
  pub expires_in: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadBatchItemResponse {
  pub key: String,
  pub url: String,
  #[serde(rename = "expiresAt")]
  pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignUploadBatchResponse {
  pub items: Vec<PresignUploadBatchItemResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignDownloadBatchRequest {
  pub keys: Vec<String>,
  #[serde(rename = "expiresIn")]
  pub expires_in: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignDownloadBatchItemResponse {
  pub key: String,
  pub url: String,
  #[serde(rename = "expiresAt")]
  pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresignDownloadBatchResponse {
  pub items: Vec<PresignDownloadBatchItemResponse>,
}

// Delete prefix types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletePrefixRequest {
  pub prefix: String,
  #[serde(rename = "tombstoneKey")]
  pub tombstone_key: Option<String>,
  #[serde(rename = "deletedAt")]
  pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletePrefixResponse {
  #[serde(rename = "deletedCount")]
  pub deleted_count: u32,
  #[serde(rename = "tombstoneCreated")]
  pub tombstone_created: bool,
}

#[derive(Debug)]
pub enum SyncError {
  NotConfigured,
  NetworkError(String),
  AuthError(String),
  IoError(String),
  SerializationError(String),
  ConflictError(String),
  InvalidData(String),
  Cancelled,
}

impl std::fmt::Display for SyncError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      SyncError::NotConfigured => write!(f, "Sync not configured"),
      SyncError::NetworkError(msg) => write!(f, "Network error: {msg}"),
      SyncError::AuthError(msg) => write!(f, "Authentication error: {msg}"),
      SyncError::IoError(msg) => write!(f, "IO error: {msg}"),
      SyncError::SerializationError(msg) => write!(f, "Serialization error: {msg}"),
      SyncError::ConflictError(msg) => write!(f, "Conflict error: {msg}"),
      SyncError::InvalidData(msg) => write!(f, "Invalid data: {msg}"),
      SyncError::Cancelled => write!(f, "Sync cancelled by user"),
    }
  }
}

impl std::error::Error for SyncError {}

impl SyncError {
  /// The code the app shows for this failure, with its params. English
  /// details stay in the log; only what a translation needs leaves here.
  pub fn user_code(&self) -> (String, serde_json::Map<String, serde_json::Value>) {
    let mut params = serde_json::Map::new();
    let code = match self {
      SyncError::NotConfigured => "SYNC_NOT_CONFIGURED".to_string(),
      SyncError::Cancelled => "SYNC_CANCELLED".to_string(),
      SyncError::NetworkError(msg) => {
        if let Some(status) = http_status_in(msg) {
          params.insert("status".into(), status.to_string().into());
          if status >= 500 {
            "SYNC_SERVER_ERROR".to_string()
          } else {
            "SYNC_STORAGE_REJECTED".to_string()
          }
        } else if let Some(host) = storage_host_in(msg) {
          params.insert("endpoint".into(), host.into());
          "SYNC_SERVER_STORAGE_UNREACHABLE".to_string()
        } else if msg.contains("incomplete") || msg.contains("No presigned URL") {
          "SYNC_TRANSFER_INCOMPLETE".to_string()
        } else {
          "SYNC_SERVER_UNREACHABLE".to_string()
        }
      }
      SyncError::AuthError(msg) => {
        if let Some(code) = server_code_in(msg) {
          code
        } else {
          let status = auth_status_in(msg).unwrap_or(0);
          match status {
            401 => "SYNC_AUTH_FAILED".to_string(),
            403 => "SYNC_FORBIDDEN".to_string(),
            413 => "SYNC_TOO_LARGE".to_string(),
            429 => "SYNC_RATE_LIMITED".to_string(),
            _ => {
              params.insert("status".into(), status.to_string().into());
              "SYNC_SERVER_REJECTED".to_string()
            }
          }
        }
      }
      SyncError::IoError(_) => "SYNC_LOCAL_FILES_FAILED".to_string(),
      SyncError::SerializationError(_) | SyncError::ConflictError(_) => {
        "SYNC_DATA_INVALID".to_string()
      }
      SyncError::InvalidData(msg) => {
        if msg.contains("E2E password not set") {
          "NO_E2E_PASSWORD_SET".to_string()
        } else if msg.contains("unseal")
          || msg.contains("decrypt")
          || msg.contains("Key derivation")
        {
          "SYNC_DECRYPT_FAILED".to_string()
        } else {
          "SYNC_DATA_INVALID".to_string()
        }
      }
    };
    (code, params)
  }

  /// `{"code": ..., "params": {...}}`, the shape every user-facing error has.
  pub fn to_code_json(&self) -> String {
    let (code, params) = self.user_code();
    if params.is_empty() {
      serde_json::json!({ "code": code }).to_string()
    } else {
      serde_json::json!({ "code": code, "params": params }).to_string()
    }
  }

  /// The machine cannot reach the server at all, as opposed to the server
  /// refusing or failing a request.
  pub fn is_offline(&self) -> bool {
    matches!(self.user_code().0.as_str(), "SYNC_SERVER_UNREACHABLE")
  }
}

/// Whether an engine setup failure is the network: a token refresh or a
/// server lookup that never reached anything.
pub fn engine_setup_is_offline(reason: &str) -> bool {
  let reason = reason.to_ascii_lowercase();
  [
    "error sending request",
    "dns",
    "connect",
    "timed out",
    "network",
    "unreachable",
  ]
  .iter()
  .any(|needle| reason.contains(needle))
}

/// The code of an engine that could not be built. The reasons are strings
/// from `SyncEngine::create_from_settings`.
pub fn engine_setup_code_name(reason: &str) -> &'static str {
  if engine_setup_is_offline(reason) {
    "SYNC_SERVER_UNREACHABLE"
  } else if reason.contains("not configured") || reason.contains("not available") {
    "SYNC_NOT_CONFIGURED"
  } else if reason.contains("token") {
    "SYNC_AUTH_FAILED"
  } else {
    "INTERNAL_ERROR"
  }
}

/// The same, as the JSON a command returns.
pub fn engine_setup_code(reason: &str) -> String {
  match engine_setup_code_name(reason) {
    "INTERNAL_ERROR" => crate::backend_error_with_detail("INTERNAL_ERROR", reason),
    code => crate::backend_error(code),
  }
}

/// `Upload failed with status 403 Forbidden: ...` -> 403.
fn http_status_in(msg: &str) -> Option<u16> {
  let rest = &msg[msg.find("status")? + "status".len()..];
  let digits: String = rest
    .trim_start_matches([':', ' '])
    .chars()
    .take_while(char::is_ascii_digit)
    .collect();
  digits
    .parse()
    .ok()
    .filter(|status| (100..600).contains(status))
}

/// `... (storage host minio:9000)` -> `minio:9000`.
fn storage_host_in(msg: &str) -> Option<String> {
  let rest = &msg[msg.find("(storage host ")? + "(storage host ".len()..];
  Some(rest.trim_end_matches(')').to_string()).filter(|host| !host.is_empty())
}

/// `(401 Unauthorized) body` -> 401.
fn auth_status_in(msg: &str) -> Option<u16> {
  let digits: String = msg
    .trim_start_matches('(')
    .chars()
    .take_while(char::is_ascii_digit)
    .collect();
  digits.parse().ok()
}

/// The server's own code in a refusal body, `{"code": ...}` or NestJS's
/// `{"message": {"code": ...}}`.
fn server_code_in(msg: &str) -> Option<String> {
  let body: serde_json::Value = serde_json::from_str(&msg[msg.find("{\"")?..]).ok()?;
  body
    .get("code")
    .or_else(|| body.pointer("/message/code"))
    .and_then(serde_json::Value::as_str)
    .filter(|code| {
      !code.is_empty()
        && code
          .chars()
          .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    })
    .map(str::to_string)
}

pub type SyncResult<T> = Result<T, SyncError>;
