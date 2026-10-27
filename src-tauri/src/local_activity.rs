//! The local activity journal: what this desktop actually did, recorded by the
//! app itself. In the vendor product this data lived in the cloud ("team
//! activity"); the local product has no cloud, so the same surface reads this
//! journal instead — launches, cookie-bot runs, engine installs, sync
//! sessions. Append-only JSONL, capped, oldest entries dropped first.

use crate::app_dirs;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Mutex;

/// Entries kept on disk. Enough for a working history without unbounded
/// growth on the system drive.
const MAX_ENTRIES: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityEntry {
  /// RFC 3339 timestamp, UTC.
  pub at: String,
  /// Machine-readable kind: `profile.launched`, `cookiebot.run`, …
  pub kind: String,
  /// Human-readable subject, e.g. the profile name or engine version.
  pub subject: String,
  /// Optional free-form details (counts, outcomes, versions).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub details: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum JournalError {
  #[error("activity journal io: {0}")]
  Io(#[from] std::io::Error),
  #[error("activity journal serde: {0}")]
  Serde(#[from] serde_json::Error),
}

/// Test-only journal directory; when set it wins over the real data dir.
#[cfg(test)]
static TEST_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

fn journal_path() -> PathBuf {
  #[cfg(test)]
  if let Some(dir) = TEST_DIR.lock().unwrap_or_else(|p| p.into_inner()).clone() {
    return dir.join("local_activity.jsonl");
  }
  app_dirs::data_dir().join("local_activity.jsonl")
}

static JOURNAL_LOCK: Mutex<()> = Mutex::new(());

/// Record one entry. Failures are reported, not fatal: a journal write must
/// never break the operation it describes.
pub fn record(kind: &str, subject: &str, details: Option<&str>) -> Result<(), JournalError> {
  let entry = ActivityEntry {
    at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    kind: kind.to_string(),
    subject: subject.to_string(),
    details: details.map(str::to_string),
  };
  let _guard = JOURNAL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
  let path = journal_path();
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent)?;
  }
  let mut line = serde_json::to_string(&entry)?;
  line.push('\n');
  let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
  file.write_all(line.as_bytes())?;
  trim_to_cap(&path)?;
  Ok(())
}

/// Read the journal, newest first, optionally filtered by kind prefix
/// (e.g. `profile.` or `cookiebot.`), capped to `limit`.
pub fn list(kind_prefix: Option<&str>, limit: usize) -> Result<Vec<ActivityEntry>, JournalError> {
  let _guard = JOURNAL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
  let path = journal_path();
  if !path.exists() {
    return Ok(Vec::new());
  }
  let file = File::open(&path)?;
  let reader = BufReader::new(file);
  let mut entries = Vec::new();
  for line in reader.lines() {
    let line = line?;
    if line.trim().is_empty() {
      continue;
    }
    // One corrupted line must not erase the history around it.
    let Ok(entry) = serde_json::from_str::<ActivityEntry>(&line) else {
      continue;
    };
    if let Some(prefix) = kind_prefix {
      if !entry.kind.starts_with(prefix) {
        continue;
      }
    }
    entries.push(entry);
  }
  entries.reverse();
  entries.truncate(limit);
  Ok(entries)
}

/// Drop the oldest lines until the journal is at or below the cap.
fn trim_to_cap(path: &std::path::Path) -> Result<(), JournalError> {
  let file = File::open(path)?;
  let mut lines: Vec<String> = BufReader::new(file)
    .lines()
    .collect::<Result<Vec<_>, _>>()?;
  if lines.len() <= MAX_ENTRIES {
    return Ok(());
  }
  let keep = lines.split_off(lines.len() - MAX_ENTRIES);
  let mut file = File::create(path)?;
  for line in keep {
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
  }
  Ok(())
}

/// Newest-first page of the local journal for the UI. The local product's
/// answer to the cloud "team activity" feed: same surface, local truth.
#[tauri::command]
pub fn list_local_activity(
  kind_prefix: Option<String>,
  limit: Option<usize>,
) -> Result<Vec<ActivityEntry>, String> {
  list(kind_prefix.as_deref(), limit.unwrap_or(200).min(2000)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Scope the journal to a fresh temp dir for the duration of the closure.
  /// Tests hold this lock: the directory override is process-global.
  static TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
  fn with_test_journal(f: impl FnOnce(&std::path::Path)) {
    let _guard = TEST_GUARD.lock().unwrap_or_else(|p| p.into_inner());
    let dir = std::env::temp_dir().join(format!(
      "donut-activity-{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    *TEST_DIR.lock().unwrap() = Some(dir.clone());
    f(&dir);
    *TEST_DIR.lock().unwrap() = None;
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn entries_roundtrip_newest_first_and_filter() {
    with_test_journal(|_| {
      record("profile.launched", "Alpha", Some("wayfern 155")).unwrap();
      record("cookiebot.run", "Alpha", Some("visits=12")).unwrap();
      record("profile.stopped", "Alpha", None).unwrap();

      let all = list(None, 10).unwrap();
      assert_eq!(all.len(), 3);
      assert_eq!(all[0].kind, "profile.stopped");
      assert_eq!(all[2].details.as_deref(), Some("wayfern 155"));
      let filtered = list(Some("cookiebot."), 10).unwrap();
      assert_eq!(filtered.len(), 1);
      assert_eq!(filtered[0].subject, "Alpha");
    });
  }

  #[test]
  fn corrupted_lines_are_skipped_not_fatal() {
    with_test_journal(|dir| {
      std::fs::write(dir.join("local_activity.jsonl"), "{not json}\n").unwrap();
      assert!(list(None, 10).unwrap().is_empty());
      // Recording still works after corruption.
      record("engine.installed", "wayfern 155", None).unwrap();
      let entries = list(None, 10).unwrap();
      assert_eq!(entries.len(), 1);
      assert_eq!(entries[0].kind, "engine.installed");
    });
  }
}
