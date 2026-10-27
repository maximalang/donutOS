//! The Wayfern automation gate, found by meaning and opened by one branch.
//!
//! The vendor engine ships a licence gate on its CDP automation surface: a
//! call that answers whether automation is allowed, `test eax,eax`, and a
//! branch that skips a refusal block when the answer says no. A fresh vendor
//! build refuses automation; the installed engines here run with that branch
//! forced (see docs/engine-patches.md for the history of 151/154/155, patched
//! first by hand, then by byte signature, now by this module).
//!
//! Finding the gate no longer depends on how a particular build was compiled.
//! The scanner anchors on the one thing that carries meaning across versions —
//! the refusal message itself ("Browser automation requires a paid …") — then
//! follows the code that loads it: a rip-relative `lea` aimed at the string,
//! enclosed by the entitlement call, its `test`, and the conditional branch
//! that jumps past the whole refusal. Register allocation, instruction order
//! and the message tail can all change; the shape holds. The old byte
//! signature (call/test/je plus `cmp eax,2` and `mov ebx,0x36` inside the
//! refusal) stays as a fallback for builds whose string was reworded but
//! whose code still matches the classic form.
//!
//! Only when exactly one site is found does anything get patched, and the
//! patch is never trusted on faith: after it, `verify_gate_open` launches the
//! engine headless and evaluates through CDP, which is the behaviour the gate
//! exists to block. A new engine version either passes that ladder or is
//! removed untouched, which is what makes engine updates safe to attempt.

use std::path::{Path, PathBuf};

/// Why a patch could not happen, in the caller's words.
#[derive(Debug)]
pub enum PatchError {
  /// The file could not be read or is not a PE.
  Unreadable(String),
  /// The PE has no executable section to scan.
  NoCode,
  /// No gate was found by anchor or by signature — the engine differs from
  /// every version seen so far. The file is untouched.
  SignatureNotFound,
  /// The gate matched more than once; patching blind could corrupt.
  /// The file is untouched.
  Ambiguous(usize),
  /// The bytes changed under us between reading and writing.
  ChangedOnDisk,
}

impl std::fmt::Display for PatchError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      PatchError::Unreadable(why) => write!(f, "chrome.dll unreadable: {why}"),
      PatchError::NoCode => write!(f, "chrome.dll has no executable section"),
      PatchError::SignatureNotFound => write!(
        f,
        "the automation gate was not found; this engine version is new and needs a new patch"
      ),
      PatchError::Ambiguous(n) => write!(
        f,
        "the automation gate matched {n} sites; refusing to patch blind"
      ),
      PatchError::ChangedOnDisk => write!(f, "chrome.dll changed while being patched"),
    }
  }
}

/// How the gate was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchMethod {
  /// Through the refusal message and the code that loads it.
  RefusalAnchor,
  /// Through the classic byte signature of 154/155.
  ByteSignature,
}

impl PatchMethod {
  fn as_str(self) -> &'static str {
    match self {
      PatchMethod::RefusalAnchor => "refusal-anchor",
      PatchMethod::ByteSignature => "byte-signature",
    }
  }
}

/// What a patch did.
#[derive(Debug)]
pub struct PatchReport {
  /// File offset of the branch that was forced.
  pub gate_offset: usize,
  /// The original six bytes, for the record.
  pub original: [u8; 6],
  /// True when the branch was already forced on an earlier run.
  pub already_patched: bool,
  /// How the gate was located.
  pub method: &'static str,
}

/// A PE section: where its bytes sit in the file and where they load in
/// memory. The VA matters because rip-relative references — the string loads
/// this module follows — are resolved in memory addresses, not file offsets.
struct Section {
  characteristics: u32,
  va: u32,
  raw: std::ops::Range<usize>,
}

/// Every section of a PE that has bytes in the file.
fn image_sections(buf: &[u8]) -> Result<Vec<Section>, PatchError> {
  let read_u16 = |off: usize| u16::from_le_bytes([buf[off], buf[off + 1]]);
  let read_u32 =
    |off: usize| u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
  if buf.len() < 0x40 || &buf[0..2] != b"MZ" {
    return Err(PatchError::Unreadable("not an MZ image".into()));
  }
  let pe = read_u32(0x3c) as usize;
  if pe + 24 > buf.len() || &buf[pe..pe + 4] != b"PE\0\0" {
    return Err(PatchError::Unreadable("no PE header".into()));
  }
  let sections = read_u16(pe + 6) as usize;
  let opt_size = read_u16(pe + 20) as usize;
  let table = pe + 24 + opt_size;
  if table + sections * 40 > buf.len() {
    return Err(PatchError::Unreadable("section table out of range".into()));
  }
  let mut out = Vec::new();
  for i in 0..sections {
    let off = table + i * 40;
    let raw_size = read_u32(off + 16) as usize;
    let raw = read_u32(off + 20) as usize;
    let va = read_u32(off + 12);
    let characteristics = read_u32(off + 36);
    if raw == 0 || raw_size == 0 {
      continue;
    }
    out.push(Section {
      characteristics,
      va,
      raw: raw..(raw + raw_size).min(buf.len()),
    });
  }
  if out.is_empty() {
    return Err(PatchError::NoCode);
  }
  Ok(out)
}

/// The executable section's byte range of a PE, as file offsets: the largest
/// section marked executable.
fn executable_range(buf: &[u8]) -> Result<std::ops::Range<usize>, PatchError> {
  const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
  let mut best: Option<std::ops::Range<usize>> = None;
  for section in image_sections(buf)? {
    if section.characteristics & IMAGE_SCN_MEM_EXECUTE == 0 {
      continue;
    }
    if best.as_ref().is_none_or(|r| section.raw.len() > r.len()) {
      best = Some(section.raw);
    }
  }
  best.ok_or(PatchError::NoCode)
}

/// Where a file offset lands in memory, if it belongs to a section.
fn va_of(off: usize, sections: &[Section]) -> Option<u32> {
  sections
    .iter()
    .find(|s| s.raw.contains(&off))
    .map(|s| s.va + (off - s.raw.start) as u32)
}

/// The stable part of the refusal message. The tail can be reworded, and the
/// length always changes with it; the opening is what identifies the string.
const REFUSAL_PREFIX: &[u8] = b"Browser automation requires a paid";

/// The refusal message's memory address, in whichever section holds it.
fn refusal_anchor(buf: &[u8], sections: &[Section]) -> Option<u32> {
  for section in sections {
    let hay = &buf[section.raw.clone()];
    if let Some(pos) = hay
      .windows(REFUSAL_PREFIX.len())
      .position(|w| w == REFUSAL_PREFIX)
    {
      return Some(section.va + pos as u32);
    }
  }
  None
}

/// File offsets, within `code`, of rip-relative `lea` instructions whose
/// target is `anchor_va` — every place the refusal message gets loaded.
fn lea_references(
  buf: &[u8],
  code: std::ops::Range<usize>,
  sections: &[Section],
  anchor_va: u32,
) -> Vec<usize> {
  let mut refs = Vec::new();
  let start = code.start.max(8);
  let end = code.end.min(buf.len().saturating_sub(8));
  for i in start..end {
    // `lea reg, [rip+rel32]`: an optional REX prefix, then opcode 8D and a
    // rip-relative modrm (mod 00, rm 101).
    let (opcode, len) = match (buf[i], buf.get(i + 1)) {
      (0x48 | 0x4C, Some(0x8D)) => (i + 1, 7),
      (0x8D, _) => (i, 6),
      _ => continue,
    };
    if buf[opcode + 1] & 0xC7 != 0x05 {
      continue;
    }
    let Some(site_va) = va_of(i, sections) else {
      continue;
    };
    let rel = i32::from_le_bytes([
      buf[opcode + 2],
      buf[opcode + 3],
      buf[opcode + 4],
      buf[opcode + 5],
    ]);
    if site_va as i64 + len as i64 + rel as i64 == anchor_va as i64 {
      refs.push(i);
    }
  }
  refs
}

/// Whether a `call rel32` answered by `test eax,eax`/`test al,al` sits just
/// before the branch at `branch` — the entitlement check itself.
fn call_test_before(buf: &[u8], branch: usize) -> bool {
  let lo = branch.saturating_sub(24).max(8);
  let hi = branch.saturating_sub(7);
  for p in (lo..=hi).rev() {
    if buf[p] != 0xE8 {
      continue;
    }
    if !matches!(&buf[p + 5..p + 7], [0x85, 0xC0] | [0x84, 0xC0]) {
      continue;
    }
    let after_test = p + 7;
    if branch >= after_test && branch - after_test <= 4 {
      return true;
    }
  }
  false
}

/// The conditional branch enclosing a refusal load at `lea`: a six-byte jcc —
/// or the five-byte jmp that replaced it — whose target lands past the
/// refusal, with the entitlement call and test in front of it.
fn gate_branch_before(buf: &[u8], lea: usize, sections: &[Section]) -> Option<usize> {
  let lea_va = va_of(lea, sections)? as i64;
  let lo = lea.saturating_sub(96).max(8);
  for b in (lo..lea).rev() {
    let (size, rel_at) = match (buf[b], buf.get(b + 1)) {
      (0x0F, Some(0x84 | 0x85)) => (6, b + 2),
      (0xE9, _) => (5, b + 1),
      _ => continue,
    };
    if rel_at + 4 > buf.len() {
      continue;
    }
    let rel = i32::from_le_bytes([
      buf[rel_at],
      buf[rel_at + 1],
      buf[rel_at + 2],
      buf[rel_at + 3],
    ]);
    let Some(branch_va) = va_of(b, sections) else {
      continue;
    };
    // The branch must skip forward over the refusal load, not merely point
    // at something nearby.
    if branch_va as i64 + size as i64 + rel as i64 <= lea_va {
      continue;
    }
    if !call_test_before(buf, b) {
      continue;
    }
    return Some(b);
  }
  None
}

/// Gate sites found through the refusal anchor: every code position that
/// loads the refusal message and is enclosed by the entitlement branch.
/// Deduplicated, because one branch may guard several loads.
fn gate_sites_anchored(
  buf: &[u8],
  code: std::ops::Range<usize>,
  sections: &[Section],
) -> Vec<usize> {
  let Some(anchor) = refusal_anchor(buf, sections) else {
    return Vec::new();
  };
  let mut sites: Vec<usize> = lea_references(buf, code, sections, anchor)
    .iter()
    .filter_map(|&lea| gate_branch_before(buf, lea, sections))
    .collect();
  sites.sort_unstable();
  sites.dedup();
  sites
}

/// Offsets of gate sites by the classic byte signature, patched or not: a
/// `call rel32` answered with `test eax,eax`, a six-byte conditional branch
/// over the refusal, and the refusal's own fingerprint — `cmp eax,2`, then
/// `mov ebx,0x36` — in the bytes the branch skips. Pure, so it can be tested
/// on crafted bytes.
fn gate_sites(buf: &[u8], code: std::ops::Range<usize>) -> Vec<usize> {
  let mut sites = Vec::new();
  let start = code.start.max(8);
  let end = code.end.min(buf.len().saturating_sub(112));
  let mut i = start;
  while i < end {
    if buf[i] == 0xE8 {
      // call rel32 ; test eax,eax (85 c0) or test al,al (84 c0) ; a six-byte
      // conditional branch (0f 8x rel32) — or, once patched, the five-byte
      // jmp (e9 rel32) that replaced it — over the refusal.
      let after_call = i + 5;
      let tested = matches!(
        &buf[after_call..after_call + 2],
        [0x85, 0xC0] | [0x84, 0xC0]
      );
      let conditional = matches!(&buf[after_call + 2..after_call + 4], [0x0F, 0x84 | 0x85]);
      let forced = buf[after_call + 2] == 0xE9;
      if tested && (conditional || forced) {
        let branch = after_call + 2;
        let skipped = &buf[branch + 6..branch + 6 + 96];
        let cmp_eax_2 = skipped.windows(3).any(|w| w == [0x83, 0xF8, 0x02]);
        let mov_ebx_36 = skipped
          .windows(5)
          .any(|w| w == [0xBB, 0x36, 0x00, 0x00, 0x00]);
        if cmp_eax_2 && mov_ebx_36 {
          sites.push(branch);
        }
      }
    }
    i += 1;
  }
  sites
}

/// The one gate site, by anchor first and signature as a fallback. Refusing
/// beats guessing: more than one anchored site is an ambiguity, not a hint to
/// fall back and patch by looser rules.
fn find_gate(
  buf: &[u8],
  code: std::ops::Range<usize>,
  sections: &[Section],
) -> Result<(usize, PatchMethod), PatchError> {
  let mut sites = gate_sites_anchored(buf, code.clone(), sections);
  let mut method = PatchMethod::RefusalAnchor;
  if sites.is_empty() {
    sites = gate_sites(buf, code);
    method = PatchMethod::ByteSignature;
  }
  match sites.as_slice() {
    [one] => Ok((*one, method)),
    [] => Err(PatchError::SignatureNotFound),
    many => Err(PatchError::Ambiguous(many.len())),
  }
}

/// Whether the branch at `branch` is already forced (je/jne → jmp).
fn branch_is_forced(buf: &[u8], branch: usize) -> bool {
  buf[branch] == 0xE9
}

/// Read the dll, find the one gate site, and force its branch. Writes only
/// when the site is not forced yet; the original six bytes stay beside the
/// dll as `chrome.dll.orig` for rollback.
pub fn patch_chrome_dll(dll: &Path) -> Result<PatchReport, PatchError> {
  let buf = std::fs::read(dll).map_err(|e| PatchError::Unreadable(e.to_string()))?;
  let sections = image_sections(&buf)?;
  let code = executable_range(&buf)?;
  let (offset, method) = find_gate(&buf, code, &sections)?;
  let mut original = [0u8; 6];
  original.copy_from_slice(&buf[offset..offset + 6]);
  if branch_is_forced(&buf, offset) {
    return Ok(PatchReport {
      gate_offset: offset,
      original,
      already_patched: true,
      method: method.as_str(),
    });
  }
  // je rel32 (6 bytes) becomes jmp rel32 (5) + nop, landing on the same
  // address: the jmp measures from one byte earlier, so the displacement
  // grows by one.
  let rel = i32::from_le_bytes([
    buf[offset + 2],
    buf[offset + 3],
    buf[offset + 4],
    buf[offset + 5],
  ]);
  let mut patched = buf.clone();
  patched[offset] = 0xE9;
  patched[offset + 1..offset + 5].copy_from_slice(&((rel + 1) as i32).to_le_bytes());
  patched[offset + 5] = 0x90;
  let backup = dll.with_extension("dll.orig");
  if !backup.exists() {
    std::fs::write(&backup, &buf).map_err(|e| PatchError::Unreadable(e.to_string()))?;
  }
  // Replace the whole file atomically enough for an engine binary that
  // no browser has open: write beside, then rename over.
  let staged = dll.with_extension("dll.patching");
  std::fs::write(&staged, &patched).map_err(|e| PatchError::Unreadable(e.to_string()))?;
  std::fs::rename(&staged, dll).map_err(|_| PatchError::ChangedOnDisk)?;
  log::info!(
    "Engine gate patched file={} offset={:#x} was={} method={} branch-forced",
    dll.display(),
    offset,
    original
      .iter()
      .map(|b| format!("{b:02x}"))
      .collect::<Vec<_>>()
      .join(" "),
    method.as_str()
  );
  Ok(PatchReport {
    gate_offset: offset,
    original,
    already_patched: false,
    method: method.as_str(),
  })
}

/// Whether the dll's gate branch is already forced.
pub fn is_patched(dll: &Path) -> Result<bool, PatchError> {
  let buf = std::fs::read(dll).map_err(|e| PatchError::Unreadable(e.to_string()))?;
  let sections = image_sections(&buf)?;
  let code = executable_range(&buf)?;
  let (branch, _) = find_gate(&buf, code, &sections)?;
  Ok(branch_is_forced(&buf, branch))
}

/// Whether an installed Wayfern version runs with its gate forced. `None`
/// when that version is not installed; `Err` when its dll matches no known
/// gate shape, which the engine list surfaces instead of a silent engine.
#[tauri::command]
pub fn is_wayfern_patched(version: String) -> Result<Option<bool>, String> {
  let dll = chrome_dll_of(&crate::app_dirs::binaries_dir(), &version);
  if !dll.exists() {
    return Ok(None);
  }
  is_patched(&dll).map(Some).map_err(|why| why.to_string())
}

/// The chrome.dll of an installed Wayfern version, by the layout the
/// downloader lays out: binaries/wayfern/<version>/chrome.dll.
pub fn chrome_dll_of(binaries_dir: &Path, version: &str) -> PathBuf {
  binaries_dir
    .join("wayfern")
    .join(version)
    .join("chrome.dll")
}

/// Launch the patched engine headless and prove its gate is actually open:
/// evaluate through CDP on a throwaway profile. A forced branch that does
/// not survive to behaviour — a different gate, a second check, a broken
/// build — fails here, and the caller removes the engine instead of shipping
/// a silent refusal.
pub async fn verify_gate_open(engine_dir: &Path) -> Result<(), String> {
  let exe = engine_dir.join("chrome.exe");
  if !exe.is_file() {
    return Err("chrome.exe not found in the engine directory".to_string());
  }
  let user_data = std::env::temp_dir().join(format!("wayfern-smoke-{}", std::process::id()));
  let _ = std::fs::remove_dir_all(&user_data);
  std::fs::create_dir_all(&user_data).map_err(|e| format!("smoke profile dir: {e}"))?;

  let mut child = std::process::Command::new(&exe)
    .args([
      "--headless=new",
      "--remote-debugging-port=0",
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-gpu",
    ])
    .arg(format!("--user-data-dir={}", user_data.display()))
    .arg("about:blank")
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
    .map_err(|e| format!("failed to launch engine: {e}"))?;

  let verdict = smoke_probe(&user_data).await;
  let _ = child.kill();
  let _ = child.wait();
  // Give the dying engine a moment to let go of its profile files, then
  // remove them; a locked leftover is harmless in the temp dir.
  tokio::time::sleep(std::time::Duration::from_millis(750)).await;
  let _ = std::fs::remove_dir_all(&user_data);
  verdict
}

/// A CDP target's debugging socket, present on automatable targets.
fn target_socket(target: &serde_json::Value) -> Option<&str> {
  target.get("webSocketDebuggerUrl").and_then(|v| v.as_str())
}

/// The CDP half of the smoke: find the debugging port, open a page socket,
/// evaluate, and expect the answer a closed gate would have swallowed.
async fn smoke_probe(user_data: &Path) -> Result<(), String> {
  use futures_util::{sink::SinkExt, stream::StreamExt};

  // The engine writes DevToolsActivePort once it picks a port.
  let port_file = user_data.join("DevToolsActivePort");
  let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
  let port = loop {
    if std::time::Instant::now() > deadline {
      return Err("engine did not open a debugging port in 30s".to_string());
    }
    if let Ok(text) = std::fs::read_to_string(&port_file) {
      if let Some(port) = text
        .lines()
        .next()
        .and_then(|line| line.trim().parse::<u16>().ok())
      {
        break port;
      }
    }
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
  };

  let http = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(10))
    .build()
    .map_err(|e| format!("smoke http client: {e}"))?;
  let targets: serde_json::Value = http
    .get(format!("http://127.0.0.1:{port}/json/list"))
    .send()
    .await
    .map_err(|e| format!("smoke /json/list: {e}"))?
    .json()
    .await
    .map_err(|e| format!("smoke /json/list body: {e}"))?;
  let ws_url = targets
    .as_array()
    .and_then(|list| {
      // The vendor build reports its blank target as "other", so prefer a
      // page and fall back to whichever target offers a socket.
      list
        .iter()
        .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
        .and_then(target_socket)
        .or_else(|| list.iter().find_map(|t| target_socket(t)))
        .map(str::to_string)
    })
    .ok_or_else(|| "smoke: no page target with a debugger socket".to_string())?;

  let (mut stream, _) = tokio_tungstenite::connect_async(ws_url)
    .await
    .map_err(|e| format!("smoke websocket: {e}"))?;
  let evaluate = serde_json::json!({
    "id": 1,
    "method": "Runtime.evaluate",
    "params": { "expression": "6*7", "returnByValue": true }
  });
  stream
    .send(tokio_tungstenite::tungstenite::Message::Text(
      evaluate.to_string().into(),
    ))
    .await
    .map_err(|e| format!("smoke send: {e}"))?;

  let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
  loop {
    if std::time::Instant::now() > deadline {
      return Err("smoke: no CDP answer in 20s".to_string());
    }
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
      .await
      .map_err(|_| "smoke: CDP socket stalled".to_string())?
      .ok_or_else(|| "smoke: CDP socket closed".to_string())?
      .map_err(|e| format!("smoke read: {e}"))?;
    if let tokio_tungstenite::tungstenite::Message::Text(text) = frame {
      let answer: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("smoke: bad CDP frame: {e}"))?;
      if answer.get("id").and_then(|v| v.as_u64()) == Some(1) {
        if let Some(err) = answer.get("error") {
          return Err(format!("smoke: CDP refused: {err}"));
        }
        let value = answer
          .pointer("/result/result/value")
          .and_then(serde_json::Value::as_i64);
        return match value {
          Some(42) => Ok(()),
          other => Err(format!(
            "smoke: automation refused (evaluate answered {other:?})"
          )),
        };
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The gate's code shape: call, test, a six-byte branch over a refusal
  /// that compares eax with 2 and loads error 0x36. Starts at 0 so the
  /// offset math is easy to read.
  fn gate_code(forced: bool) -> Vec<u8> {
    let mut code = vec![0u8; 256];
    let call = 123usize;
    let test = call + 5;
    let branch = test + 2; // 130
    code[call] = 0xE8; // call rel32
    code[test] = 0x85; // test eax,eax
    code[test + 1] = 0xC0;
    if forced {
      code[branch] = 0xE9; // jmp rel32
      code[branch + 1..branch + 5].copy_from_slice(&0x20i32.to_le_bytes());
      code[branch + 5] = 0x90; // nop
    } else {
      code[branch] = 0x0F; // je rel32
      code[branch + 1] = 0x84;
      code[branch + 2..branch + 6].copy_from_slice(&0x1Fi32.to_le_bytes());
    }
    let refusal = branch + 6;
    code[refusal..refusal + 3].copy_from_slice(&[0x83, 0xF8, 0x02]); // cmp eax,2
    code[refusal + 8..refusal + 13].copy_from_slice(&[0xBB, 0x36, 0x00, 0x00, 0x00]); // mov ebx,0x36
    code
  }

  /// `gate_code` wrapped in the smallest PE that marks one executable
  /// section over it, so the file-level paths can run on it.
  fn synthetic_gate_dll(forced: bool) -> Vec<u8> {
    let code = gate_code(forced);
    let mut buf = vec![0u8; 0x100];
    buf[0..2].copy_from_slice(b"MZ");
    let pe = 0x40usize;
    buf[0x3c..0x40].copy_from_slice(&(pe as u32).to_le_bytes());
    buf[pe..pe + 4].copy_from_slice(b"PE\0\0");
    buf[pe + 6..pe + 8].copy_from_slice(&1u16.to_le_bytes()); // one section
    buf[pe + 20..pe + 22].copy_from_slice(&0u16.to_le_bytes()); // no optional header
    let table = pe + 24;
    buf[table..table + 5].copy_from_slice(b".text");
    buf[table + 16..table + 20].copy_from_slice(&(code.len() as u32).to_le_bytes());
    buf[table + 20..table + 24].copy_from_slice(&(0x100u32).to_le_bytes());
    buf[table + 36..table + 40].copy_from_slice(&0x2000_0000u32.to_le_bytes());
    buf.extend_from_slice(&code);
    buf
  }

  /// The code file offset the anchored fixture below lays its `.text` at.
  const ANCHORED_CODE_FILE: usize = 0x200;

  /// A PE whose code loads a refusal string by rip-relative lea and branches
  /// over the load — the shape of 154/155 as a future build may compile it:
  /// no `cmp eax,2`, no fixed 0x36 length, the message reworded past the
  /// anchor prefix. `noise_lea` adds a second load of the same string in code
  /// that is not a gate, which must not conjure a second site.
  fn anchored_gate_dll(forced: bool, noise_lea: bool) -> Vec<u8> {
    const TEXT_VA: u32 = 0x1000;
    const RDATA_VA: u32 = 0x2000;
    let message: &[u8] = b"Browser automation requires a paid Friends plan.\0";

    let mut code = vec![0u8; 0x100];
    // call status_fn at +0x80; test eax,eax; je allow.
    code[0x00] = 0xE8;
    code[0x01..0x05].copy_from_slice(&(0x80i32 - 0x05).to_le_bytes());
    code[0x05] = 0x85;
    code[0x06] = 0xC0;
    let branch = 0x07usize;
    let allow = 0x30usize;
    if forced {
      code[branch] = 0xE9;
      code[branch + 1..branch + 5]
        .copy_from_slice(&((allow as i32) - (branch as i32) - 5).to_le_bytes());
      code[branch + 5] = 0x90;
    } else {
      code[branch] = 0x0F;
      code[branch + 1] = 0x84;
      code[branch + 2..branch + 6]
        .copy_from_slice(&((allow as i32) - (branch as i32) - 6).to_le_bytes());
    }
    // The refusal: lea rdx,[rip+string]; mov ebx,<real length>; call report.
    let lea = 0x0Dusize;
    code[lea] = 0x48;
    code[lea + 1] = 0x8D;
    code[lea + 2] = 0x15; // rdx
    let lea_va = TEXT_VA + lea as u32;
    code[lea + 3..lea + 7]
      .copy_from_slice(&((RDATA_VA as i32) - (lea_va as i32) - 7).to_le_bytes());
    code[lea + 7] = 0xBB; // mov ebx, <len> — the real length, not 0x36
    code[lea + 8..lea + 12].copy_from_slice(&((message.len() - 1) as u32).to_le_bytes());
    code[lea + 12] = 0xE8; // call report
    code[lea + 13..lea + 17].copy_from_slice(&0x100i32.to_le_bytes());
    code[allow] = 0xC3; // allow: ret
                        // status_fn: xor eax,eax; ret.
    code[0x80] = 0x31;
    code[0x81] = 0xC0;
    code[0x82] = 0xC3;
    if noise_lea {
      // A second load of the same message with no entitlement branch around
      // it — a log line, not a gate.
      let noise = 0x60usize;
      code[noise] = 0x48;
      code[noise + 1] = 0x8D;
      code[noise + 2] = 0x15;
      let noise_va = TEXT_VA + noise as u32;
      code[noise + 3..noise + 7]
        .copy_from_slice(&((RDATA_VA as i32) - (noise_va as i32) - 7).to_le_bytes());
    }

    let rdata_file = ANCHORED_CODE_FILE + code.len();
    let mut buf = vec![0u8; ANCHORED_CODE_FILE];
    buf[0..2].copy_from_slice(b"MZ");
    let pe = 0x40usize;
    buf[0x3c..0x40].copy_from_slice(&(pe as u32).to_le_bytes());
    buf[pe..pe + 4].copy_from_slice(b"PE\0\0");
    buf[pe + 6..pe + 8].copy_from_slice(&2u16.to_le_bytes()); // two sections
    buf[pe + 20..pe + 22].copy_from_slice(&0u16.to_le_bytes());
    let table = pe + 24;
    // .text
    buf[table..table + 5].copy_from_slice(b".text");
    buf[table + 12..table + 16].copy_from_slice(&TEXT_VA.to_le_bytes());
    buf[table + 16..table + 20].copy_from_slice(&(code.len() as u32).to_le_bytes());
    buf[table + 20..table + 24].copy_from_slice(&(ANCHORED_CODE_FILE as u32).to_le_bytes());
    buf[table + 36..table + 40].copy_from_slice(&0x2000_0000u32.to_le_bytes());
    // .rdata
    let second = table + 40;
    buf[second..second + 6].copy_from_slice(b".rdata");
    buf[second + 12..second + 16].copy_from_slice(&RDATA_VA.to_le_bytes());
    buf[second + 16..second + 20].copy_from_slice(&(message.len() as u32).to_le_bytes());
    buf[second + 20..second + 24].copy_from_slice(&(rdata_file as u32).to_le_bytes());
    buf.extend_from_slice(&code);
    buf.extend_from_slice(message);
    buf
  }

  #[test]
  fn the_signature_finds_the_gate() {
    let code = gate_code(false);
    assert_eq!(gate_sites(&code, 0..code.len()), vec![130]);
  }

  #[test]
  fn a_forced_gate_is_still_the_gate() {
    let code = gate_code(true);
    assert_eq!(gate_sites(&code, 0..code.len()), vec![130]);
  }

  #[test]
  fn no_gate_in_plain_bytes() {
    let code = vec![0x90u8; 512];
    assert!(gate_sites(&code, 0..code.len()).is_empty());
  }

  #[test]
  fn a_refusal_without_the_error_code_is_not_the_gate() {
    let mut code = gate_code(false);
    // Drop the mov ebx,0x36 fingerprint: this is some other branch.
    for byte in &mut code[144..149] {
      *byte = 0x90;
    }
    assert!(gate_sites(&code, 0..code.len()).is_empty());
  }

  #[test]
  fn the_anchor_finds_a_recompiled_gate() {
    let buf = anchored_gate_dll(false, false);
    let sections = image_sections(&buf).unwrap();
    let code = executable_range(&buf).unwrap();
    // The classic signature cannot see this build: no cmp eax,2, no 0x36.
    assert!(gate_sites(&buf, code.clone()).is_empty());
    // The anchor can: one gate at the je in front of the refusal load.
    assert_eq!(
      gate_sites_anchored(&buf, code, &sections),
      vec![ANCHORED_CODE_FILE + 0x07]
    );
  }

  #[test]
  fn a_noise_load_of_the_same_string_is_not_a_second_gate() {
    let buf = anchored_gate_dll(false, true);
    let sections = image_sections(&buf).unwrap();
    let code = executable_range(&buf).unwrap();
    assert_eq!(
      gate_sites_anchored(&buf, code, &sections),
      vec![ANCHORED_CODE_FILE + 0x07]
    );
  }

  #[test]
  fn patching_by_anchor_forces_the_branch_and_is_idempotent() {
    let dir = std::env::temp_dir().join(format!("engine-patch-v2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dll = dir.join("chrome.dll");
    std::fs::write(&dll, anchored_gate_dll(false, false)).unwrap();
    let report = patch_chrome_dll(&dll).expect("patch");
    assert!(!report.already_patched);
    assert_eq!(report.method, "refusal-anchor");
    assert_eq!(report.gate_offset, ANCHORED_CODE_FILE + 0x07);
    let patched = std::fs::read(&dll).unwrap();
    assert_eq!(patched[report.gate_offset], 0xE9);
    assert_eq!(patched[report.gate_offset + 5], 0x90);
    // The jmp lands where the je did.
    let old_rel = i32::from_le_bytes([
      report.original[2],
      report.original[3],
      report.original[4],
      report.original[5],
    ]);
    let new_rel = i32::from_le_bytes([
      patched[report.gate_offset + 1],
      patched[report.gate_offset + 2],
      patched[report.gate_offset + 3],
      patched[report.gate_offset + 4],
    ]);
    assert_eq!(old_rel + 1, new_rel);
    // A forced gate is still found — by anchor, not only by signature.
    let again = patch_chrome_dll(&dll).expect("idempotent");
    assert!(again.already_patched);
    assert_eq!(again.method, "refusal-anchor");
    assert!(is_patched(&dll).unwrap());
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn patching_forces_the_branch_and_is_idempotent() {
    let dir = std::env::temp_dir().join(format!("engine-patch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dll = dir.join("chrome.dll");
    std::fs::write(&dll, synthetic_gate_dll(false)).unwrap();
    let report = patch_chrome_dll(&dll).expect("patch");
    assert!(!report.already_patched);
    assert_eq!(report.gate_offset, 0x100 + 130);
    assert_eq!(report.method, "byte-signature");
    let patched = std::fs::read(&dll).unwrap();
    assert_eq!(patched[report.gate_offset], 0xE9);
    assert_eq!(patched[report.gate_offset + 5], 0x90);
    // The jump lands where the je did: displacement grew by one.
    let old_rel = i32::from_le_bytes([
      report.original[2],
      report.original[3],
      report.original[4],
      report.original[5],
    ]);
    let new_rel = i32::from_le_bytes([
      patched[report.gate_offset + 1],
      patched[report.gate_offset + 2],
      patched[report.gate_offset + 3],
      patched[report.gate_offset + 4],
    ]);
    assert_eq!(old_rel + 1, new_rel);
    assert!(dll.with_extension("dll.orig").exists());
    let again = patch_chrome_dll(&dll).expect("idempotent");
    assert!(again.already_patched);
    assert!(is_patched(&dll).unwrap());
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn a_dll_without_the_gate_is_refused_untouched() {
    let dir = std::env::temp_dir().join(format!("engine-patch-none-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dll = dir.join("chrome.dll");
    std::fs::write(&dll, vec![0x90u8; 1024]).unwrap();
    match patch_chrome_dll(&dll) {
      Err(PatchError::Unreadable(_)) | Err(PatchError::NoCode) => {}
      other => panic!("expected refusal, got {other:?}"),
    }
    std::fs::remove_dir_all(&dir).ok();
  }

  /// Proof against a real engine, not a fixture: the anchor must land on the
  /// same site the manual hunt found. Run manually with the vendor's
  /// unpatched dll and the documented offset, e.g.
  /// `WAYFERN_ORIG=<…>/wayfern/155.0.8059.79/chrome.dll.orig \
  ///  WAYFERN_EXPECT=0x40a381b cargo test --lib engine_patch -- --ignored`
  #[test]
  #[ignore = "needs a real chrome.dll.orig; set WAYFERN_ORIG and WAYFERN_EXPECT"]
  fn the_real_engine_anchor_matches_the_manual_hunt() {
    let Ok(path) = std::env::var("WAYFERN_ORIG") else {
      return;
    };
    let buf = std::fs::read(&path).expect("fixture readable");
    let sections = image_sections(&buf).unwrap();
    let code = executable_range(&buf).unwrap();
    let sites = gate_sites_anchored(&buf, code, &sections);
    assert_eq!(sites.len(), 1, "expected exactly one anchored gate site");
    if let Ok(expect) = std::env::var("WAYFERN_EXPECT") {
      let expect = usize::from_str_radix(expect.trim_start_matches("0x"), 16).unwrap();
      assert_eq!(sites[0], expect, "anchor disagrees with the manual hunt");
    }
  }

  /// Live smoke against a real installed engine, e.g.
  /// `WAYFERN_SMOKE=<…>/binaries/wayfern/155.0.8059.79 \
  ///  cargo test --lib engine_patch -- --ignored`
  #[tokio::test]
  #[ignore = "launches a real engine; set WAYFERN_SMOKE to the engine dir"]
  async fn the_real_engine_passes_the_smoke() {
    let Ok(dir) = std::env::var("WAYFERN_SMOKE") else {
      return;
    };
    verify_gate_open(Path::new(&dir))
      .await
      .expect("the patched engine must automate");
  }
}
