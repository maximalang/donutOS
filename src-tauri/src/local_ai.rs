//! The agent's local brain: an OpenAI-compatible endpoint the person owns.
//!
//! The vendor build asked the Donut cloud to plan agent work. The free build
//! has no cloud to ask, so the agent's conversation and recipe generation
//! call an endpoint the person configures — any OpenAI-compatible provider
//! with a `/chat/completions` shape: a local model, a hosted one, a relay.
//! The API key never leaves this machine unencrypted (it lives in its own
//! sealed file like the other secrets) and never appears in a log, a
//! command answer, or a thread item.

use serde::Serialize;
use serde_json::{json, Value};

const AI_TIMEOUT_SECS: u64 = 60;

/// Everything a call needs, resolved once so a request cannot half-change
/// configuration mid-flight.
pub(crate) struct AiConfig {
  base_url: String,
  model: String,
  api_key: String,
}

impl AiConfig {
  /// The chat endpoint: a base that already ends in `/v1` is respected,
  /// anything else gets `/v1` appended.
  fn chat_url(&self) -> String {
    let base = self.base_url.trim().trim_end_matches('/');
    if base.ends_with("/v1") {
      format!("{base}/chat/completions")
    } else {
      format!("{base}/v1/chat/completions")
    }
  }
}

/// The configured provider, or the honest refusal. `AI_NOT_CONFIGURED`
/// carries no detail on purpose: there is nothing to leak.
pub(crate) fn load_config() -> Result<AiConfig, String> {
  let manager = crate::settings_manager::SettingsManager::instance();
  let settings = manager
    .load_settings()
    .map_err(|_| crate::backend_error("AI_NOT_CONFIGURED"))?;
  if !settings.ai_provider_enabled {
    return Err(crate::backend_error("AI_NOT_CONFIGURED"));
  }
  let base_url = settings
    .ai_provider_base_url
    .filter(|url| !url.trim().is_empty())
    .ok_or_else(|| crate::backend_error("AI_NOT_CONFIGURED"))?;
  let model = settings
    .ai_provider_model
    .filter(|model| !model.trim().is_empty())
    .ok_or_else(|| crate::backend_error("AI_NOT_CONFIGURED"))?;
  let api_key = manager
    .get_ai_provider_key()
    .map_err(|_| crate::backend_error("AI_NOT_CONFIGURED"))?
    .filter(|key| !key.trim().is_empty())
    .ok_or_else(|| crate::backend_error("AI_NOT_CONFIGURED"))?;
  Ok(AiConfig {
    base_url,
    model,
    api_key,
  })
}

/// One round trip: messages in, the assistant's content out.
async fn chat(config: &AiConfig, messages: &[Value]) -> Result<String, String> {
  let client = reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(AI_TIMEOUT_SECS))
    .build()
    .map_err(|_| crate::backend_error("AI_REQUEST_FAILED"))?;
  let response = client
    .post(config.chat_url())
    .bearer_auth(&config.api_key)
    .json(&json!({ "model": config.model, "messages": messages }))
    .send()
    .await
    .map_err(|e| {
      log::warn!("AI request failed: {e}");
      crate::backend_error("AI_REQUEST_FAILED")
    })?;
  if !response.status().is_success() {
    // The status is safe to log; the body may echo the key's shape and is not.
    log::warn!("AI provider answered {}", response.status());
    return Err(crate::backend_error("AI_REQUEST_FAILED"));
  }
  let body: Value = response
    .json()
    .await
    .map_err(|_| crate::backend_error("AI_BAD_RESPONSE"))?;
  body
    .pointer("/choices/0/message/content")
    .and_then(Value::as_str)
    .map(str::to_string)
    .filter(|text| !text.trim().is_empty())
    .ok_or_else(|| crate::backend_error("AI_BAD_RESPONSE"))
}

/// The agent conversation's answer, grounded in the tools this app serves.
pub(crate) async fn ask_agent(
  question: &str,
  profile_context: Option<&str>,
) -> Result<String, String> {
  let config = load_config()?;
  let mut catalog = String::new();
  for tool in crate::mcp_server::McpServer::instance().get_tools() {
    catalog.push_str(&format!("- {}: {}\n", tool.name, tool.description));
  }
  let mut system = format!(
    "You are the automation agent of DonutOS, a local antidetect browser. \
You drive profiles through the tools listed below. Answer briefly and \
concretely in the user's language; when the user describes a workflow, name \
the exact tool sequence you would run. Tools:\n{catalog}"
  );
  if let Some(context) = profile_context {
    system.push_str(&format!("\nCurrent profile context: {context}"));
  }
  chat(
    &config,
    &[
      json!({ "role": "system", "content": system }),
      json!({ "role": "user", "content": question }),
    ],
  )
  .await
}

/// What the settings page shows: nothing secret.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiStatus {
  enabled: bool,
  base_url: Option<String>,
  model: Option<String>,
  has_key: bool,
}

fn status() -> Result<AiStatus, String> {
  let manager = crate::settings_manager::SettingsManager::instance();
  let settings = manager
    .load_settings()
    .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e.to_string()))?;
  let has_key = manager
    .get_ai_provider_key()
    .map(|key| key.is_some_and(|key| !key.trim().is_empty()))
    .unwrap_or(false);
  Ok(AiStatus {
    enabled: settings.ai_provider_enabled,
    base_url: settings.ai_provider_base_url,
    model: settings.ai_provider_model,
    has_key,
  })
}

/// The provider's configuration as the settings page sees it: fields without
/// the key, plus whether one is stored.
#[tauri::command]
pub fn get_ai_provider_status() -> Result<Value, String> {
  let status = status()?;
  Ok(serde_json::to_value(status).unwrap_or_default())
}

/// Save the provider configuration. An empty `api_key` keeps the stored one;
/// `None` in the fields keeps their current values, so a settings save that
/// knows only the toggle does not wipe the rest.
#[tauri::command]
pub async fn save_ai_provider_config(
  enabled: bool,
  base_url: Option<String>,
  model: Option<String>,
  api_key: Option<String>,
) -> Result<Value, String> {
  let manager = crate::settings_manager::SettingsManager::instance();
  let mut settings = manager
    .load_settings()
    .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e.to_string()))?;
  settings.ai_provider_enabled = enabled;
  if let Some(base) = base_url {
    let base = base.trim().to_string();
    settings.ai_provider_base_url = (!base.is_empty()).then_some(base);
  }
  if let Some(model) = model {
    let model = model.trim().to_string();
    settings.ai_provider_model = (!model.is_empty()).then_some(model);
  }
  manager
    .save_settings(&settings)
    .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e.to_string()))?;
  if let Some(key) = api_key {
    let key = key.trim();
    if !key.is_empty() {
      manager
        .store_ai_provider_key(key)
        .map_err(|_| crate::backend_error("AI_KEY_STORE_FAILED"))?;
    }
  }
  let status = status()?;
  Ok(serde_json::to_value(status).unwrap_or_default())
}

/// A description in a recipe out: the LLM answers a strict-JSON prompt built
/// from the recipe step schema, the answer is validated by the same rules
/// the manual editor enforces, and only then saved.
#[tauri::command]
pub async fn generate_recipe_from_text(
  description: String,
  name: Option<String>,
) -> Result<Value, String> {
  let description = description.trim().to_string();
  if description.is_empty() {
    return Err(crate::backend_error("AGENT_NOTE_EMPTY"));
  }
  let config = load_config()?;
  let system = "\
You convert a described browser workflow into a DonutOS automation recipe. \
Answer with ONE JSON array and nothing else - no prose, no code fence. Each \
element is a step object with a `type` field and its parameters:
- {\"type\":\"navigate\",\"url\":\"https://...\"}
- {\"type\":\"click\",\"selector\":\"css selector\"} (or \"locator\": {\"text\":\"visible text\"})
- {\"type\":\"type\",\"selector\":\"...\",\"text\":\"what to type\"}
- {\"type\":\"waitFor\",\"seconds\":2}
- {\"type\":\"pressKey\",\"key\":\"Enter\"}
- {\"type\":\"extract\",\"selector\":\"...\"} or with locator
Use only these types. Prefer stable selectors; use text locators for links \
and buttons by their visible label.";
  let answer = chat(
    &config,
    &[
      json!({ "role": "system", "content": system }),
      json!({ "role": "user", "content": description }),
    ],
  )
  .await?;
  let steps_json =
    extract_json_array(&answer).ok_or_else(|| crate::backend_error("AI_RECIPE_INVALID"))?;
  let steps: Vec<Value> =
    serde_json::from_str(&steps_json).map_err(|_| crate::backend_error("AI_RECIPE_INVALID"))?;
  let name = name
    .map(|n| n.trim().to_string())
    .filter(|n| !n.is_empty())
    .unwrap_or_else(|| {
      let mut short: String = description.chars().take(48).collect();
      if description.chars().count() > 48 {
        short.push('…');
      }
      short
    });
  let (_, steps) = crate::recipes::validate_recipe(&name, &steps)?;
  let recipe = crate::recipes::create_recipe(&name, &steps).await?;
  serde_json::to_value(recipe).map_err(|_| crate::backend_error("AI_BAD_RESPONSE"))
}

/// The model is told to answer one JSON array; strip a code fence or any
/// prose around it when it did not.
fn extract_json_array(text: &str) -> Option<String> {
  let trimmed = text.trim();
  let start = trimmed.find('[')?;
  let end = trimmed.rfind(']')?;
  (start < end).then(|| trimmed[start..=end].to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn an_array_survives_fences_and_prose() {
    assert_eq!(
      extract_json_array("```json\n[{\"type\":\"navigate\"}]\n```"),
      Some("[{\"type\":\"navigate\"}]".to_string())
    );
    assert_eq!(
      extract_json_array("Here you go: [1,2] hope it helps"),
      Some("[1,2]".to_string())
    );
    assert_eq!(extract_json_array("no steps needed"), None);
  }
}
