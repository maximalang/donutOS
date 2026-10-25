# Local automation engines (free build): cookie-bot, recipes, schedules

Upstream v0.35.x keeps the *behaviour* of these three surfaces in the vendor
cloud (`{CLOUD}/api/cookie-bot/*`, `/api/agent/recipes`, `/api/agent/schedules`);
the desktop ships wire-only clients. The free build has no vendor account, so
the wire is dead code there. This note records what replaces it locally and
what is deliberately not ported. Basis: `docs/product/feature-matrix.md` §4.

## Decision

Replace the **transport**, keep the **contract**. Every public function and
wire type in `cookie_bot.rs`, `recipes.rs` and `agent_schedules.rs` keeps its
name and shape; under `#[cfg(feature = "free")]` they are served by local
engines instead of `cloud_api::call`. Tauri commands, the REST mirror under
`/v1/cookie-bot/*`, the OpenAPI document and all UI components stay unchanged.
Non-free builds keep the vendor path — the split is compile-time, so it is
reversible by building without the feature.

New modules:

| module | role |
| --- | --- |
| `local_store.rs` | tiny atomic JSON store (write-temp + rename), one file per surface |
| `cookie_bot_local.rs` | schedule store, due-time math, scheduler tick, executor, runs, presets, usage |
| `recipes_local.rs` | recipe store (same validation as the wire client) + replay engine |
| `agent_schedules_local.rs` | schedule store, fire logic, per-schedule runs, webhook notices |

Storage lives under `app_dirs::data_subdir()/automation/`: `cookie-bot.json`,
`recipes.json`, `agent-schedules.json`. One machine, one store; no merge
protocol (see *not ported*).

### Execution model — one engine, no drift

Both executors (cookie-bot warming, recipe replay) drive the browser through
the **existing MCP tool surface**: `mcp_server::call_tool(name, arguments)` —
a `pub(crate)` wrapper over `handle_request` with a caller that owns no
session. Steps therefore run with exactly the semantics of `navigate`,
`click_element`/`click_locator`, `type_text`/`type_locator`, `wait_for`,
`press_key`, `scroll`, `screenshot`, `extract_structured` — same Wayfern/Vellum
paths, same rate limiter, same human typing. The local build implements no
second browser driver.

Recipe step → tool mapping: `navigate{url}`, `click{selector|locator}`,
`type{selector|locator,text}`, `waitFor{selector,timeoutMs}`, `pressKey{key}`,
`scroll{direction,amount}`, `screenshot`, `sleep{ms}` (local sleep, no tool),
`extract{...}` (`extract_structured`; Wayfern profiles only, a failure marks
the run `partial`, it does not abort it).

Cookie-bot executor: launch the profile (`launch_browser_profile`, advisory
gate), then for each site in the schedule: `navigate` → dwell (`scroll` +
random pause) → next site; cycle sites until `max_minutes` elapses; kill the
browser; record the run (`sites_visited`/`sites_failed`/`billed_seconds`,
status `succeeded`/`partial`/`failed`). Jitter is applied before start, as
upstream does. Cancel support: an in-memory `AtomicBool` registry keyed by run
id; `cancel_run` flips it and the executor observes it between steps.

Scheduler: one tokio task spawned at setup (free builds only), tick 30 s.
Due-time math per cadence: weekly (`days_mask` + `run_at_minute` in
`timezone`, chrono-tz), once (`run_once_at`), interval (`interval_minutes`
anchored at creation). Fire keys (`schedule id` + local date/interval index)
are persisted so a restart never double-fires the same slot.

Presets: local, fixed — `light` (20 min), `balanced` (45, default),
`deep` (90); ids match the UI's existing `t()` keys. Limits published
locally (min 10 / max 480 min, 1–25 sites, 8 slots, 80-char template names).

Usage / remote hours: no remote fleet exists, so there is nothing to ration.
`team_usage` and `remote_hours_quota` answer from the local run history with
`scope: "local"`, `seats: 1`, `granted_hours: 8760` (a year) and a synthetic
`local` member. Honest numbers, no quota theatre.

Conflicts: always empty. Upstream conflicts are *teammates enrolling the same
profile on the vendor account*; a local store has no teammates.

Notices: `notifyEmail` cannot work without an SMTP story — `test-notice`
answers `{"email":"off", ...}` and a failing run logs instead of mailing.
Webhooks ARE ported: the engine POSTs the same JSON body upstream documents,
signed with the schedule's `webhookSecret` (`X-Donut-Signature:
sha256=<HMAC-SHA256>`), secret rotation included. That is pure automation and
belongs local.

## Not ported, and why

| vendor feature | local answer |
| --- | --- |
| runs on *remote hosts* (windows/macos/linux fleet) | runs on **this machine**, visibly or headless per profile; `platform` recorded as the local OS. The remote fleet is rented vendor hardware — nothing to reimplement |
| curated browsing templates (`template_id` with server-owned site pools) | **not ported**: the pools are vendor content we do not have and will not fabricate. User site lists and user templates are fully local. `templates: []` in the preset list |
| cloud-side conflict detection between teammates | empty list (single-owner store) |
| team usage metering / billing | local run-history report, no billing |
| email failure notices | off (no local SMTP); webhook parity kept |
| schedule `target: "fleet"` | refused at save with `AGENT_SCHEDULE_INVALID` — only `desktop` exists locally |
| cloud replay store (`/api/agent/replays`) | local run records per schedule (`list_agent_schedule_runs`); the recipe itself is the replay source |

## UI

`FREE_HIDDEN_PAGES` in `rail-nav.tsx` shrinks from `{cookieBot, agent,
account}` to `{account}`: the cookie-bot and agent pages come back in free
builds because every control on them now answers locally. The account page
stays hidden — it exists to manage the vendor subscription we do not have;
self-hosted sync configuration is surfaced elsewhere (sync server picker).

## Gates

Per module, before wiring: `cargo test` (unit) in `src-tauri` with
`--features free` and without it (both feature worlds must compile), plus the
existing suite. Frontend: `npm run build` in the app root for the rail-nav
change.
