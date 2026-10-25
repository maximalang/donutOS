//! Local scheduled-automation engine (free build): recipe schedules, fire
//! logic, run history and webhook notices — everything the vendor cloud
//! served for `/api/agent/schedules`, on this machine.
//!
//! The public functions mirror the Tauri commands in `agent_schedules.rs`
//! one-to-one and speak the same camelCase JSON the app sends, so the
//! schedule UI, the REST mirror and every caller keep working unchanged.
//! Storage is one JSON document under `<data>/automation/` (see
//! `local_store.rs`).
//!
//! A firing replays the schedule's recipe on every enrolled profile through
//! the existing MCP tool surface (`recipes_local::replay_steps`) — no second
//! browser driver. Notices: email is off (no local SMTP), webhook parity is
//! kept — the same JSON body the cloud posted, signed with the schedule's
//! secret (`X-Donut-Signature: sha256=<HMAC-SHA256>`, ring).
//!
//! Not ported, on purpose: `target: "fleet"` (rented vendor hardware) is
//! refused at save; cloud replay storage is replaced by local run records.

use crate::automation_time::{
  due_today, next_weekly_fire, parse_instant, valid_timezone, weekly_fire_key, WeeklySlot,
};
use crate::local_store::LocalStore;
use crate::recipes_local;
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;
use std::time::Duration as StdDuration;

const MIN_INTERVAL_MINUTES: i64 = 15;
const MAX_INTERVAL_MINUTES: i64 = 10_080;
const MAX_NAME_CHARS: usize = 80;
const MAX_PROFILES: usize = 100;
/// How long after a once-schedule's instant it still fires (machine asleep);
/// beyond that it is marked missed.
const ONCE_WINDOW_MINUTES: i64 = 360;
/// How long after a weekly slot's local time it still fires.
const WEEKLY_WINDOW_MINUTES: i64 = 45;
const TICK_SECONDS: u64 = 30;

// --- Wire types ------------------------------------------------------------------

/// A schedule as the app sends and reads it (camelCase on the wire).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSchedule {
  pub id: String,
  pub name: String,
  #[serde(default = "default_true")]
  pub enabled: bool,
  pub target: String,
  #[serde(default)]
  pub recipe_id: Option<String>,
  #[serde(default)]
  pub profile_ids: Vec<String>,
  #[serde(default)]
  pub group_id: Option<String>,
  #[serde(default)]
  pub cadence: String,
  #[serde(default)]
  pub days_mask: u8,
  #[serde(default)]
  pub run_at_minute: u32,
  #[serde(default = "default_timezone")]
  pub timezone: String,
  #[serde(default)]
  pub run_once_at: Option<String>,
  #[serde(default)]
  pub interval_minutes: Option<i64>,
  #[serde(default)]
  pub next_run_at: Option<String>,
  #[serde(default)]
  pub last_run_at: Option<String>,
  #[serde(default)]
  pub notify_email: bool,
  #[serde(default)]
  pub webhook_url: Option<String>,
  #[serde(default)]
  pub webhook_secret: Option<String>,
  /// Local bookkeeping: the interval anchor. Never sent by the app; the wire
  /// tolerates the extra key, and the store needs it to fire intervals.
  #[serde(default)]
  pub created_at: Option<String>,
}

impl Default for AgentSchedule {
  fn default() -> Self {
    Self {
      id: String::new(),
      name: String::new(),
      enabled: true,
      target: "desktop".to_string(),
      recipe_id: None,
      profile_ids: Vec::new(),
      group_id: None,
      cadence: String::new(),
      days_mask: 0,
      run_at_minute: 0,
      timezone: "UTC".to_string(),
      run_once_at: None,
      interval_minutes: None,
      next_run_at: None,
      last_run_at: None,
      notify_email: false,
      webhook_url: None,
      webhook_secret: None,
      created_at: None,
    }
  }
}

fn default_true() -> bool {
  true
}

fn default_timezone() -> String {
  "UTC".to_string()
}

/// One run of one schedule, as the run list shows it.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentScheduleRun {
  pub id: String,
  #[serde(default)]
  pub schedule_id: String,
  pub status: String,
  pub scheduled_for: String,
  pub label: String,
  /// Local detail beyond the wire shape: what the replay actually did.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub detail: Option<String>,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct Doc {
  #[serde(default)]
  schedules: Vec<AgentSchedule>,
  #[serde(default)]
  runs: Vec<AgentScheduleRun>,
  /// Firing-slot identities already fired, bounded.
  #[serde(default)]
  fired: Vec<String>,
}

fn store() -> &'static LocalStore<Doc> {
  static STORE: OnceLock<LocalStore<Doc>> = OnceLock::new();
  STORE.get_or_init(|| LocalStore::new("agent-schedules"))
}

fn now_iso() -> String {
  Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn invalid() -> String {
  crate::backend_error("AGENT_SCHEDULE_INVALID")
}

fn not_found() -> String {
  crate::backend_error("AGENT_SCHEDULE_NOT_FOUND")
}

fn parse_rfc3339(text: &str) -> Option<DateTime<Utc>> {
  parse_instant(text)
}

// --- Validation -------------------------------------------------------------------

/// Validate and normalise a schedule body. `existing` carries the fields the
/// patch does not mention. Pure — no store, no clock.
fn validate(body: &Value, existing: Option<&AgentSchedule>) -> Result<AgentSchedule, String> {
  let bad = || invalid();
  let get_str = |key: &str| body.get(key).and_then(Value::as_str);
  let get_bool = |key: &str| body.get(key).and_then(Value::as_bool);
  let get_u64 = |key: &str| body.get(key).and_then(Value::as_u64);

  let base = existing.cloned().unwrap_or_default();
  let name = match get_str("name") {
    Some(name) => name.trim().to_string(),
    None => base.name.clone(),
  };
  if name.is_empty() || name.len() > MAX_NAME_CHARS {
    return Err(bad());
  }

  let enabled = get_bool("enabled").unwrap_or(base.enabled);
  let target = get_str("target").unwrap_or(&base.target).to_string();
  if target != "desktop" {
    // The remote fleet is rented vendor hardware; only this machine exists.
    return Err(bad());
  }

  let recipe_id = match body.get("recipeId") {
    Some(Value::String(id)) => Some(id.clone()),
    Some(Value::Null) => None,
    None => base.recipe_id.clone(),
    Some(_) => return Err(bad()),
  };
  let recipe_id = recipe_id.ok_or_else(bad)?;

  let profile_ids = match body.get("profileIds").and_then(Value::as_array) {
    Some(list) => {
      let mut ids: Vec<String> = Vec::new();
      for id in list {
        let id = id.as_str().ok_or_else(bad)?.trim().to_string();
        if id.is_empty() {
          return Err(bad());
        }
        if !ids.contains(&id) {
          ids.push(id);
        }
      }
      ids
    }
    None => base.profile_ids.clone(),
  };
  if profile_ids.is_empty() || profile_ids.len() > MAX_PROFILES {
    return Err(bad());
  }

  let cadence = get_str("cadence").unwrap_or(&base.cadence).to_string();
  let mut schedule = AgentSchedule {
    id: base.id.clone(),
    name,
    enabled,
    target,
    recipe_id: Some(recipe_id),
    profile_ids,
    group_id: None,
    cadence: cadence.clone(),
    days_mask: base.days_mask,
    run_at_minute: base.run_at_minute,
    timezone: base.timezone.clone(),
    run_once_at: base.run_once_at.clone(),
    interval_minutes: base.interval_minutes,
    next_run_at: None,
    last_run_at: base.last_run_at.clone(),
    notify_email: get_bool("notifyEmail").unwrap_or(base.notify_email),
    webhook_url: base.webhook_url.clone(),
    webhook_secret: base.webhook_secret.clone(),
    created_at: base.created_at.clone(),
  };

  match cadence.as_str() {
    "weekly" => {
      let days_mask = get_u64("daysMask").unwrap_or(schedule.days_mask as u64) as u8;
      let run_at_minute = get_u64("runAtMinute").unwrap_or(schedule.run_at_minute as u64) as u32;
      let timezone = get_str("timezone")
        .unwrap_or(&schedule.timezone)
        .to_string();
      if days_mask == 0 || days_mask > 127 {
        return Err(bad());
      }
      if run_at_minute > 1439 {
        return Err(bad());
      }
      if !valid_timezone(&timezone) {
        return Err(bad());
      }
      schedule.days_mask = days_mask;
      schedule.run_at_minute = run_at_minute;
      schedule.timezone = timezone;
    }
    "once" => {
      let instant = match get_str("runOnceAt") {
        Some(text) => text,
        None => match &schedule.run_once_at {
          Some(text) => text.as_str(),
          None => return Err(bad()),
        },
      };
      let parsed = parse_rfc3339(instant).ok_or_else(bad)?;
      if parsed <= Utc::now() && existing.is_none() {
        return Err(bad());
      }
      schedule.run_once_at = Some(instant.to_string());
    }
    "interval" => {
      let minutes = get_u64("intervalMinutes")
        .map(|m| m as i64)
        .or(schedule.interval_minutes)
        .ok_or_else(bad)?;
      if !(MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES).contains(&minutes) {
        return Err(bad());
      }
      schedule.interval_minutes = Some(minutes);
    }
    _ => return Err(bad()),
  }

  // Webhook: absent keeps, null or empty clears, otherwise https, no creds.
  match body.get("webhookUrl") {
    Some(Value::Null) => schedule.webhook_url = None,
    Some(Value::String(url)) => {
      let trimmed = url.trim();
      if trimmed.is_empty() {
        schedule.webhook_url = None;
      } else {
        if trimmed.len() > 2048 {
          return Err(bad());
        }
        let parsed = url::Url::parse(trimmed).map_err(|_| bad())?;
        if parsed.scheme() != "https"
          || !parsed.username().is_empty()
          || parsed.password().is_some()
        {
          return Err(bad());
        }
        schedule.webhook_url = Some(trimmed.to_string());
      }
    }
    _ => {}
  }

  // Secret rotation is a patch flag, not a field.
  if body.get("rotateWebhookSecret").and_then(Value::as_bool) == Some(true)
    || schedule.webhook_secret.is_none()
  {
    schedule.webhook_secret = Some(uuid::Uuid::new_v4().simple().to_string());
  }

  Ok(schedule)
}

// --- Public surface (mirrors the Tauri commands) -----------------------------------

pub(crate) async fn list() -> Result<Value, String> {
  let mut doc = store().read();
  let now = Utc::now();
  for schedule in &mut doc.schedules {
    schedule.next_run_at =
      next_fire(schedule, now).map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
  }
  serde_json::to_value(&doc.schedules).map_err(|e| e.to_string())
}

pub(crate) async fn create(body: Value) -> Result<Value, String> {
  if !body.is_object() {
    return Err(invalid());
  }
  // The recipe must exist locally — there is no cloud to hold it.
  let recipe_id = body
    .get("recipeId")
    .and_then(Value::as_str)
    .ok_or_else(invalid)?
    .to_string();
  let recipes = recipes_local::list_recipes().await?;
  if !recipes.iter().any(|r| r.id == recipe_id) {
    return Err(invalid());
  }
  let built = validate(&body, None)?;
  let created = store().update(|doc| {
    let mut schedule = built.clone();
    schedule.id = uuid::Uuid::new_v4().to_string();
    schedule.created_at = Some(now_iso());
    doc.schedules.push(schedule.clone());
    Ok::<AgentSchedule, String>(schedule)
  })?;
  log::info!("Schedule created schedule={}", created.id);
  serde_json::to_value(&created).map_err(|e| e.to_string())
}

pub(crate) async fn update(id: &str, patch: Value) -> Result<Value, String> {
  if !patch.is_object() {
    return Err(invalid());
  }
  let updated = store().update(|doc| {
    let existing = match doc.schedules.iter().find(|s| s.id == id) {
      Some(schedule) => schedule.clone(),
      None => return Err::<AgentSchedule, String>(not_found()),
    };
    let mut built = validate(&patch, Some(&existing))?;
    built.id = existing.id.clone();
    built.created_at = existing.created_at.clone();
    if let Some(schedule) = doc.schedules.iter_mut().find(|s| s.id == id) {
      *schedule = built.clone();
    }
    Ok::<AgentSchedule, String>(built)
  })?;
  serde_json::to_value(&updated).map_err(|e| e.to_string())
}

pub(crate) async fn delete(id: &str) -> Result<(), String> {
  store().update(|doc| {
    doc.schedules.retain(|s| s.id != id);
    Ok::<(), String>(())
  })?;
  log::info!("Schedule deleted schedule={id}");
  Ok(())
}

pub(crate) async fn run_now(id: &str) -> Result<Value, String> {
  let schedule = store()
    .read()
    .schedules
    .into_iter()
    .find(|s| s.id == id)
    .ok_or_else(not_found)?;
  let run = fire_schedule(schedule, "manual").await;
  serde_json::to_value(&run).map_err(|e| e.to_string())
}

pub(crate) async fn runs(id: &str) -> Result<Value, String> {
  let doc = store().read();
  let mut runs: Vec<AgentScheduleRun> = doc
    .runs
    .iter()
    .filter(|r| r.schedule_id == id)
    .cloned()
    .collect();
  runs.sort_by(|a, b| b.scheduled_for.cmp(&a.scheduled_for).then(b.id.cmp(&a.id)));
  serde_json::to_value(&runs).map_err(|e| e.to_string())
}

pub(crate) async fn test_notice(id: &str) -> Result<Value, String> {
  let schedule = store()
    .read()
    .schedules
    .into_iter()
    .find(|s| s.id == id)
    .ok_or_else(not_found)?;
  let body = json!({
    "type": "test",
    "scheduleId": schedule.id,
    "scheduleName": schedule.name,
    "at": now_iso(),
  });
  let (webhook, status) = match &schedule.webhook_url {
    None => ("off", None),
    Some(url) => match post_webhook(url, schedule.webhook_secret.as_deref(), &body).await {
      Ok(code) => ("sent", Some(code)),
      Err(e) => {
        log::warn!("Schedule test webhook failed schedule={id}: {e}");
        ("failed", None)
      }
    },
  };
  // Email needs an SMTP story the free build does not have; it is off.
  Ok(json!({ "email": "off", "webhook": webhook, "webhookStatus": status }))
}

// --- Firing ------------------------------------------------------------------------

/// When this schedule fires next, if it does.
fn next_fire(schedule: &AgentSchedule, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
  if !schedule.enabled {
    return None;
  }
  match schedule.cadence.as_str() {
    "weekly" => next_weekly_fire(
      WeeklySlot {
        days_mask: schedule.days_mask,
        run_at_minute: schedule.run_at_minute,
      },
      &schedule.timezone,
      now,
    ),
    "once" => schedule
      .run_once_at
      .as_deref()
      .and_then(parse_rfc3339)
      .filter(|dt| *dt > now),
    "interval" => {
      let minutes = schedule.interval_minutes?;
      let anchor = schedule.created_at.as_deref().and_then(parse_rfc3339)?;
      let step = Duration::minutes(minutes);
      let mut next = anchor + step;
      while next <= now {
        next += step;
      }
      Some(next)
    }
    _ => None,
  }
}

/// The firing slot identities that are due at `now` and not yet fired.
/// Pure over the doc, so restart behaviour is testable.
fn due_fires(doc: &Doc, now: DateTime<Utc>) -> Vec<(String, String)> {
  let mut due = Vec::new();
  for schedule in &doc.schedules {
    if !schedule.enabled {
      continue;
    }
    match schedule.cadence.as_str() {
      "weekly" => {
        let slot = WeeklySlot {
          days_mask: schedule.days_mask,
          run_at_minute: schedule.run_at_minute,
        };
        if let Some(fire) = due_today(slot, &schedule.timezone, now, WEEKLY_WINDOW_MINUTES) {
          let key = weekly_fire_key(
            &format!("as:{}", schedule.id),
            slot,
            &schedule.timezone,
            fire,
          );
          if !doc.fired.contains(&key) {
            due.push((schedule.id.clone(), key));
          }
        }
      }
      "once" => {
        if let Some(instant) = schedule.run_once_at.as_deref().and_then(parse_rfc3339) {
          let key = format!("as:{}:once", schedule.id);
          // Slots older than the window are not due: the caller records them
          // as missed so they never fire on a much-later start.
          if now >= instant
            && !doc.fired.contains(&key)
            && (now - instant) <= Duration::minutes(ONCE_WINDOW_MINUTES)
          {
            due.push((schedule.id.clone(), key));
          }
        }
      }
      "interval" => {
        if let (Some(minutes), Some(anchor)) = (
          schedule.interval_minutes,
          schedule.created_at.as_deref().and_then(parse_rfc3339),
        ) {
          if minutes >= 1 {
            let index = ((now - anchor).num_minutes() / minutes).max(1);
            let key = format!("as:{}:i:{}", schedule.id, index);
            if now >= anchor && !doc.fired.contains(&key) {
              due.push((schedule.id.clone(), key));
            }
          }
        }
      }
      _ => {}
    }
  }
  due
}

/// Mark keys fired (including missed once-slots), keep the log bounded.
fn mark_fired(doc: &mut Doc, keys: Vec<String>) {
  for key in keys {
    doc.fired.retain(|k| *k != key);
    doc.fired.push(key);
  }
  let cut = doc.fired.len().saturating_sub(500);
  if cut > 0 {
    doc.fired.drain(0..cut);
  }
}

/// Replay the schedule's recipe on every enrolled profile and record the run.
async fn fire_schedule(schedule: AgentSchedule, trigger: &str) -> AgentScheduleRun {
  let recipe_name = recipes_local::list_recipes()
    .await
    .ok()
    .and_then(|recipes| {
      recipes
        .iter()
        .find(|r| Some(&r.id) == schedule.recipe_id.as_ref())
        .map(|r| r.name.clone())
    })
    .unwrap_or_else(|| schedule.recipe_id.clone().unwrap_or_default());
  let steps: Vec<Value> = recipes_local::list_recipes()
    .await
    .ok()
    .and_then(|recipes| {
      recipes
        .iter()
        .find(|r| Some(&r.id) == schedule.recipe_id.as_ref())
        .map(|r| r.steps.clone())
    })
    .unwrap_or_default();

  let cancel = AtomicBool::new(false);
  let mut statuses: Vec<(&str, u32, u32)> = Vec::new();
  let mut first_error: Option<String> = None;
  for profile_id in &schedule.profile_ids {
    let summary = recipes_local::replay_steps(profile_id, &steps, &cancel).await;
    if let Some(error) = &summary.first_error {
      if first_error.is_none() {
        first_error = Some(error.clone());
      }
    }
    statuses.push((profile_id.as_str(), summary.steps_ok, summary.steps_failed));
  }

  let total_failed: u32 = statuses.iter().map(|(_, _, failed)| *failed).sum();
  let total_ok: u32 = statuses.iter().map(|(_, ok, _)| *ok).sum();
  let status = if total_ok == 0 && total_failed > 0 {
    "failed"
  } else if total_failed > 0 {
    "partial"
  } else {
    "succeeded"
  };
  let detail = first_error.clone();

  let run = AgentScheduleRun {
    id: uuid::Uuid::new_v4().to_string(),
    schedule_id: schedule.id.clone(),
    status: status.to_string(),
    scheduled_for: now_iso(),
    label: recipe_name.clone(),
    detail: detail.clone(),
  };
  let schedule_id = schedule.id.clone();
  let run_scheduled_for = run.scheduled_for.clone();
  if let Err(e) = store().update(|doc| {
    doc.runs.push(run.clone());
    if doc.runs.len() > 500 {
      let cut = doc.runs.len() - 500;
      doc.runs.drain(0..cut);
    }
    if let Some(schedule) = doc.schedules.iter_mut().find(|s| s.id == schedule_id) {
      schedule.last_run_at = Some(run_scheduled_for.clone());
    }
    Ok::<(), String>(())
  }) {
    log::error!(
      "Schedule run record not persisted schedule={}: {e}",
      schedule.id
    );
  }

  // Webhook notice: same body shape the cloud posted, signed.
  if let Some(url) = &schedule.webhook_url {
    let body = json!({
      "type": if status == "succeeded" { "run_succeeded" } else { "run_failed" },
      "scheduleId": schedule.id,
      "scheduleName": schedule.name,
      "runId": run.id,
      "status": status,
      "at": run.scheduled_for,
      "detail": detail,
    });
    if let Err(e) = post_webhook(url, schedule.webhook_secret.as_deref(), &body).await {
      log::warn!("Schedule webhook failed schedule={}: {e}", schedule.id);
    }
  }
  if status != "succeeded" {
    // No SMTP story: failures log, they do not mail.
    log::warn!(
      "Schedule run finished schedule={} trigger={trigger} status={status} failed_steps={total_failed}",
      schedule.id
    );
  } else {
    log::info!(
      "Schedule run finished schedule={} trigger={trigger} status=succeeded ok_steps={total_ok}",
      schedule.id
    );
  }
  run
}

// --- Webhook ------------------------------------------------------------------------

fn sign_webhook(secret: &str, body: &str) -> String {
  let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
  let tag = ring::hmac::sign(&key, body.as_bytes());
  let mut hex = String::with_capacity(tag.as_ref().len() * 2);
  for byte in tag.as_ref() {
    hex.push_str(&format!("{byte:02x}"));
  }
  format!("sha256={hex}")
}

async fn post_webhook(url: &str, secret: Option<&str>, body: &Value) -> Result<u16, String> {
  let text = serde_json::to_string(body).map_err(|e| e.to_string())?;
  let mut request = reqwest::Client::builder()
    .timeout(StdDuration::from_secs(10))
    .build()
    .map_err(|e| e.to_string())?
    .post(url)
    .header("Content-Type", "application/json");
  if let Some(secret) = secret {
    request = request.header("X-Donut-Signature", sign_webhook(secret, &text));
  }
  let response = request.body(text).send().await.map_err(|e| e.to_string())?;
  let status = response.status().as_u16();
  if !(200..300).contains(&status) {
    return Err(format!("webhook answered {status}"));
  }
  Ok(status)
}

// --- The scheduler -------------------------------------------------------------------

/// Entry point spawned at app setup (free builds only). One tick every 30 s.
pub(crate) async fn run_scheduler() {
  log::info!("Agent-schedule local scheduler started (tick {TICK_SECONDS}s)");
  loop {
    let now = Utc::now();
    // Record the fire keys BEFORE firing, so a crash mid-run cannot re-fire.
    let due: Vec<(String, String)> = {
      let doc = store().read();
      due_fires(&doc, now)
    };
    if !due.is_empty() {
      // Missed once-slots (older than the window) are also marked, so they
      // never fire on a much-later start.
      let doc = store().read();
      let mut to_mark = due.iter().map(|(_, key)| key.clone()).collect::<Vec<_>>();
      for schedule in &doc.schedules {
        if schedule.cadence == "once" {
          if let Some(instant) = schedule.run_once_at.as_deref().and_then(parse_rfc3339) {
            if (now - instant) > Duration::minutes(ONCE_WINDOW_MINUTES) {
              to_mark.push(format!("as:{}:once", schedule.id));
            }
          }
        }
      }
      let _ = store().update(|doc| {
        mark_fired(doc, to_mark.clone());
        Ok::<(), String>(())
      });
    }
    for (id, _key) in due {
      let schedule = store().read().schedules.into_iter().find(|s| s.id == id);
      if let Some(schedule) = schedule {
        tokio::spawn(fire_schedule(schedule, "schedule"));
      }
    }
    tokio::time::sleep(StdDuration::from_secs(TICK_SECONDS)).await;
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::TimeZone;

  fn sunday_morning() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 25, 9, 0, 0).unwrap()
  }

  fn weekly_body() -> Value {
    json!({
      "name": "Nightly warm",
      "recipeId": "r1",
      "target": "desktop",
      "profileIds": ["p1", "p2"],
      "cadence": "weekly",
      "daysMask": 127,
      "runAtMinute": 540,
      "timezone": "Europe/Berlin",
      "webhookUrl": "https://hooks.example/abc",
    })
  }

  #[test]
  fn a_weekly_body_validates_with_a_generated_secret() {
    let schedule = validate(&weekly_body(), None).expect("valid");
    assert_eq!(schedule.name, "Nightly warm");
    assert_eq!(schedule.profile_ids.len(), 2);
    assert!(schedule.enabled);
    assert!(schedule.webhook_secret.is_some());
    assert_eq!(
      schedule.webhook_url.as_deref(),
      Some("https://hooks.example/abc")
    );
  }

  #[test]
  fn the_fleet_target_is_refused() {
    let mut body = weekly_body();
    body["target"] = json!("fleet");
    assert_eq!(validate(&body, None).unwrap_err(), invalid());
  }

  #[test]
  fn a_missing_recipe_a_missing_name_and_an_empty_profile_list_are_refused() {
    let mut body = weekly_body();
    body["recipeId"] = Value::Null;
    assert!(validate(&body, None).is_err());
    let mut body = weekly_body();
    body["name"] = json!("   ");
    assert!(validate(&body, None).is_err());
    let mut body = weekly_body();
    body["profileIds"] = json!([]);
    assert!(validate(&body, None).is_err());
  }

  #[test]
  fn an_interval_out_of_range_is_refused() {
    let mut body = weekly_body();
    body["cadence"] = json!("interval");
    body["intervalMinutes"] = json!(10);
    assert!(validate(&body, None).is_err());
    body["intervalMinutes"] = json!(20_000);
    assert!(validate(&body, None).is_err());
    body["intervalMinutes"] = json!(60);
    assert!(validate(&body, None).is_ok());
  }

  #[test]
  fn a_plain_http_webhook_is_refused_but_null_clears_it() {
    let mut body = weekly_body();
    body["webhookUrl"] = json!("http://hooks.example/abc");
    assert!(validate(&body, None).is_err());
    body["webhookUrl"] = Value::Null;
    let schedule = validate(&body, None).expect("null clears");
    assert!(schedule.webhook_url.is_none());
  }

  #[test]
  fn a_patch_rotates_the_secret_and_keeps_the_rest() {
    let base = validate(&weekly_body(), None).expect("valid");
    let first_secret = base.webhook_secret.clone().unwrap();
    let patch = json!({ "rotateWebhookSecret": true });
    let patched = validate(&patch, Some(&base)).expect("patch applies");
    assert_eq!(patched.name, base.name);
    assert_ne!(
      patched.webhook_secret.as_deref(),
      Some(first_secret.as_str())
    );
  }

  #[test]
  fn due_weekly_slots_fire_once() {
    let mut doc = Doc::default();
    let mut schedule = AgentSchedule {
      id: "s1".to_string(),
      cadence: "weekly".to_string(),
      days_mask: 0b0100_0000, // Sunday
      run_at_minute: 9 * 60,
      timezone: "UTC".to_string(),
      ..Default::default()
    };
    schedule.enabled = true;
    doc.schedules.push(schedule);

    // Sunday 09:10 UTC — due.
    let now = Utc.with_ymd_and_hms(2026, 10, 25, 9, 10, 0).unwrap();
    assert_eq!(due_fires(&doc, now).len(), 1);

    // Marked fired → not due again at the same slot.
    let keys: Vec<String> = due_fires(&doc, now)
      .into_iter()
      .map(|(_, key)| key)
      .collect();
    mark_fired(&mut doc, keys);
    assert!(due_fires(&doc, now).is_empty());
  }

  #[test]
  fn a_once_schedule_past_its_window_is_missed_not_queued() {
    let mut doc = Doc::default();
    doc.schedules.push(AgentSchedule {
      id: "s2".to_string(),
      enabled: true,
      cadence: "once".to_string(),
      run_once_at: Some("2026-10-25T06:00:00Z".to_string()),
      ..Default::default()
    });
    // Six hours later — inside the window.
    let inside = Utc.with_ymd_and_hms(2026, 10, 25, 11, 0, 0).unwrap();
    assert_eq!(due_fires(&doc, inside).len(), 1);
    // A day later — missed.
    let missed = Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap();
    assert!(due_fires(&doc, missed).is_empty());
  }

  #[test]
  fn interval_indexes_advance_and_never_repeat() {
    let mut doc = Doc::default();
    doc.schedules.push(AgentSchedule {
      id: "s3".to_string(),
      enabled: true,
      cadence: "interval".to_string(),
      interval_minutes: Some(30),
      created_at: Some("2026-10-25T09:00:00Z".to_string()),
      ..Default::default()
    });
    let now = Utc.with_ymd_and_hms(2026, 10, 25, 10, 5, 0).unwrap();
    let first: Vec<String> = due_fires(&doc, now).into_iter().map(|(_, k)| k).collect();
    assert_eq!(first.len(), 1);
    mark_fired(&mut doc, first);
    // Still inside the same interval — no new fire.
    let same_slot = Utc.with_ymd_and_hms(2026, 10, 25, 10, 10, 0).unwrap();
    assert!(due_fires(&doc, same_slot).is_empty());
    // The next interval opens a new key.
    let next_slot = Utc.with_ymd_and_hms(2026, 10, 25, 10, 35, 0).unwrap();
    assert_eq!(due_fires(&doc, next_slot).len(), 1);
  }

  #[test]
  fn the_signature_is_a_stable_hmac_hex() {
    let a = sign_webhook("secret", "{\"x\":1}");
    let b = sign_webhook("secret", "{\"x\":1}");
    let c = sign_webhook("other", "{\"x\":1}");
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert!(a.starts_with("sha256="));
    assert_eq!(a.len(), "sha256=".len() + 64);
  }

  #[test]
  fn next_fire_handles_every_cadence() {
    let weekly = AgentSchedule {
      enabled: true,
      cadence: "weekly".to_string(),
      days_mask: 0b0000_0001, // Monday
      run_at_minute: 9 * 60 + 30,
      timezone: "Europe/Berlin".to_string(),
      ..Default::default()
    };
    let fire = next_fire(&weekly, sunday_morning()).unwrap();
    assert_eq!(fire, Utc.with_ymd_and_hms(2026, 10, 26, 8, 30, 0).unwrap());

    let once = AgentSchedule {
      enabled: true,
      cadence: "once".to_string(),
      run_once_at: Some("2026-10-26T12:00:00Z".to_string()),
      ..Default::default()
    };
    assert_eq!(
      next_fire(&once, sunday_morning()).unwrap(),
      Utc.with_ymd_and_hms(2026, 10, 26, 12, 0, 0).unwrap()
    );

    let past = once.clone();
    assert!(next_fire(&past, Utc.with_ymd_and_hms(2026, 10, 27, 0, 0, 0).unwrap()).is_none());

    let interval = AgentSchedule {
      enabled: true,
      cadence: "interval".to_string(),
      interval_minutes: Some(60),
      created_at: Some("2026-10-25T09:00:00Z".to_string()),
      ..Default::default()
    };
    assert_eq!(
      next_fire(&interval, sunday_morning()).unwrap(),
      Utc.with_ymd_and_hms(2026, 10, 25, 10, 0, 0).unwrap()
    );

    let disabled = AgentSchedule {
      enabled: false,
      ..weekly
    };
    assert!(next_fire(&disabled, sunday_morning()).is_none());
  }
}
