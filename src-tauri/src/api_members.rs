//! Named API members with roles: the local product's answer to the vendor's
//! team seats. Each member is an automation client of the local API server —
//! a fleet bot, an operator's script — with a name, a role and a personal
//! token. Tokens are stored as SHA-256 digests only; the plaintext is shown
//! exactly once, at issue time. The master automation token keeps its owner
//! rights untouched, so existing fleet clients never break.

use crate::app_dirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
  /// The master token's implicit role, and members trusted with management:
  /// create and delete anything, manage members.
  Owner,
  /// Drive the fleet: launch, stop, navigate, cookie-bot runs, agent
  /// actions. No configuration changes, no deletions.
  Operator,
  /// Read-only: profiles, statuses, schedules, run history.
  Viewer,
}

impl Role {
  pub fn parse(value: &str) -> Option<Self> {
    match value {
      "owner" => Some(Self::Owner),
      "operator" => Some(Self::Operator),
      "viewer" => Some(Self::Viewer),
      _ => None,
    }
  }

  pub fn as_str(&self) -> &'static str {
    match self {
      Self::Owner => "owner",
      Self::Operator => "operator",
      Self::Viewer => "viewer",
    }
  }
}

/// What a member may do. Safe to hand to scripts: no secrets inside.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMember {
  pub id: Uuid,
  pub name: String,
  pub role: Role,
  pub created_at: String,
  pub last_used_at: Option<String>,
  /// SHA-256 of the token, hex. The token itself exists only at issue time.
  pub token_hash: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct MemberDoc {
  members: Vec<ApiMember>,
}

#[derive(Debug, thiserror::Error)]
pub enum MemberError {
  #[error("member store io: {0}")]
  Io(#[from] std::io::Error),
  #[error("member store serde: {0}")]
  Serde(#[from] serde_json::Error),
  #[error("a member named {0} already exists")]
  Duplicate(String),
  #[error("unknown member {0}")]
  Unknown(Uuid),
  #[error("invalid member name")]
  InvalidName,
}

fn store_path() -> PathBuf {
  app_dirs::data_dir().join("api_members.json")
}

static STORE_LOCK: Mutex<()> = Mutex::new(());

/// Test-only store directory override.
#[cfg(test)]
static TEST_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

fn path() -> PathBuf {
  #[cfg(test)]
  if let Some(dir) = TEST_DIR.lock().unwrap_or_else(|p| p.into_inner()).clone() {
    return dir.join("api_members.json");
  }
  store_path()
}

fn load() -> MemberDoc {
  let p = path();
  if !p.exists() {
    return MemberDoc::default();
  }
  std::fs::read(&p)
    .ok()
    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    .unwrap_or_default()
}

fn save(doc: &MemberDoc) -> Result<(), MemberError> {
  let p = path();
  if let Some(parent) = p.parent() {
    std::fs::create_dir_all(parent)?;
  }
  std::fs::write(&p, serde_json::to_vec_pretty(doc)?)?;
  Ok(())
}

fn hash_token(token: &str) -> String {
  let digest = Sha256::digest(token.as_bytes());
  digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn mint_token() -> String {
  use base64::Engine as _;
  let mut bytes = [0u8; 32];
  for slot in bytes.iter_mut() {
    *slot = rand::random::<u8>();
  }
  base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Issue a member. Returns the member and the plaintext token exactly once.
pub fn issue(name: &str, role: Role) -> Result<(ApiMember, String), MemberError> {
  let name = name.trim();
  if name.is_empty() || name.len() > 40 {
    return Err(MemberError::InvalidName);
  }
  let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
  let mut doc = load();
  if doc.members.iter().any(|m| m.name == name) {
    return Err(MemberError::Duplicate(name.to_string()));
  }
  let token = mint_token();
  let member = ApiMember {
    id: Uuid::new_v4(),
    name: name.to_string(),
    role,
    created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    last_used_at: None,
    token_hash: hash_token(&token),
  };
  doc.members.push(member.clone());
  save(&doc)?;
  Ok((member, token))
}

/// Resolve a presented token to its member. Constant-time digest comparison
/// per candidate; member counts are small (tens), so the linear scan is fine.
pub fn find_by_token(token: &str) -> Option<ApiMember> {
  use subtle::ConstantTimeEq;
  let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
  let doc = load();
  let presented = hash_token(token);
  let presented_bytes = presented.as_bytes();
  doc
    .members
    .iter()
    .find(|m| {
      let stored = m.token_hash.as_bytes();
      stored.len() == presented_bytes.len() && stored.ct_eq(presented_bytes).into()
    })
    .cloned()
}

/// The public roster, without digests.
pub fn list() -> Vec<ApiMember> {
  let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
  load().members
}

/// Remove a member; their token stops working on the next request.
pub fn revoke(id: Uuid) -> Result<(), MemberError> {
  let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
  let mut doc = load();
  let before = doc.members.len();
  doc.members.retain(|m| m.id != id);
  if doc.members.len() == before {
    return Err(MemberError::Unknown(id));
  }
  save(&doc)
}

/// Stamp a member's last use. A failed stamp is noise, not an error.
pub fn touch(id: Uuid) {
  let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
  let mut doc = load();
  if let Some(m) = doc.members.iter_mut().find(|m| m.id == id) {
    m.last_used_at = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    let _ = save(&doc);
  }
}

// ---- Tauri command surface (the UI runs as the owner) ----

#[derive(Serialize)]
pub struct IssuedMember {
  #[serde(flatten)]
  pub member: ApiMember,
  /// The plaintext token, shown exactly once.
  pub token: String,
}

#[tauri::command]
pub fn list_api_members() -> Result<Vec<ApiMember>, String> {
  Ok(list())
}

#[tauri::command]
pub fn create_api_member(name: String, role: String) -> Result<IssuedMember, String> {
  let role = Role::parse(&role).ok_or_else(|| format!("Unknown role: {role}"))?;
  let (member, token) = issue(&name, role).map_err(|e| e.to_string())?;
  Ok(IssuedMember { member, token })
}

#[tauri::command]
pub fn revoke_api_member(id: Uuid) -> Result<(), String> {
  revoke(id).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  static TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

  fn with_store(f: impl FnOnce()) {
    let _guard = TEST_GUARD.lock().unwrap_or_else(|p| p.into_inner());
    let dir = std::env::temp_dir().join(format!(
      "donut-members-{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    *TEST_DIR.lock().unwrap() = Some(dir.clone());
    f();
    *TEST_DIR.lock().unwrap() = None;
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn issue_find_revoke_roundtrip() {
    with_store(|| {
      let (member, token) = issue("herald", Role::Operator).unwrap();
      assert!(!token.is_empty());
      assert!(!member.token_hash.contains(&token[..8]));

      let found = find_by_token(&token).unwrap();
      assert_eq!(found.id, member.id);
      assert_eq!(found.role, Role::Operator);

      // A wrong token resolves to nothing.
      assert!(find_by_token(&token[..31]).is_none());

      touch(member.id);
      let stamped = list().into_iter().find(|m| m.id == member.id).unwrap();
      assert!(stamped.last_used_at.is_some());

      revoke(member.id).unwrap();
      assert!(find_by_token(&token).is_none());
      assert!(list().is_empty());
    });
  }

  #[test]
  fn names_are_unique_and_validated() {
    with_store(|| {
      assert!(issue("herald", Role::Viewer).is_ok());
      assert!(matches!(
        issue("herald", Role::Viewer),
        Err(MemberError::Duplicate(_))
      ));
      assert!(matches!(
        issue("  ", Role::Viewer),
        Err(MemberError::InvalidName)
      ));
    });
  }
}
