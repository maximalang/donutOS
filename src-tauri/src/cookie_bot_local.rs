//! Local cookie-bot engine (free build): storage, due-time math, executor,
//! scheduler, presets, usage — everything the vendor cloud served for this
//! surface, on this machine.
//!
//! The wire types (`CookieBotSchedule`, `CookieBotRun`, …) and the public
//! function shapes are the ones `cookie_bot.rs` already speaks, so Tauri
//! commands, the REST mirror under `/v1/cookie-bot/*` and the OpenAPI
//! document do not change. What changes is the transport: no
//! `cloud_api::call`, one JSON document per surface under
//! `<data>/automation/` (see `local_store.rs`).
//!
//! Execution drives the browser through the existing MCP tool surface
//! (`McpServer::call_tool`): `run_profile` to launch, `navigate` / `scroll`
//! to warm, `kill_profile` to stop. There is deliberately no second browser
//! driver here to drift — a warming run behaves exactly like an agent
//! calling the same tools, same engine paths, same rate limiter.
//!
//! Not ported from the vendor cloud, on purpose: curated server-side
//! browsing templates (vendor content we do not have — `templates: []`),
//! teammates' conflict detection (no teammates in a single-owner store),
//! remote-fleet execution (rented vendor hardware), and email notices (no
//! local SMTP — failures log instead).

use crate::automation_time::{
  due_today, next_weekly_fire, valid_timezone, weekly_fire_key, WeeklySlot,
};
use crate::cookie_bot::{
  CookieBotConflictCheck, CookieBotError, CookieBotLimits, CookieBotPreset, CookieBotPresetList,
  CookieBotRun, CookieBotRunPage, CookieBotRunStarted, CookieBotSchedule, CookieBotScheduleDeleted,
  CookieBotScheduleInput, CookieBotScheduleList, CookieBotScheduleSaved, CookieBotUsage,
  CookieBotUsageMember, CookieBotUsageProfile, CookieBotUserTemplate, RemoteHoursBreakdown,
  RemoteHoursMember, RemoteHoursQuota,
};
use crate::local_store::LocalStore;
use crate::mcp_server::McpServer;
use chrono::{DateTime, Utc};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration as StdDuration;

// --- Limits and presets (published locally, same shape as the cloud's) ------

pub(crate) const MIN_MINUTES: u32 = 10;
pub(crate) const MAX_MINUTES: u32 = 480;
pub(crate) const MIN_SITES: usize = 1;
pub(crate) const MAX_SITES: usize = 25;
pub(crate) const MAX_SLOTS: usize = 8;
pub(crate) const MAX_TEMPLATE_NAME_LENGTH: usize = 80;
pub(crate) const MAX_SITES_TOTAL: usize = 25;

const PRESETS: [(&str, u32, bool); 3] = [
  ("light", 20, false),
  ("balanced", 45, true),
  ("deep", 90, false),
];
const DEFAULT_JITTER_SECONDS: u32 = 120;
const MAX_JITTER_SECONDS: u32 = 3600;
/// How long after a slot's local time the scheduler still considers it due.
/// Beyond this the slot was missed (machine off) and is skipped, not queued.
const FIRE_WINDOW_MINUTES: i64 = 45;

// --- Documents ----------------------------------------------------------------

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct CookieBotDoc {
  #[serde(default)]
  schedules: Vec<CookieBotSchedule>,
  #[serde(default)]
  runs: Vec<CookieBotRun>,
  /// Firing-slot identities already fired, newest last, bounded.
  #[serde(default)]
  fired: Vec<String>,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct UserTemplateDoc {
  #[serde(default)]
  templates: Vec<CookieBotUserTemplate>,
}

fn doc_store() -> &'static LocalStore<CookieBotDoc> {
  static STORE: OnceLock<LocalStore<CookieBotDoc>> = OnceLock::new();
  STORE.get_or_init(|| LocalStore::new("cookie-bot"))
}

fn template_store() -> &'static LocalStore<UserTemplateDoc> {
  static STORE: OnceLock<LocalStore<UserTemplateDoc>> = OnceLock::new();
  STORE.get_or_init(|| LocalStore::new("cookie-bot-user-templates"))
}

/// One warming job, handed to the executor task.
struct WarmingJob {
  run_id: String,
  profile_id: String,
  profile_name: String,
  sites: Vec<String>,
  minutes: u32,
  flag: Arc<AtomicBool>,
}

/// The executor's inbound queue. `start_run` enqueues; `run_executor`
/// (spawned once at app setup) drains. The queue breaks the async type
/// cycle start_run -> execute_warming -> MCP dispatch -> run_cookie_bot_now
/// tool -> start_run, which rustc can neither type nor prove `Send` across —
/// and a one-at-a-time executor is the honest budget for one machine.
type WarmingChannel = (
  tokio::sync::mpsc::UnboundedSender<WarmingJob>,
  std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<WarmingJob>>>,
);

fn warming_channel() -> &'static WarmingChannel {
  static CHANNEL: OnceLock<WarmingChannel> = OnceLock::new();
  CHANNEL.get_or_init(|| {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    (sender, std::sync::Mutex::new(Some(receiver)))
  })
}

/// Hand a started run to the executor. A send fails only when the executor
/// is gone; the run row then stays `running` and is failed by hand — the
/// store never claims a warm that did not happen.
fn enqueue_warming(job: WarmingJob) -> bool {
  warming_channel().0.send(job).is_ok()
}

/// In-memory cancel flags for running/pending runs, keyed by run id.
fn cancels() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
  static CANCELS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
  CANCELS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_iso() -> String {
  Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn invalid(code: &str) -> CookieBotError {
  CookieBotError(crate::cloud_errors::BackendFailure {
    status: 400,
    code: code.to_string(),
    params: Default::default(),
  })
}

fn not_found(code: &str) -> CookieBotError {
  CookieBotError(crate::cloud_errors::BackendFailure {
    status: 404,
    code: code.to_string(),
    params: Default::default(),
  })
}

// --- Site and name validation --------------------------------------------------

/// A site is an http(s) URL, short enough for the wire, no duplicates.
/// Returns the cleaned list or the schedule error code.
fn clean_sites(sites: &[String]) -> Result<Vec<String>, CookieBotError> {
  let bad = || invalid("COOKIE_BOT_INVALID_SCHEDULE");
  let mut seen = std::collections::HashSet::new();
  let mut clean = Vec::with_capacity(sites.len());
  for site in sites {
    let trimmed = site.trim();
    if trimmed.len() > 2048 {
      return Err(bad());
    }
    let url = url::Url::parse(trimmed).map_err(|_| bad())?;
    if url.scheme() != "http" && url.scheme() != "https" {
      return Err(bad());
    }
    if url.username() != "" || url.password().is_some() {
      return Err(bad());
    }
    if seen.insert(trimmed.to_string()) {
      clean.push(trimmed.to_string());
    }
  }
  Ok(clean)
}

fn clean_template_name(name: &str) -> Result<String, CookieBotError> {
  let trimmed = name.trim();
  if trimmed.is_empty() || trimmed.len() > MAX_TEMPLATE_NAME_LENGTH {
    return Err(invalid("COOKIE_BOT_INVALID_TEMPLATE_NAME"));
  }
  Ok(trimmed.to_string())
}

// --- Schedules -----------------------------------------------------------------

/// Build the stored schedule from an input, validating everything the cloud
/// validated. Pure over the given stores, so it is unit-testable.
fn build_schedule(
  profile_id: &str,
  input: &CookieBotScheduleInput,
  templates: &UserTemplateDoc,
  last_run: Option<(&str, &str)>,
  now: DateTime<Utc>,
) -> Result<CookieBotSchedule, CookieBotError> {
  let bad = || invalid("COOKIE_BOT_INVALID_SCHEDULE");
  if !valid_timezone(&input.timezone) {
    return Err(bad());
  }
  let preset = if input.preset.is_empty() {
    "balanced"
  } else {
    input.preset.as_str()
  };
  if !PRESETS.iter().any(|(id, _, _)| *id == preset) {
    return Err(bad());
  }
  if !(MIN_MINUTES..=MAX_MINUTES).contains(&input.max_minutes) {
    return Err(bad());
  }

  // Slots: either the multi-slot list or the single scalar pair.
  let mut slots: Vec<(u8, u16)> = Vec::new();
  match &input.slots {
    Some(list) => {
      if list.is_empty() || list.len() > MAX_SLOTS {
        return Err(bad());
      }
      for slot in list {
        if slot.days_mask == 0 || slot.days_mask > 127 {
          return Err(bad());
        }
        if slot.run_at_minute > 1439 {
          return Err(bad());
        }
        slots.push((slot.days_mask, slot.run_at_minute));
      }
    }
    None => {
      if input.days_mask == 0 || input.days_mask > 127 {
        return Err(bad());
      }
      if input.run_at_minute > 1439 {
        return Err(bad());
      }
      slots.push((input.days_mask, input.run_at_minute));
    }
  }

  // Sites: exactly one of a user template or an explicit list.
  let sites: Vec<String> = match &input.template_id {
    Some(template_id) => {
      if !input.sites.is_empty() {
        return Err(bad());
      }
      let found = templates
        .templates
        .iter()
        .find(|t| &t.id == template_id)
        .ok_or_else(|| {
          // The curated vendor pools are cloud content the free build does
          // not have and will not fabricate; only user templates exist.
          not_found("COOKIE_BOT_TEMPLATE_NOT_FOUND")
        })?;
      found.sites.clone()
    }
    None => {
      let clean = clean_sites(&input.sites)?;
      if clean.len() < MIN_SITES || clean.len() > MAX_SITES {
        return Err(bad());
      }
      clean
    }
  };

  let jitter = input
    .jitter_seconds
    .unwrap_or(DEFAULT_JITTER_SECONDS)
    .min(MAX_JITTER_SECONDS);

  let blocked_by = if !input.enabled {
    Some("disabled")
  } else if !input.has_proxy {
    Some("proxy_required")
  } else {
    None
  };

  let next_run_at = if input.enabled {
    slots
      .iter()
      .filter_map(|&(days_mask, run_at_minute)| {
        next_weekly_fire(
          WeeklySlot {
            days_mask,
            run_at_minute: run_at_minute as u32,
          },
          &input.timezone,
          now,
        )
      })
      .min()
      .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
  } else {
    None
  };

  Ok(CookieBotSchedule {
    profile_id: profile_id.to_string(),
    profile_name: input.profile_name.clone(),
    platform: input.platform.clone(),
    enabled: input.enabled,
    run_at_minute: slots[0].1,
    days_mask: slots[0].0,
    slots: slots
      .iter()
      .map(
        |&(days_mask, run_at_minute)| crate::cookie_bot::CookieBotSlot {
          days_mask,
          run_at_minute,
        },
      )
      .collect(),
    timezone: input.timezone.clone(),
    preset: preset.to_string(),
    template_id: input.template_id.clone(),
    max_minutes: input.max_minutes,
    sites,
    jitter_seconds: jitter,
    sync_enabled: input.sync_enabled,
    encrypted_sync: input.encrypted_sync,
    has_proxy: input.has_proxy,
    proxy_remote_reachable: input.proxy_remote_reachable,
    touch_fingerprint: input.touch_fingerprint,
    sticky_exit: input.sticky_exit,
    profile_state_at: Some(now_iso()),
    blocked_by: blocked_by.map(str::to_string),
    next_run_at,
    last_run_at: last_run.map(|(at, _id)| at.to_string()),
    last_run_id: last_run.map(|(_at, id)| id.to_string()),
    owner_user_id: None,
    owner_email: None,
    updated_at: Some(now_iso()),
  })
}

pub(crate) async fn list_schedules(
  scope: Option<&str>,
) -> Result<CookieBotScheduleList, CookieBotError> {
  let mut schedules = doc_store().read().schedules;
  schedules.sort_by(|a, b| a.profile_name.cmp(&b.profile_name));
  Ok(CookieBotScheduleList {
    schedules,
    team_id: None,
    scope: Some(scope.unwrap_or("local").to_string()),
  })
}

pub(crate) async fn get_schedule(
  profile_id: &str,
) -> Result<Option<CookieBotSchedule>, CookieBotError> {
  Ok(
    doc_store()
      .read()
      .schedules
      .into_iter()
      .find(|s| s.profile_id == profile_id),
  )
}

pub(crate) async fn save_schedule(
  profile_id: &str,
  input: &CookieBotScheduleInput,
  _acknowledge_conflict: bool,
) -> Result<CookieBotScheduleSaved, CookieBotError> {
  let templates = template_store().read();
  let now = Utc::now();
  let outcome = doc_store()
    .update(|doc| {
      let last = doc
        .runs
        .iter()
        .filter(|r| r.profile_id == profile_id && r.ended_at.is_some())
        .max_by(|a, b| a.ended_at.cmp(&b.ended_at))
        .map(|r| (r.ended_at.clone().unwrap_or_default(), r.id.clone()));
      let built = build_schedule(
        profile_id,
        input,
        &templates,
        last.as_ref().map(|(a, b)| (a.as_str(), b.as_str())),
        now,
      )?;
      doc.schedules.retain(|s| s.profile_id != profile_id);
      doc.schedules.push(built.clone());
      Ok::<CookieBotSchedule, CookieBotError>(built)
    })
    .map_err(|e| {
      log::error!("Cookie-bot schedule store write failed: {e}");
      invalid("COOKIE_BOT_INVALID_SCHEDULE")
    })?;
  Ok(CookieBotScheduleSaved {
    schedule: outcome,
    conflicts: Vec::new(),
  })
}

pub(crate) async fn delete_schedule(
  profile_id: &str,
) -> Result<CookieBotScheduleDeleted, CookieBotError> {
  let removed = doc_store()
    .update(|doc| {
      let before = doc.schedules.len();
      doc.schedules.retain(|s| s.profile_id != profile_id);
      Ok::<bool, String>(before != doc.schedules.len())
    })
    .map_err(|e| {
      log::error!("Cookie-bot schedule store write failed: {e}");
      invalid("COOKIE_BOT_INVALID_SCHEDULE")
    })?;
  Ok(CookieBotScheduleDeleted {
    profile_id: profile_id.to_string(),
    deleted: removed,
  })
}

pub(crate) async fn check_conflicts(
  profile_id: &str,
  _run_at_minute: Option<u16>,
  _timezone: Option<&str>,
  _days_mask: Option<u8>,
) -> Result<CookieBotConflictCheck, CookieBotError> {
  // Conflicts are teammates enrolling the same profile on the vendor
  // account; a single-owner local store has none.
  Ok(CookieBotConflictCheck {
    profile_id: profile_id.to_string(),
    conflicts: Vec::new(),
  })
}

// --- Runs ----------------------------------------------------------------------

fn status_rank(status: &str) -> u8 {
  match status {
    "running" | "pending" => 0,
    _ => 1,
  }
}

/// Start a warming run for an enrolled profile and hand it to the executor.
/// `trigger` is "manual" (user pressed run) or "schedule" (the tick fired).
async fn start_run(
  profile_id: &str,
  max_minutes: Option<u32>,
  trigger: &str,
) -> Result<CookieBotRunStarted, CookieBotError> {
  let conflict = || {
    CookieBotError(crate::cloud_errors::BackendFailure {
      status: 409,
      code: "COOKIE_BOT_RUN_IN_PROGRESS".to_string(),
      params: Default::default(),
    })
  };
  let schedule = doc_store()
    .read()
    .schedules
    .into_iter()
    .find(|s| s.profile_id == profile_id)
    .ok_or_else(|| not_found("COOKIE_BOT_NOT_ENROLLED"))?;
  let minutes = max_minutes
    .unwrap_or(schedule.max_minutes)
    .clamp(MIN_MINUTES, MAX_MINUTES);

  let created = doc_store()
    .update(|doc| {
      if doc
        .runs
        .iter()
        .any(|r| r.profile_id == profile_id && status_rank(&r.status) == 0)
      {
        return Err(conflict());
      }
      let run = CookieBotRun {
        id: uuid::Uuid::new_v4().to_string(),
        profile_id: profile_id.to_string(),
        profile_name: Some(schedule.profile_name.clone()),
        user_id: None,
        email: None,
        team_id: None,
        trigger: trigger.to_string(),
        status: "running".to_string(),
        scheduled_for: now_iso(),
        dispatch_after: None,
        started_at: Some(now_iso()),
        ended_at: None,
        max_minutes: minutes,
        chunks_total: 1,
        chunk_index: 1,
        sites_total: schedule.sites.len() as u32,
        sites_visited: 0,
        sites_failed: 0,
        consent_dismissed: 0,
        billed_seconds: 0,
        outcome_code: None,
        session_id: None,
      };
      doc.runs.push(run.clone());
      Ok::<CookieBotRun, CookieBotError>(run)
    })
    .map_err(|e: String| {
      // A deliberate refusal from the update — a run already in progress —
      // round-trips through the store's flat String error. Hand it back as
      // the typed refusal instead of masking it as a schedule failure.
      if e.contains("COOKIE_BOT_RUN_IN_PROGRESS") {
        return CookieBotError(crate::cloud_errors::BackendFailure {
          status: 409,
          code: "COOKIE_BOT_RUN_IN_PROGRESS".to_string(),
          params: Default::default(),
        });
      }
      log::error!("Cookie-bot run store write failed: {e}");
      invalid("COOKIE_BOT_INVALID_SCHEDULE")
    })?;
  let run = created;
  let flag = {
    let mut registry = cancels().lock().unwrap();
    registry
      .entry(run.id.clone())
      .or_insert_with(|| Arc::new(AtomicBool::new(false)))
      .clone()
  };
  let sites = schedule.sites.clone();
  let profile_name = schedule.profile_name.clone();
  let queued = enqueue_warming(WarmingJob {
    run_id: run.id.clone(),
    profile_id: profile_id.to_string(),
    profile_name,
    sites,
    minutes,
    flag,
  });
  if !queued {
    log::error!(
      "Cookie-bot executor is gone; run {} stays unexecuted",
      run.id
    );
  }
  Ok(CookieBotRunStarted {
    run,
    session_id: None,
  })
}

pub(crate) async fn run_now(
  profile_id: &str,
  max_minutes: Option<u32>,
) -> Result<CookieBotRunStarted, CookieBotError> {
  start_run(profile_id, max_minutes, "manual").await
}

/// Fire a scheduled slot after its jitter delay. Called by the tick task.
async fn fire_scheduled(profile_id: String, jitter_seconds: u32) {
  let mut waited = 0u32;
  while waited < jitter_seconds {
    let step = StdDuration::from_secs(1);
    tokio::time::sleep(step).await;
    waited += 1;
  }
  match start_run(&profile_id, None, "schedule").await {
    Ok(_) => log::info!("Cookie-bot scheduled run started profile={profile_id}"),
    Err(e) => log::warn!(
      "Cookie-bot scheduled run refused profile={profile_id} code={}",
      e.0.code
    ),
  }
}

pub(crate) async fn cancel_run(run_id: &str) -> Result<CookieBotRun, CookieBotError> {
  if let Some(flag) = cancels().lock().unwrap().get(run_id) {
    flag.store(true, Ordering::SeqCst);
  }
  let found: Option<CookieBotRun> = doc_store()
    .update(|doc| {
      Ok::<Option<CookieBotRun>, String>(doc.runs.iter_mut().find(|r| r.id == run_id).map(|r| {
        if status_rank(&r.status) == 0 {
          r.status = "cancelled".to_string();
          r.ended_at = Some(now_iso());
        }
        r.clone()
      }))
    })
    .ok()
    .flatten();
  found.ok_or_else(|| not_found("COOKIE_BOT_RUN_NOT_FOUND"))
}

pub(crate) async fn list_runs(
  profile_id: Option<&str>,
  _scope: Option<&str>,
  limit: Option<u32>,
  before: Option<&str>,
) -> Result<CookieBotRunPage, CookieBotError> {
  let page = runs_page(&doc_store().read(), profile_id, limit, before);
  Ok(page)
}

/// Keyset pagination over the run history, newest first. Pure, testable.
fn runs_page(
  doc: &CookieBotDoc,
  profile_id: Option<&str>,
  limit: Option<u32>,
  before: Option<&str>,
) -> CookieBotRunPage {
  let limit = limit.unwrap_or(20).clamp(1, 100) as usize;
  let mut runs: Vec<&CookieBotRun> = doc
    .runs
    .iter()
    .filter(|r| profile_id.map(|p| r.profile_id == p).unwrap_or(true))
    .collect();
  runs.sort_by(|a, b| b.scheduled_for.cmp(&a.scheduled_for).then(b.id.cmp(&a.id)));
  let mut items = runs.as_slice();
  if let Some(before) = before {
    if let Some(pos) = items.iter().position(|r| r.id == before) {
      items = &items[pos + 1..];
    } else {
      // An unknown cursor is an empty page, not a full one.
      items = &[];
    }
  }
  let take = items.len().min(limit);
  let page: Vec<CookieBotRun> = items[..take].iter().map(|r| (*r).clone()).collect();
  let next_before = if take < items.len() {
    page.last().map(|r| r.id.clone())
  } else {
    None
  };
  CookieBotRunPage {
    runs: page,
    next_before,
  }
}

// --- The warming executor ------------------------------------------------------

/// Dwell pause between 2 and 6 seconds, in cancel-friendly slices.
async fn dwell(flag: &AtomicBool, low_ms: u64, high_ms: u64) -> bool {
  let span = high_ms - low_ms;
  let total = low_ms + (rand_slice() % span.max(1));
  let mut slept = 0u64;
  while slept < total {
    tokio::time::sleep(StdDuration::from_millis(250)).await;
    slept += 250;
    if flag.load(Ordering::SeqCst) {
      return false;
    }
  }
  true
}

/// A cheap time source for jittered pauses; nanos from the clock.
fn rand_slice() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.subsec_nanos() as u64)
    .unwrap_or(0)
}

/// Drive one MCP tool, exactly as a remote agent would.
async fn call_tool(name: &str, arguments: serde_json::Value) -> Result<serde_json::Value, String> {
  McpServer::instance().call_tool(name, &arguments).await
}

/// One warming run: launch the profile, cycle the site list with navigate +
/// scroll dwells until the minute budget is spent, then stop the browser and
/// record the outcome. Steps run through the MCP tool surface, so behaviour
/// matches an agent driving the same tools.
async fn execute_warming(
  run_id: String,
  profile_id: String,
  profile_name: String,
  sites: Vec<String>,
  max_minutes: u32,
  flag: Arc<AtomicBool>,
) {
  let started = std::time::Instant::now();
  let deadline = started + StdDuration::from_secs(max_minutes as u64 * 60);
  let mut visited: u32 = 0;
  let mut failed: u32 = 0;
  let mut outcome: Option<String> = None;

  if call_tool("run_profile", json!({ "profile_id": profile_id }))
    .await
    .is_err()
  {
    outcome = Some("launch_failed".to_string());
    failed = sites.len() as u32;
  } else if !sites.is_empty() {
    let mut index = 0usize;
    loop {
      if flag.load(Ordering::SeqCst) {
        outcome = Some("cancelled".to_string());
        break;
      }
      if std::time::Instant::now() >= deadline {
        break;
      }
      let site = &sites[index % sites.len()];
      if call_tool("navigate", json!({ "profile_id": profile_id, "url": site }))
        .await
        .is_err()
      {
        failed += 1;
      } else if !dwell(&flag, 2_000, 6_000).await {
        outcome = Some("cancelled".to_string());
        break;
      // Scroll is vendor-only: the vendor browser build gates it (and other
      // automation commands) behind a paid-plan entitlement token its cloud
      // mints. Free warming counts the site on navigate + dwell alone — the
      // page loads and cookies land either way.
      } else if !cfg!(feature = "free")
        && call_tool(
          "scroll",
          json!({ "profile_id": profile_id, "direction": "down", "pages": 1 }),
        )
        .await
        .is_err()
      {
        failed += 1;
      } else if !dwell(&flag, 1_000, 3_000).await {
        outcome = Some("cancelled".to_string());
        break;
      } else {
        visited += 1;
      }
      index += 1;
    }
  }

  let _ = call_tool("kill_profile", json!({ "profile_id": profile_id })).await;

  let cancelled = outcome.as_deref() == Some("cancelled");
  let status = if cancelled {
    "cancelled"
  } else if outcome.is_some() || visited == 0 {
    "failed"
  } else if failed > 0 {
    "partial"
  } else {
    "succeeded"
  }
  .to_string();
  let billed = started.elapsed().as_secs();

  let finished = doc_store().update(|doc| {
    if let Some(run) = doc.runs.iter_mut().find(|r| r.id == run_id) {
      run.status = status.clone();
      run.ended_at = Some(now_iso());
      run.sites_visited = visited;
      run.sites_failed = failed;
      run.billed_seconds = billed;
      run.outcome_code = outcome.clone().or_else(|| {
        if status == "failed" {
          Some("no_site_succeeded".to_string())
        } else {
          None
        }
      });
    }
    // Keep the history bounded: the newest 500 runs stay.
    if doc.runs.len() > 500 {
      let cut = doc.runs.len() - 500;
      doc.runs.drain(0..cut);
    }
    Ok::<(), String>(())
  });
  if let Err(e) = finished {
    log::error!("Cookie-bot run record not persisted run={run_id}: {e}");
  }
  cancels().lock().unwrap().remove(&run_id);
  log::info!(
    "Cookie-bot run finished run={run_id} profile={profile_id} name={profile_name} status={status} visited={visited} failed={failed} billed={billed}s"
  );
}

// --- Presets, usage, quota ------------------------------------------------------

pub(crate) async fn list_presets() -> Result<CookieBotPresetList, CookieBotError> {
  Ok(CookieBotPresetList {
    presets: PRESETS
      .iter()
      .map(|&(id, minutes, recommended)| CookieBotPreset {
        id: id.to_string(),
        typical_minutes: Some(minutes),
        recommended,
        name: None,
        description: None,
      })
      .collect(),
    default_preset: Some("balanced".to_string()),
    templates: Vec::new(),
    limits: Some(CookieBotLimits {
      min_minutes: Some(MIN_MINUTES),
      max_minutes: Some(MAX_MINUTES),
      min_sites: Some(MIN_SITES as u32),
      max_sites: Some(MAX_SITES as u32),
      max_slots: Some(MAX_SLOTS as u32),
      max_template_name_length: Some(MAX_TEMPLATE_NAME_LENGTH as u32),
    }),
  })
}

/// Monthly bot-hours usage from the local run history. Pure over the doc.
fn usage_for(doc: &CookieBotDoc, schedules: &[CookieBotSchedule], period: &str) -> CookieBotUsage {
  let month = if period.len() == 7 {
    period.to_string()
  } else {
    Utc::now().format("%Y-%m").to_string()
  };
  let in_period = |run: &CookieBotRun| run.scheduled_for.starts_with(&month);
  let runs: Vec<&CookieBotRun> = doc.runs.iter().filter(|r| in_period(r)).collect();
  let bot_seconds: u64 = runs.iter().map(|r| r.billed_seconds).sum();
  let bot_hours = bot_seconds as f64 / 3600.0;
  let failed = runs
    .iter()
    .filter(|r| r.status == "failed" || r.status == "partial")
    .count() as u32;

  let mut profiles: Vec<CookieBotUsageProfile> = Vec::new();
  let mut profile_ids: Vec<String> = runs
    .iter()
    .map(|r| r.profile_id.clone())
    .collect::<std::collections::HashSet<_>>()
    .into_iter()
    .collect();
  profile_ids.sort();
  for id in profile_ids {
    let mine: Vec<&&CookieBotRun> = runs.iter().filter(|r| r.profile_id == id).collect();
    let seconds: u64 = mine.iter().map(|r| r.billed_seconds).sum();
    let last = mine
      .iter()
      .max_by(|a, b| a.scheduled_for.cmp(&b.scheduled_for));
    profiles.push(CookieBotUsageProfile {
      profile_id: id.clone(),
      profile_name: schedules
        .iter()
        .find(|s| s.profile_id == id)
        .map(|s| s.profile_name.clone()),
      owner_email: None,
      bot_hours: seconds as f64 / 3600.0,
      runs: mine.len() as u32,
      runs_failed: mine
        .iter()
        .filter(|r| r.status == "failed" || r.status == "partial")
        .count() as u32,
      last_run_at: last.map(|r| r.scheduled_for.clone()),
      last_status: last.map(|r| r.status.clone()),
    });
  }

  CookieBotUsage {
    period: month.clone(),
    period_start: Some(format!("{month}-01T00:00:00Z")),
    period_end: None,
    team_id: None,
    seats: 1,
    granted_hours: 8760.0,
    used_hours: bot_hours,
    remaining_hours: (8760.0 - bot_hours).max(0.0),
    members: vec![CookieBotUsageMember {
      user_id: "local".to_string(),
      email: "local".to_string(),
      role: Some("owner".to_string()),
      interactive_hours: 0.0,
      bot_hours,
      used_hours: bot_hours,
      sessions: 0,
      bot_runs: runs.len() as u32,
      bot_runs_failed: failed,
    }],
    profiles,
  }
}

pub(crate) async fn team_usage(period: Option<&str>) -> Result<CookieBotUsage, CookieBotError> {
  let doc = doc_store().read();
  Ok(usage_for(&doc, &doc.schedules, period.unwrap_or("")))
}

pub(crate) async fn remote_hours_quota() -> Result<RemoteHoursQuota, CookieBotError> {
  let year = Utc::now().format("%Y").to_string();
  let doc = doc_store().read();
  let year_seconds: u64 = doc
    .runs
    .iter()
    .filter(|r| r.scheduled_for.starts_with(&year))
    .map(|r| r.billed_seconds)
    .sum();
  let bot_hours = year_seconds as f64 / 3600.0;
  Ok(RemoteHoursQuota {
    granted_hours: 8760.0,
    remaining_hours: (8760.0 - bot_hours).max(0.0),
    used_hours: bot_hours,
    period_start: Some(format!("{year}-01-01T00:00:00Z")),
    period_end: Some(format!("{year}-12-31T23:59:59Z")),
    scope: Some("local".to_string()),
    team_id: None,
    seats: 1,
    per_seat_hours: 8760.0,
    breakdown: Some(RemoteHoursBreakdown {
      interactive_hours: 0.0,
      bot_hours,
    }),
    members: vec![RemoteHoursMember {
      user_id: "local".to_string(),
      email: "local".to_string(),
      role: Some("owner".to_string()),
      used_hours: bot_hours,
      interactive_hours: 0.0,
      bot_hours,
    }],
  })
}

// --- User templates --------------------------------------------------------------

pub(crate) async fn list_user_templates() -> Result<Vec<CookieBotUserTemplate>, CookieBotError> {
  Ok(template_store().read().templates)
}

pub(crate) async fn create_user_template(
  name: &str,
  sites: &[String],
) -> Result<CookieBotUserTemplate, CookieBotError> {
  let name = clean_template_name(name)?;
  let sites = clean_sites(sites)?;
  if sites.len() < MIN_SITES || sites.len() > MAX_SITES_TOTAL {
    return Err(invalid("COOKIE_BOT_INVALID_SCHEDULE"));
  }
  let created = template_store()
    .update(|doc| {
      if doc
        .templates
        .iter()
        .any(|t| t.name.eq_ignore_ascii_case(&name))
      {
        return Err(invalid("COOKIE_BOT_TEMPLATE_NAME_TAKEN"));
      }
      let template = CookieBotUserTemplate {
        id: format!("user:{}", uuid::Uuid::new_v4()),
        name: name.clone(),
        sites: sites.clone(),
        updated_at: Some(now_iso()),
      };
      doc.templates.push(template.clone());
      Ok::<CookieBotUserTemplate, CookieBotError>(template)
    })
    .map_err(|e| {
      log::error!("Cookie-bot template store write failed: {e}");
      invalid("COOKIE_BOT_INVALID_TEMPLATE_NAME")
    })?;
  Ok(created)
}

/// Partial update, exactly as the wire client sends it: `None` keeps the
/// stored value for that field.
pub(crate) async fn update_user_template(
  id: &str,
  name: Option<&str>,
  sites: Option<&[String]>,
) -> Result<CookieBotUserTemplate, CookieBotError> {
  let updated = template_store()
    .update(|doc| {
      // Validate the name against every OTHER template before taking the
      // mutable borrow of the one being changed.
      let new_name = match name {
        Some(name) => {
          let name = clean_template_name(name)?;
          if doc
            .templates
            .iter()
            .any(|t| t.id != id && t.name.eq_ignore_ascii_case(&name))
          {
            return Err(invalid("COOKIE_BOT_TEMPLATE_NAME_TAKEN"));
          }
          Some(name)
        }
        None => None,
      };
      let new_sites = match sites {
        Some(sites) => {
          let sites = clean_sites(sites)?;
          if sites.len() < MIN_SITES || sites.len() > MAX_SITES_TOTAL {
            return Err(invalid("COOKIE_BOT_INVALID_SCHEDULE"));
          }
          Some(sites)
        }
        None => None,
      };
      let found = match doc.templates.iter_mut().find(|t| t.id == id) {
        Some(found) => found,
        None => return Ok::<Option<CookieBotUserTemplate>, CookieBotError>(None),
      };
      if let Some(name) = new_name {
        found.name = name;
      }
      if let Some(sites) = new_sites {
        found.sites = sites;
      }
      found.updated_at = Some(now_iso());
      Ok(Some(found.clone()))
    })
    .map_err(|e| {
      log::error!("Cookie-bot template store write failed: {e}");
      invalid("COOKIE_BOT_INVALID_TEMPLATE_NAME")
    })?;
  updated.ok_or_else(|| not_found("COOKIE_BOT_TEMPLATE_NOT_FOUND"))
}

pub(crate) async fn delete_user_template(id: &str) -> Result<bool, CookieBotError> {
  let removed = template_store()
    .update(|doc| {
      let before = doc.templates.len();
      doc.templates.retain(|t| t.id != id);
      Ok::<bool, String>(before != doc.templates.len())
    })
    .map_err(|e| {
      log::error!("Cookie-bot template store write failed: {e}");
      invalid("COOKIE_BOT_INVALID_TEMPLATE_NAME")
    })?;
  Ok(removed)
}

/// Runs left `running` by a previous process can never finish: their warming
/// executor died with the app. Mark them failed once at startup, or the stale
/// row blocks every future run for the profile.
fn reconcile_orphans(doc: &mut CookieBotDoc, now: String) -> u32 {
  let mut count = 0u32;
  for run in doc.runs.iter_mut() {
    if status_rank(&run.status) == 0 {
      run.status = "failed".to_string();
      run.ended_at = Some(now.clone());
      run.outcome_code = Some("app_restarted".to_string());
      count += 1;
    }
  }
  count
}

fn reconcile_orphan_runs() {
  match doc_store().update(|doc| {
    let count = reconcile_orphans(doc, now_iso());
    Ok::<u32, CookieBotError>(count)
  }) {
    Ok(count) if count > 0 => {
      log::info!("Cookie-bot reconciled {count} orphan run(s) left by a previous process");
    }
    Ok(_) => {}
    Err(e) => log::warn!("Cookie-bot orphan reconciliation failed: {e}"),
  }
}

/// The single warming executor: one run at a time, in start order. Spawned
/// once at app setup (free builds only).
pub(crate) async fn run_executor() {
  let mut receiver = warming_channel()
    .1
    .lock()
    .unwrap()
    .take()
    .expect("run_executor spawned twice");
  log::info!("Cookie-bot local executor started");
  reconcile_orphan_runs();
  while let Some(job) = receiver.recv().await {
    execute_warming(
      job.run_id,
      job.profile_id,
      job.profile_name,
      job.sites,
      job.minutes,
      job.flag,
    )
    .await;
  }
}

// --- The scheduler tick --------------------------------------------------------

/// The local instant of `slot` on today's date in `tz_name`, when its weekday
/// is selected and its time has passed within the fire window. Delegates to
/// the shared due-math so cookie-bot and agent schedules cannot drift.
fn slot_due_today(slot: WeeklySlot, tz_name: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
  due_today(slot, tz_name, now, FIRE_WINDOW_MINUTES)
}

/// Slots that should fire now: enabled schedules, due today, not yet fired.
/// Pure over the doc, so restart behaviour is testable.
fn due_slots(doc: &CookieBotDoc, now: DateTime<Utc>) -> Vec<(String, u32, String)> {
  let mut due = Vec::new();
  for schedule in &doc.schedules {
    if !schedule.enabled {
      continue;
    }
    for slot in &schedule.slots {
      let weekly = WeeklySlot {
        days_mask: slot.days_mask,
        run_at_minute: slot.run_at_minute as u32,
      };
      if let Some(fire) = slot_due_today(weekly, &schedule.timezone, now) {
        let key = weekly_fire_key(
          &format!("cb:{}", schedule.profile_id),
          weekly,
          &schedule.timezone,
          fire,
        );
        if !doc.fired.contains(&key) {
          due.push((schedule.profile_id.clone(), schedule.jitter_seconds, key));
        }
      }
    }
  }
  due
}

/// Mark a fire key consumed, keeping the log bounded.
fn mark_fired(doc: &mut CookieBotDoc, key: String) {
  doc.fired.retain(|k| *k != key);
  doc.fired.push(key);
  let cut = doc.fired.len().saturating_sub(500);
  if cut > 0 {
    doc.fired.drain(0..cut);
  }
}

/// Refresh every enabled schedule's `next_run_at` from its slots.
fn refresh_next_runs(doc: &mut CookieBotDoc, now: DateTime<Utc>) {
  for schedule in &mut doc.schedules {
    schedule.next_run_at = if schedule.enabled {
      schedule
        .slots
        .iter()
        .filter_map(|slot| {
          next_weekly_fire(
            WeeklySlot {
              days_mask: slot.days_mask,
              run_at_minute: slot.run_at_minute as u32,
            },
            &schedule.timezone,
            now,
          )
        })
        .min()
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
    } else {
      None
    };
  }
}

/// Entry point spawned at app setup (free builds only). One tick every 30 s.
/// The fire keys are recorded BEFORE the run is spawned, so a crash mid-run
/// can never re-fire the same slot on restart.
pub(crate) async fn run_scheduler() {
  log::info!("Cookie-bot local scheduler started (tick 30s)");
  loop {
    let now = Utc::now();
    let due: Vec<(String, u32, String)> = {
      let doc = doc_store().read();
      due_slots(&doc, now)
    };
    if !due.is_empty() {
      let _ = doc_store().update(|doc| {
        for (_, _, key) in &due {
          mark_fired(doc, key.clone());
        }
        refresh_next_runs(doc, now);
        Ok::<(), String>(())
      });
      for (profile_id, jitter, _key) in due {
        tokio::spawn(fire_scheduled(profile_id, jitter));
      }
    }
    tokio::time::sleep(StdDuration::from_secs(30)).await;
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::cookie_bot::{CookieBotScheduleInput, CookieBotSlot};
  use chrono::TimeZone;

  #[test]
  fn an_app_restart_fails_the_runs_it_left_running() {
    let mut doc = CookieBotDoc {
      runs: vec![
        test_run("running"),
        test_run("failed"),
        test_run("succeeded"),
      ],
      ..Default::default()
    };
    let count = reconcile_orphans(&mut doc, "2026-10-25T12:00:00Z".to_string());
    assert_eq!(count, 1, "only the non-terminal run is an orphan");
    assert_eq!(doc.runs[0].status, "failed");
    assert_eq!(doc.runs[0].outcome_code.as_deref(), Some("app_restarted"));
    assert_eq!(doc.runs[0].ended_at.as_deref(), Some("2026-10-25T12:00:00Z"));
    assert_eq!(doc.runs[1].status, "failed", "terminal runs stay untouched");
    assert_eq!(doc.runs[1].outcome_code, None);
    assert_eq!(doc.runs[2].status, "succeeded", "terminal runs stay untouched");
  }

  fn test_run(status: &str) -> CookieBotRun {
    CookieBotRun {
      id: uuid::Uuid::new_v4().to_string(),
      profile_id: "02f0bab1-0000-0000-0000-000000000000".to_string(),
      profile_name: Some("t".to_string()),
      user_id: None,
      email: None,
      team_id: None,
      trigger: "manual".to_string(),
      status: status.to_string(),
      scheduled_for: "2026-10-25T11:00:00Z".to_string(),
      dispatch_after: None,
      started_at: Some("2026-10-25T11:00:00Z".to_string()),
      ended_at: None,
      max_minutes: 10,
      chunks_total: 1,
      chunk_index: 1,
      sites_total: 1,
      sites_visited: 0,
      sites_failed: 0,
      consent_dismissed: 0,
      billed_seconds: 0,
      outcome_code: None,
      session_id: None,
    }
  }

  fn at(test: &str) -> DateTime<Utc> {
    let _ = test;
    Utc.with_ymd_and_hms(2026, 10, 25, 9, 0, 0).unwrap()
  }

  fn input() -> CookieBotScheduleInput {
    CookieBotScheduleInput {
      profile_name: "Shopping".to_string(),
      platform: "windows".to_string(),
      enabled: true,
      run_at_minute: 540,
      days_mask: 127,
      slots: None,
      timezone: "Europe/Berlin".to_string(),
      preset: "balanced".to_string(),
      template_id: None,
      max_minutes: 45,
      sites: vec!["https://example.com".to_string()],
      jitter_seconds: None,
      sync_enabled: false,
      has_proxy: true,
      proxy_remote_reachable: true,
      encrypted_sync: false,
      touch_fingerprint: false,
      sticky_exit: false,
    }
  }

  fn try_build(input: &CookieBotScheduleInput) -> Result<CookieBotSchedule, CookieBotError> {
    let templates = UserTemplateDoc::default();
    build_schedule("p1", input, &templates, None, at("now"))
  }

  fn built(input: CookieBotScheduleInput) -> CookieBotSchedule {
    try_build(&input).expect("valid input builds")
  }

  #[test]
  fn a_valid_input_builds_a_schedule_with_defaults() {
    let schedule = built(input());
    assert_eq!(schedule.jitter_seconds, DEFAULT_JITTER_SECONDS);
    assert_eq!(schedule.slots.len(), 1);
    assert_eq!(schedule.sites, vec!["https://example.com"]);
    assert!(schedule.blocked_by.is_none());
    assert!(schedule.next_run_at.is_some());
  }

  #[test]
  fn both_a_template_and_sites_are_refused() {
    let mut bad = input();
    bad.template_id = Some("user:x".to_string());
    assert_eq!(
      try_build(&bad).unwrap_err().0.code,
      "COOKIE_BOT_INVALID_SCHEDULE"
    );
  }

  #[test]
  fn an_invalid_timezone_is_refused() {
    let mut bad = input();
    bad.timezone = "Mars/Olympus".to_string();
    assert_eq!(
      try_build(&bad).unwrap_err().0.code,
      "COOKIE_BOT_INVALID_SCHEDULE"
    );
  }

  #[test]
  fn an_out_of_range_minute_budget_is_refused() {
    for minutes in [0, 9, 481, 6000] {
      let mut bad = input();
      bad.max_minutes = minutes;
      assert_eq!(
        try_build(&bad).unwrap_err().0.code,
        "COOKIE_BOT_INVALID_SCHEDULE"
      );
    }
  }

  #[test]
  fn an_unknown_curated_template_is_refused() {
    let mut bad = input();
    bad.sites = Vec::new();
    bad.template_id = Some("low-intent-purchaser".to_string());
    assert_eq!(
      try_build(&bad).unwrap_err().0.code,
      "COOKIE_BOT_TEMPLATE_NOT_FOUND"
    );
  }

  #[test]
  fn an_empty_site_list_is_refused() {
    let mut bad = input();
    bad.sites = Vec::new();
    assert_eq!(
      try_build(&bad).unwrap_err().0.code,
      "COOKIE_BOT_INVALID_SCHEDULE"
    );
  }

  #[test]
  fn a_non_http_site_is_refused() {
    let mut bad = input();
    bad.sites = vec!["ftp://example.com".to_string()];
    assert_eq!(
      try_build(&bad).unwrap_err().0.code,
      "COOKIE_BOT_INVALID_SCHEDULE"
    );
  }

  #[test]
  fn a_disabled_schedule_reports_its_block() {
    let mut disabled = input();
    disabled.enabled = false;
    let schedule = built(disabled);
    assert_eq!(schedule.blocked_by.as_deref(), Some("disabled"));
    assert!(schedule.next_run_at.is_none());
  }

  #[test]
  fn a_proxyless_schedule_reports_its_block() {
    let mut no_proxy = input();
    no_proxy.has_proxy = false;
    let schedule = built(no_proxy);
    assert_eq!(schedule.blocked_by.as_deref(), Some("proxy_required"));
  }

  #[test]
  fn multi_slot_inputs_mirror_the_first_slot_into_the_scalars() {
    let mut multi = input();
    multi.slots = Some(vec![
      CookieBotSlot {
        days_mask: 0b0000_0001,
        run_at_minute: 540,
      },
      CookieBotSlot {
        days_mask: 0b0100_0000,
        run_at_minute: 1200,
      },
    ]);
    let schedule = built(multi);
    assert_eq!(schedule.slots.len(), 2);
    assert_eq!(schedule.run_at_minute, 540);
    assert_eq!(schedule.days_mask, 0b0000_0001);
  }

  #[test]
  fn more_than_eight_slots_are_refused() {
    let mut multi = input();
    multi.slots = Some(
      (0..9)
        .map(|i| CookieBotSlot {
          days_mask: 1 << (i % 7),
          run_at_minute: 540,
        })
        .collect(),
    );
    assert!(try_build(&multi).is_err());
  }

  #[test]
  fn a_user_template_expands_into_sites() {
    let mut uses_template = input();
    uses_template.sites = Vec::new();
    uses_template.template_id = Some("user:abc".to_string());
    let templates = UserTemplateDoc {
      templates: vec![CookieBotUserTemplate {
        id: "user:abc".to_string(),
        name: "Shops".to_string(),
        sites: vec![
          "https://a.example".to_string(),
          "https://b.example".to_string(),
        ],
        updated_at: None,
      }],
    };
    let schedule = build_schedule("p1", &uses_template, &templates, None, at("template"))
      .expect("template expands");
    assert_eq!(schedule.sites.len(), 2);
    assert_eq!(schedule.template_id.as_deref(), Some("user:abc"));
  }

  #[test]
  fn runs_page_paginates_newest_first_with_a_keyset_cursor() {
    let mut doc = CookieBotDoc::default();
    for i in 0..5 {
      doc.runs.push(CookieBotRun {
        id: format!("r{i}"),
        profile_id: "p1".to_string(),
        profile_name: None,
        user_id: None,
        email: None,
        team_id: None,
        trigger: "manual".to_string(),
        status: "succeeded".to_string(),
        scheduled_for: format!("2026-10-2{i}T10:00:00Z"),
        dispatch_after: None,
        started_at: None,
        ended_at: None,
        max_minutes: 45,
        chunks_total: 1,
        chunk_index: 1,
        sites_total: 1,
        sites_visited: 1,
        sites_failed: 0,
        consent_dismissed: 0,
        billed_seconds: 60,
        outcome_code: None,
        session_id: None,
      });
    }
    let page = runs_page(&doc, None, Some(2), None);
    assert_eq!(
      page.runs.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
      vec!["r4", "r3"]
    );
    assert_eq!(page.next_before.as_deref(), Some("r3"));
    let next = runs_page(&doc, None, Some(2), page.next_before.as_deref());
    assert_eq!(
      next.runs.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
      vec!["r2", "r1"]
    );
    let filtered = runs_page(&doc, Some("other"), None, None);
    assert!(filtered.runs.is_empty());
    let unknown_cursor = runs_page(&doc, None, None, Some("nope"));
    assert!(unknown_cursor.runs.is_empty());
  }

  #[test]
  fn usage_aggregates_from_run_history() {
    let mut doc = CookieBotDoc::default();
    for i in 0..3 {
      doc.runs.push(CookieBotRun {
        id: format!("r{i}"),
        profile_id: "p1".to_string(),
        profile_name: Some("Shopping".to_string()),
        user_id: None,
        email: None,
        team_id: None,
        trigger: "manual".to_string(),
        status: if i == 2 {
          "failed".to_string()
        } else {
          "succeeded".to_string()
        },
        scheduled_for: "2026-10-20T10:00:00Z".to_string(),
        dispatch_after: None,
        started_at: None,
        ended_at: None,
        max_minutes: 45,
        chunks_total: 1,
        chunk_index: 1,
        sites_total: 1,
        sites_visited: 1,
        sites_failed: 0,
        consent_dismissed: 0,
        billed_seconds: 3600,
        outcome_code: None,
        session_id: None,
      });
    }
    let schedules = vec![CookieBotSchedule {
      profile_id: "p1".to_string(),
      profile_name: "Shopping".to_string(),
      ..empty_schedule()
    }];
    let usage = usage_for(&doc, &schedules, "2026-10");
    assert_eq!(usage.period, "2026-10");
    assert_eq!(usage.used_hours, 3.0);
    assert_eq!(usage.members.len(), 1);
    assert_eq!(usage.members[0].bot_runs, 3);
    assert_eq!(usage.members[0].bot_runs_failed, 1);
    assert_eq!(usage.profiles.len(), 1);
    assert_eq!(usage.profiles[0].bot_hours, 3.0);
  }

  fn empty_schedule() -> CookieBotSchedule {
    CookieBotSchedule {
      profile_id: String::new(),
      profile_name: String::new(),
      platform: "windows".to_string(),
      enabled: true,
      run_at_minute: 540,
      days_mask: 127,
      slots: vec![],
      timezone: "UTC".to_string(),
      preset: "balanced".to_string(),
      template_id: None,
      max_minutes: 45,
      sites: vec![],
      jitter_seconds: 0,
      sync_enabled: false,
      encrypted_sync: false,
      has_proxy: true,
      proxy_remote_reachable: true,
      touch_fingerprint: false,
      sticky_exit: false,
      profile_state_at: None,
      blocked_by: None,
      next_run_at: None,
      last_run_at: None,
      last_run_id: None,
      owner_user_id: None,
      owner_email: None,
      updated_at: None,
    }
  }

  #[test]
  fn due_slots_fire_once_and_only_within_the_window() {
    let mut doc = CookieBotDoc::default();
    let mut schedule = empty_schedule();
    schedule.profile_id = "p1".to_string();
    schedule.timezone = "UTC".to_string();
    schedule.slots = vec![crate::cookie_bot::CookieBotSlot {
      days_mask: 0b0100_0000, // Sunday
      run_at_minute: 9 * 60,
    }];
    doc.schedules.push(schedule);

    // Sunday 2026-10-25 09:10 UTC: ten minutes past the slot — due.
    let now = Utc.with_ymd_and_hms(2026, 10, 25, 9, 10, 0).unwrap();
    assert_eq!(due_slots(&doc, now).len(), 1);

    // Two hours past: missed, not queued.
    let late = Utc.with_ymd_and_hms(2026, 10, 25, 11, 0, 0).unwrap();
    assert!(due_slots(&doc, late).is_empty());

    // Same minute next week would be due again, but not the same slot twice.
    let key = {
      let slot = WeeklySlot {
        days_mask: 0b0100_0000,
        run_at_minute: 540,
      };
      weekly_fire_key("cb:p1", slot, "UTC", now)
    };
    doc.fired.push(key);
    assert!(due_slots(&doc, now).is_empty());
  }

  #[test]
  fn mark_fired_keeps_the_log_bounded() {
    let mut doc = CookieBotDoc::default();
    for i in 0..600 {
      mark_fired(&mut doc, format!("k{i}"));
    }
    assert!(doc.fired.len() <= 500);
    assert_eq!(*doc.fired.last().unwrap(), "k599");
  }
}
