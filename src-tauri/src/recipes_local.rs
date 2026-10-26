//! Local recipe store and replay engine (free build).
//!
//! Upstream keeps recipes in the vendor cloud: storage, validation and replay
//! all live server-side, and this client was wire-only. The free build has no
//! cloud to ask, so recipes are stored in one local JSON document and replayed
//! ON THIS MACHINE through the existing MCP tool surface
//! (`McpServer::call_tool`): every step runs with exactly the semantics of the
//! `navigate` / `click_*` / `type_*` / `wait_for` / `press_key` / `scroll` /
//! `screenshot` tools a remote agent would call — same engine paths, same rate
//! limiter, same human typing. There is deliberately no second browser driver
//! here to drift.
//!
//! Storage: `<data>/automation/recipes.json` (see `local_store.rs`). The wire
//! types (`AgentRecipe`) and the validation (`validate_recipe`) are reused
//! from `recipes.rs`, so a recipe the cloud would accept is exactly a recipe
//! the local store accepts.

use crate::local_store::LocalStore;
use crate::recipes::{validate_recipe, AgentRecipe, RecipeStep};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

fn store() -> &'static LocalStore<Vec<AgentRecipe>> {
  static STORE: OnceLock<LocalStore<Vec<AgentRecipe>>> = OnceLock::new();
  STORE.get_or_init(|| LocalStore::new("recipes"))
}

fn now_iso() -> String {
  chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub(crate) async fn list_recipes() -> Result<Vec<AgentRecipe>, String> {
  Ok(store().read())
}

pub(crate) async fn create_recipe(name: &str, steps: &[RecipeStep]) -> Result<AgentRecipe, String> {
  let (name, steps) = validate_recipe(name, steps)?;
  let recipe = AgentRecipe {
    id: uuid::Uuid::new_v4().to_string(),
    name,
    steps,
    created_at: Some(now_iso()),
    updated_at: Some(now_iso()),
  };
  store().update(|list| {
    list.push(recipe.clone());
    Ok::<(), String>(())
  })?;
  Ok(recipe)
}

pub(crate) async fn update_recipe(
  id: &str,
  name: &str,
  steps: &[RecipeStep],
) -> Result<AgentRecipe, String> {
  let (name, steps) = validate_recipe(name, steps)?;
  let updated = store().update(|list| {
    let found = list
      .iter_mut()
      .find(|recipe| recipe.id == id)
      .ok_or_else(|| "AGENT_RECIPE_NOT_FOUND".to_string())?;
    found.name = name.clone();
    found.steps = steps.clone();
    found.updated_at = Some(now_iso());
    Ok::<AgentRecipe, String>(found.clone())
  })?;
  Ok(updated)
}

pub(crate) async fn delete_recipe(id: &str) -> Result<bool, String> {
  store().update(|list| {
    let before = list.len();
    list.retain(|recipe| recipe.id != id);
    Ok::<bool, String>(before != list.len())
  })
}

/// Runs recipes one at a time on a dedicated task. A replay drives MCP
/// tools, and the run_recipe MCP tool reaches back into this module: a
/// direct await chain would form a recursive future type that can never
/// satisfy the Send bound. The channel breaks the cycle structurally.
static RECIPE_RUNNER: OnceLock<tokio::sync::mpsc::UnboundedSender<(String, String)>> =
  OnceLock::new();

fn ensure_recipe_runner() -> &'static tokio::sync::mpsc::UnboundedSender<(String, String)> {
  RECIPE_RUNNER.get_or_init(|| {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
    tokio::spawn(async move {
      while let Some((recipe_id, profile_id)) = rx.recv().await {
        let Some(recipe) = store().read().into_iter().find(|r| r.id == recipe_id) else {
          log::warn!("Recipe run skipped: recipe vanished id={recipe_id}");
          continue;
        };
        let steps = recipe.steps.clone();
        let log_name = recipe.name.clone();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let replay: std::pin::Pin<
          Box<dyn std::future::Future<Output = ReplaySummary> + Send>,
        > = Box::pin(replay_steps(&profile_id, &steps, &cancel));
        let summary = replay.await;
        log::info!(
          "Recipe replay finished recipe=\"{log_name}\" profile={profile_id} ok={} failed={} error={:?}",
          summary.steps_ok,
          summary.steps_failed,
          summary.first_error
        );
      }
    });
    tx
  })
}

/// Replay a recipe on a profile now. The replay itself runs on the recipe
/// task (a long recipe must not block the caller); the answer says what
/// started.
pub(crate) async fn run_recipe_now(
  recipe_id: &str,
  profile_id: &str,
) -> Result<serde_json::Value, String> {
  let recipe = store()
    .read()
    .into_iter()
    .find(|recipe| recipe.id == recipe_id)
    .ok_or_else(|| crate::backend_error("AGENT_RECIPE_NOT_FOUND"))?;
  let name = recipe.name.clone();
  let _ = ensure_recipe_runner().send((recipe_id.to_string(), profile_id.to_string()));
  Ok(serde_json::json!({
    "started": true,
    "recipeId": recipe_id,
    "profileId": profile_id,
    "name": name,
  }))
}

// --- Replay -----------------------------------------------------------------

/// One planned step: a tool call, or a local pause.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PlannedStep {
  /// Call this MCP tool with these arguments.
  Tool { name: String, arguments: Value },
  /// Sleep locally; no tool is involved.
  Sleep(Duration),
}

/// What one replay did.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub(crate) struct ReplaySummary {
  pub steps_total: u32,
  pub steps_ok: u32,
  pub steps_failed: u32,
  /// The first failure's message, for the run record.
  pub first_error: Option<String>,
}

/// Plan a recipe step as an MCP tool call. Pure — no browser, no clock — so
/// the mapping is unit-testable. `None` refuses a step that cannot run (an
/// unknown shape): the caller counts it failed rather than guessing.
pub(crate) fn plan_step(profile_id: &str, step: &Value) -> Option<PlannedStep> {
  let object = step.as_object()?;
  let kind = object.get("type")?.as_str()?;
  let tool = |name: &str, extra: Vec<(String, Value)>| {
    let mut arguments = serde_json::Map::new();
    arguments.insert("profile_id".to_string(), json!(profile_id));
    for (key, value) in extra {
      arguments.insert(key, value);
    }
    PlannedStep::Tool {
      name: name.to_string(),
      arguments: Value::Object(arguments),
    }
  };
  let selector = object.get("selector").and_then(Value::as_str);
  let locator = object
    .get("locator")
    .filter(|l| l.as_object().is_some_and(|o| !o.is_empty()));
  match kind {
    "navigate" => {
      let url = object
        .get("url")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
      if url.is_empty() {
        return None;
      }
      Some(tool("navigate", vec![("url".to_string(), json!(url))]))
    }
    "click" => match (selector, locator) {
      (Some(selector), None) => Some(tool(
        "click_element",
        vec![("selector".to_string(), json!(selector))],
      )),
      (None, Some(locator)) => Some(tool(
        "click_locator",
        vec![("locator".to_string(), locator.clone())],
      )),
      _ => None,
    },
    "type" => {
      let text = object.get("text").and_then(Value::as_str)?;
      match (selector, locator) {
        (Some(selector), None) => Some(tool(
          "type_text",
          vec![
            ("selector".to_string(), json!(selector)),
            ("text".to_string(), json!(text)),
          ],
        )),
        (None, Some(locator)) => Some(tool(
          "type_locator",
          vec![
            ("locator".to_string(), locator.clone()),
            ("text".to_string(), json!(text)),
          ],
        )),
        _ => None,
      }
    }
    "waitFor" => {
      let selector = selector?;
      let timeout = object
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .unwrap_or(10_000);
      Some(tool(
        "wait_for",
        vec![
          ("selector".to_string(), json!(selector)),
          ("timeout_ms".to_string(), json!(timeout)),
        ],
      ))
    }
    "pressKey" => {
      let key = object
        .get("key")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
      if key.is_empty() {
        return None;
      }
      Some(tool("press_key", vec![("key".to_string(), json!(key))]))
    }
    "scroll" => {
      let direction = object
        .get("direction")
        .and_then(Value::as_str)
        .unwrap_or("down");
      if direction != "up" && direction != "down" {
        return None;
      }
      Some(tool(
        "scroll",
        vec![
          ("direction".to_string(), json!(direction)),
          ("pages".to_string(), json!(1.0)),
        ],
      ))
    }
    "screenshot" => {
      let mut extra = Vec::new();
      if object.get("fullPage").and_then(Value::as_bool) == Some(true) {
        extra.push(("full_page".to_string(), json!(true)));
      }
      Some(tool("screenshot", extra))
    }
    "sleep" => {
      let ms = object.get("ms").and_then(Value::as_u64).unwrap_or(1_000);
      Some(PlannedStep::Sleep(Duration::from_millis(ms.min(60_000))))
    }
    // `extract` reads ONE named value off the page. A `selector` target can
    // be read by the page's own JavaScript on every engine; a `locator`
    // target goes through `extract_structured` (one row, one field), which
    // is Wayfern-only upstream and fails honestly elsewhere.
    "extract" => {
      let key = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("value")
        .to_string();
      let attribute = object.get("attr").and_then(Value::as_str);
      match (selector, locator) {
        (Some(selector), None) => {
          let read = match attribute {
            Some(attr) => format!(
              "el.getAttribute({attr})",
              attr = serde_json::to_string(attr).unwrap_or_else(|_| "\"\"".to_string())
            ),
            None => "el.textContent".to_string(),
          };
          let expression = format!(
            "(() => {{ const el = document.querySelector({selector}); \
             return el ? ({read}) : null; }})()",
            selector = serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".to_string()),
            read = read
          );
          Some(tool(
            "evaluate_javascript",
            vec![("expression".to_string(), json!(expression))],
          ))
        }
        (None, Some(locator)) => {
          let mut field = serde_json::Map::new();
          field.insert("key".to_string(), json!(key));
          field.insert("locator".to_string(), locator.clone());
          match attribute {
            Some(attr) => {
              field.insert("source".to_string(), json!("attribute"));
              field.insert("attribute".to_string(), json!(attr));
            }
            None => {
              field.insert("source".to_string(), json!("text"));
            }
          }
          let extra = vec![
            ("container".to_string(), locator.clone()),
            ("field_map".to_string(), json!([Value::Object(field)])),
          ];
          Some(tool("extract_structured", extra))
        }
        _ => None,
      }
    }
    _ => None,
  }
}

/// Replay `steps` on the profile, one step at a time, stopping early when
/// `cancel` flips. A failed step does not abort the run: the recipe's later
/// steps may not depend on it, and a partial run is more honest than no run.
/// Returns per-step counts for the run record.
pub(crate) async fn replay_steps(
  profile_id: &str,
  steps: &[RecipeStep],
  cancel: &AtomicBool,
) -> ReplaySummary {
  let mut summary = ReplaySummary {
    steps_total: steps.len() as u32,
    ..Default::default()
  };
  for step in steps {
    if cancel.load(Ordering::Relaxed) {
      break;
    }
    match plan_step(profile_id, step) {
      Some(PlannedStep::Sleep(duration)) => {
        tokio::time::sleep(duration).await;
        summary.steps_ok += 1;
      }
      Some(PlannedStep::Tool { name, arguments }) => {
        match crate::mcp_server::McpServer::instance()
          .call_tool(&name, &arguments)
          .await
        {
          Ok(_) => summary.steps_ok += 1,
          Err(message) => {
            summary.steps_failed += 1;
            if summary.first_error.is_none() {
              summary.first_error = Some(message);
            }
          }
        }
      }
      None => {
        summary.steps_failed += 1;
        if summary.first_error.is_none() {
          summary.first_error.replace(format!(
            "step {} could not be planned",
            summary.steps_ok + summary.steps_failed
          ));
        }
      }
    }
  }
  summary
}

#[cfg(test)]
mod tests {
  use super::*;

  fn planned(step: Value) -> Option<PlannedStep> {
    plan_step("p1", &step)
  }

  #[test]
  fn a_navigable_step_plans_the_navigate_tool() {
    let plan = planned(json!({"type": "navigate", "url": "https://example.com"})).expect("planned");
    match plan {
      PlannedStep::Tool { name, arguments } => {
        assert_eq!(name, "navigate");
        assert_eq!(arguments["profile_id"], "p1");
        assert_eq!(arguments["url"], "https://example.com");
      }
      _ => panic!("expected a tool call"),
    }
  }

  #[test]
  fn click_and_type_go_to_the_selector_or_locator_tool() {
    match planned(json!({"type": "click", "selector": "#buy"})).unwrap() {
      PlannedStep::Tool { name, arguments } => {
        assert_eq!(name, "click_element");
        assert_eq!(arguments["selector"], "#buy");
      }
      _ => panic!("tool"),
    }
    match planned(
      json!({"type": "type", "locator": {"role": "textbox", "name": "Search"}, "text": "donut"}),
    )
    .unwrap()
    {
      PlannedStep::Tool { name, arguments } => {
        assert_eq!(name, "type_locator");
        assert_eq!(arguments["text"], "donut");
        assert_eq!(arguments["locator"]["role"], "textbox");
      }
      _ => panic!("tool"),
    }
  }

  #[test]
  fn a_step_targeting_both_ways_or_neither_is_refused() {
    assert!(
      planned(json!({"type": "click", "selector": "#a", "locator": {"role": "button"}})).is_none()
    );
    assert!(planned(json!({"type": "click"})).is_none());
    assert!(planned(json!({"type": "type", "selector": "#a"})).is_none());
  }

  #[test]
  fn sleep_sleeps_locally_and_scroll_keeps_its_direction() {
    match planned(json!({"type": "sleep", "ms": 250})).unwrap() {
      PlannedStep::Sleep(duration) => assert_eq!(duration, Duration::from_millis(250)),
      _ => panic!("sleep"),
    }
    match planned(json!({"type": "scroll", "direction": "up"})).unwrap() {
      PlannedStep::Tool { name, arguments } => {
        assert_eq!(name, "scroll");
        assert_eq!(arguments["direction"], "up");
      }
      _ => panic!("tool"),
    }
    assert!(planned(json!({"type": "scroll", "direction": "sideways"})).is_none());
  }

  #[test]
  fn a_cap_keeps_one_step_from_sleeping_forever() {
    match planned(json!({"type": "sleep", "ms": 3_600_000})).unwrap() {
      PlannedStep::Sleep(duration) => assert_eq!(duration, Duration::from_secs(60)),
      _ => panic!("sleep"),
    }
  }

  #[test]
  fn the_store_round_trips_a_recipe() {
    let dir = std::env::temp_dir().join("donut-recipes-local-test");
    let _ = std::fs::remove_dir_all(&dir);
    let local: LocalStore<Vec<AgentRecipe>> = LocalStore::at(dir.join("recipes.json"));
    let step = json!({"type": "navigate", "url": "https://example.com"});
    local
      .update(|list| {
        list.push(AgentRecipe {
          id: "r1".to_string(),
          name: "nightly".to_string(),
          steps: vec![step.clone()],
          created_at: None,
          updated_at: None,
        });
        Ok::<(), String>(())
      })
      .expect("stored");
    local.reset();
    assert_eq!(local.read().len(), 1);
    assert_eq!(local.read()[0].steps[0], step);
  }
}
