//! A tiny JSON file store for the local automation engines (free build).
//!
//! The vendor build keeps schedules, recipes and runs in the cloud; the free
//! build has no cloud, so each surface keeps one JSON document under
//! `<data>/automation/`. The store is deliberately boring: a mutex, an atomic
//! write (temp file + rename, the same dance `SettingsManager` does), and
//! `serde` defaults on the way in so a truncated or hand-edited file degrades
//! to an empty store instead of taking the whole surface down.
//!
//! One machine, one store, one writer at a time. There is no merge protocol:
//! the file is local state, not a document shared between devices.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// Where every automation document lives: `<data>/automation/`.
pub(crate) fn dir() -> PathBuf {
  crate::app_dirs::data_subdir().join("automation")
}

/// Write `content` to `path` in one step: a sibling first, then renamed into
/// place, so a reader mid-write sees the old document or the new one, never a
/// half-written file that parses as empty.
fn write_whole(path: &std::path::Path, content: &[u8]) -> std::io::Result<()> {
  let staging = path.with_extension("json.tmp");
  fs::write(&staging, content)?;
  if let Err(e) = fs::rename(&staging, path) {
    let _ = fs::remove_file(&staging);
    return Err(e);
  }
  Ok(())
}

/// One JSON document of type `T`, cached in memory after the first load.
pub(crate) struct LocalStore<T> {
  path: PathBuf,
  state: Mutex<Option<T>>,
}

impl<T> LocalStore<T>
where
  T: Default + serde::Serialize + serde::de::DeserializeOwned + Clone,
{
  /// A store backed by `<data>/automation/<name>.json`.
  pub(crate) fn new(name: &str) -> Self {
    Self {
      path: dir().join(format!("{name}.json")),
      state: Mutex::new(None),
    }
  }

  /// A store at an explicit path, for tests.
  #[cfg(test)]
  pub(crate) fn at(path: PathBuf) -> Self {
    Self {
      path,
      state: Mutex::new(None),
    }
  }

  /// Read the document from disk, or the default when it is missing or
  /// unreadable. A corrupt file is copied aside once and then treated as
  /// empty — local state, not user data; the surface must stay usable, and
  /// the copy keeps the damage recoverable by hand.
  fn load(&self) -> T {
    match fs::read_to_string(&self.path) {
      Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
        log::warn!(
          "Automation store {} unreadable, starting empty: {e}",
          self.path.display()
        );
        let aside = self.path.with_extension("json.corrupt");
        if !aside.exists() && fs::copy(&self.path, &aside).is_ok() {
          log::warn!(
            "Corrupt automation document copied aside to {}",
            aside.display()
          );
        }
        T::default()
      }),
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => T::default(),
      Err(e) => {
        log::warn!(
          "Automation store {} unreadable, starting empty: {e}",
          self.path.display()
        );
        T::default()
      }
    }
  }

  /// The current document, loading it once and caching it.
  pub(crate) fn read(&self) -> T {
    let mut guard = self.state.lock().unwrap();
    if guard.is_none() {
      *guard = Some(self.load());
    }
    guard.clone().unwrap_or_default()
  }

  /// Persist `value` and publish it to the cache. The caller must already
  /// hold the state lock through `guard`; the in-memory copy is only updated
  /// after the write succeeded, so a failed save cannot masquerade as state.
  fn write_locked(&self, guard: &mut Option<T>, value: T) -> Result<(), String> {
    if let Some(parent) = self.path.parent() {
      fs::create_dir_all(parent).map_err(|e| format!("automation store dir: {e}"))?;
    }
    let text =
      serde_json::to_string_pretty(&value).map_err(|e| format!("automation store encode: {e}"))?;
    write_whole(&self.path, text.as_bytes()).map_err(|e| format!("automation store write: {e}"))?;
    *guard = Some(value);
    Ok(())
  }

  /// Read, change, write — under the store's own lock for the whole cycle,
  /// so a scheduler tick and a UI edit cannot lose one another's write (a
  /// dropped fired-slot key would double-fire a schedule). A nested `write`
  /// would self-deadlock; that is what `write_locked` is for. The closure
  /// reports its own failure the same way a failed save does: one flat
  /// `Result` per update, so no caller can swallow an inner layer.
  pub(crate) fn update<R, E>(
    &self,
    change: impl FnOnce(&mut T) -> Result<R, E>,
  ) -> Result<R, String>
  where
    E: std::fmt::Display,
  {
    let mut guard = self.state.lock().unwrap();
    if guard.is_none() {
      *guard = Some(self.load());
    }
    let mut value = guard.clone().unwrap_or_default();
    let outcome = change(&mut value).map_err(|e| e.to_string())?;
    self.write_locked(&mut guard, value)?;
    Ok(outcome)
  }

  /// Drop the in-memory cache so the next read goes to disk again (tests).
  #[cfg(test)]
  pub(crate) fn reset(&self) {
    *self.state.lock().unwrap() = None;
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::app_dirs;

  #[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
  struct Doc {
    #[serde(default)]
    items: Vec<u32>,
  }

  fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("donut-local-store-{test}"));
    let _ = fs::remove_dir_all(&dir);
    dir
  }

  #[test]
  fn a_missing_document_reads_as_the_default() {
    let dir = scratch("missing");
    let store: LocalStore<Doc> = LocalStore::at(dir.join("a.json"));
    assert_eq!(store.read(), Doc::default());
  }

  #[test]
  fn a_written_document_survives_a_cache_reset() {
    let dir = scratch("roundtrip");
    let store: LocalStore<Doc> = LocalStore::at(dir.join("a.json"));
    store
      .update(|doc| {
        *doc = Doc {
          items: vec![1, 2, 3],
        };
        Ok::<(), String>(())
      })
      .expect("write succeeds");
    store.reset();
    assert_eq!(store.read().items, vec![1, 2, 3]);
  }

  #[test]
  fn a_truncated_document_degrades_to_empty_not_a_panic() {
    let dir = scratch("truncated");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.json");
    fs::write(&path, b"{\"items\": [1,").unwrap();
    let store: LocalStore<Doc> = LocalStore::at(path);
    assert_eq!(store.read(), Doc::default());
  }

  #[test]
  fn update_applies_and_persists_in_one_step() {
    let dir = scratch("update");
    let store: LocalStore<Doc> = LocalStore::at(dir.join("a.json"));
    store
      .update(|doc| {
        doc.items.push(7);
        Ok::<(), String>(())
      })
      .expect("update");
    store
      .update(|doc| {
        doc.items.push(8);
        Ok::<(), String>(())
      })
      .expect("update");
    store.reset();
    assert_eq!(store.read().items, vec![7, 8]);
  }

  #[test]
  fn a_corrupt_document_is_copied_aside_once() {
    let dir = scratch("corrupt-aside");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.json");
    fs::write(&path, b"{\"items\": [1,").unwrap();
    let store: LocalStore<Doc> = LocalStore::at(path.clone());
    assert_eq!(store.read(), Doc::default());
    let aside = dir.join("a.json.corrupt");
    assert!(aside.exists(), "the damaged bytes are preserved");
    let _ = fs::write(&path, b"\"still bad\"");
    store.reset();
    assert_eq!(store.read(), Doc::default());
    let kept = fs::read_to_string(&aside).unwrap();
    assert_eq!(kept, "{\"items\": [1,");
  }

  #[test]
  fn concurrent_updates_do_not_lose_writes() {
    let dir = scratch("concurrent");
    let store = std::sync::Arc::new(LocalStore::<Doc>::at(dir.join("a.json")));
    let mut joins = Vec::new();
    for _ in 0..4 {
      let store = store.clone();
      joins.push(std::thread::spawn(move || {
        for _ in 0..25 {
          store
            .update(|doc| {
              doc.items.push(1);
              Ok::<(), String>(())
            })
            .expect("update");
        }
      }));
    }
    for join in joins {
      join.join().unwrap();
    }
    store.reset();
    assert_eq!(store.read().items.len(), 100);
  }

  #[test]
  fn the_store_lives_under_the_automation_subdirectory() {
    let guard = app_dirs::set_test_data_dir(scratch("location"));
    assert!(dir().ends_with("automation"));
    drop(guard);
  }
}
