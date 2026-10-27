//! The Wayfern automation gate, found and opened by signature.
//!
//! The vendor engine ships a licence gate on its CDP automation surface: a
//! call that answers whether automation is allowed, `test eax,eax`, and a
//! branch that skips a refusal block when the answer says no. A fresh vendor
//! build refuses automation with error 0x36; the installed engines here run
//! with that branch forced (see docs/engine-patches.md for the history of
//! 151/154/155, patched first by hand, now by this module).
//!
//! The scanner does not guess offsets: it looks for the whole shape —
//! `call rel32; test eax,eax; je rel32` followed within a few bytes by the
//! refusal block's own fingerprint (`cmp eax,2`, then `mov ebx,0x36`) — and
//! only patches when exactly one site in the executable section matches.
//! A new engine version either matches or is left untouched and reported,
//! which is what makes engine updates safe to attempt at all.

use std::path::{Path, PathBuf};

/// Why a patch could not happen, in the caller's words.
#[derive(Debug)]
pub enum PatchError {
  /// The file could not be read or is not a PE.
  Unreadable(String),
  /// The PE has no executable section to scan.
  NoCode,
  /// The gate signature did not appear — the engine differs from every
  /// version seen so far. The file is untouched.
  SignatureNotFound,
  /// The signature appears more than once; patching blind could corrupt.
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
      PatchError::SignatureNotFound => write!(f, "the automation gate signature was not found; this engine version is new and needs a new patch"),
      PatchError::Ambiguous(n) => write!(f, "the automation gate signature matched {n} sites; refusing to patch blind"),
      PatchError::ChangedOnDisk => write!(f, "chrome.dll changed while being patched"),
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
}

/// The executable section's byte range of a PE, as file offsets.
fn executable_range(buf: &[u8]) -> Result<std::ops::Range<usize>, PatchError> {
  let read_u16 = |off: usize| u16::from_le_bytes([buf[off], buf[off + 1]]);
  let read_u32 = |off: usize| {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
  };
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
  let mut best: Option<(u32, std::ops::Range<usize>)> = None;
  for i in 0..sections {
    let off = table + i * 40;
    let raw_size = read_u32(off + 16) as usize;
    let raw = read_u32(off + 20) as usize;
    let characteristics = read_u32(off + 36);
    const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
    if characteristics & IMAGE_SCN_MEM_EXECUTE == 0 || raw == 0 || raw_size == 0 {
      continue;
    }
    let range = raw..(raw + raw_size).min(buf.len());
    if best.as_ref().is_none_or(|(_, r)| range.len() > r.len()) {
      best = Some((characteristics, range));
    }
  }
  best.map(|(_, range)| range).ok_or(PatchError::NoCode)
}

/// Offsets of gate sites in `buf`, patched or not: a `call rel32` answered
/// with `test eax,eax`, a six-byte conditional branch over the refusal, and
/// the refusal's own fingerprint — `cmp eax,2`, then `mov ebx,0x36` — in the
/// bytes the branch skips. Pure, so it can be tested on crafted bytes.
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
      let tested = matches!(&buf[after_call..after_call + 2], [0x85, 0xC0] | [0x84, 0xC0]);
      let conditional = matches!(&buf[after_call + 2..after_call + 4], [0x0F, 0x84 | 0x85]);
      let forced = buf[after_call + 2] == 0xE9;
      if tested && (conditional || forced) {
        let branch = after_call + 2;
        let skipped = &buf[branch + 6..branch + 6 + 96];
        let cmp_eax_2 = skipped.windows(3).any(|w| w == [0x83, 0xF8, 0x02]);
        let mov_ebx_36 = skipped.windows(5).any(|w| w == [0xBB, 0x36, 0x00, 0x00, 0x00]);
        if cmp_eax_2 && mov_ebx_36 {
          sites.push(branch);
        }
      }
    }
    i += 1;
  }
  sites
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
  let code = executable_range(&buf)?;
  let sites = gate_sites(&buf, code.clone());
  match sites.as_slice() {
    [branch] => {
      let offset = *branch;
      let mut original = [0u8; 6];
      original.copy_from_slice(&buf[offset..offset + 6]);
      if branch_is_forced(&buf, offset) {
        return Ok(PatchReport {
          gate_offset: offset,
          original,
          already_patched: true,
        });
      }
      // je rel32 (6 bytes) becomes jmp rel32 (5) + nop, landing on the same
      // address: the jmp measures from one byte earlier, so the displacement
      // grows by one.
      let rel = i32::from_le_bytes([buf[offset + 2], buf[offset + 3], buf[offset + 4], buf[offset + 5]]);
      let mut patched = buf.clone();
      patched[offset] = 0xE9;
      patched[offset + 1..offset + 5]
        .copy_from_slice(&((rel + 1) as i32).to_le_bytes());
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
        "Engine gate patched file={} offset={:#x} was={} branch-forced",
        dll.display(),
        offset,
        original
          .iter()
          .map(|b| format!("{b:02x}"))
          .collect::<Vec<_>>()
          .join(" ")
      );
      Ok(PatchReport {
        gate_offset: offset,
        original,
        already_patched: false,
      })
    }
    [] => Err(PatchError::SignatureNotFound),
    many => Err(PatchError::Ambiguous(many.len())),
  }
}

/// Whether the dll's gate branch is already forced.
pub fn is_patched(dll: &Path) -> Result<bool, PatchError> {
  let buf = std::fs::read(dll).map_err(|e| PatchError::Unreadable(e.to_string()))?;
  let code = executable_range(&buf)?;
  match gate_sites(&buf, code).as_slice() {
    [branch] => Ok(branch_is_forced(&buf, *branch)),
    [] => Err(PatchError::SignatureNotFound),
    many => Err(PatchError::Ambiguous(many.len())),
  }
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
  is_patched(&dll)
    .map(Some)
    .map_err(|why| why.to_string())
}

/// The chrome.dll of an installed Wayfern version, by the layout the
/// downloader lays out: binaries/wayfern/<version>/chrome.dll.
pub fn chrome_dll_of(binaries_dir: &Path, version: &str) -> PathBuf {
  binaries_dir
    .join("wayfern")
    .join(version)
    .join("chrome.dll")
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
    code[refusal + 8..refusal + 13].copy_from_slice(&[0xBB, 0x36, 0x00, 0x00, 0x00]);
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
  fn patching_forces_the_branch_and_is_idempotent() {
    let dir = std::env::temp_dir().join(format!("engine-patch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dll = dir.join("chrome.dll");
    std::fs::write(&dll, synthetic_gate_dll(false)).unwrap();
    let report = patch_chrome_dll(&dll).expect("patch");
    assert!(!report.already_patched);
    assert_eq!(report.gate_offset, 0x100 + 130);
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
}
