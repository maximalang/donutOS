# Donut Browser — Feature Matrix: upstream v0.35.x vs free build

**Purpose.** Full inventory of the upstream functional surface vs our free local build. This is the product backlog source for donutOS (local replacements of vendor-cloud features: child tasks t_7238baf3, t_98a1608c).

**Method.** Source-level RE on the open upstream. Upstream read from a `git archive upstream/main` extraction (784 files) taken 2026-10-24 into the task workspace; our build read from the local checkout `C:\Users\max\donut-free` (branch `main`). No decompilation, no network calls to vendor endpoints.

**Version facts (all `seen`).**

| What | Upstream | Ours (free) |
|---|---|---|
| package.json version | **0.35.5** (`package.json` version) — i.e. upstream/main is one bump **past** the v0.35.4 release | **0.34.0** (`package.json` version) |
| CHANGELOG latest release | v0.35.4, 2026-10-24 (`CHANGELOG.md` heading) | — |
| merge-base | `1a1b6ef3` = upstream v0.34.0 (`git merge-base main upstream/main`; its `package.json` says 0.34.0) | same base |
| Distance | 107 commits ahead of base; diff vs our main: **418 files, +81,387 / −22,485** (`git diff --stat main upstream/main`) | — |

> ⚠️ The task brief said upstream = v0.35.4. The fetched `upstream/main` is the post-release bump **0.35.5** (release notes still end at v0.35.4). Merge/compare work should pin against this commit, not the 0.35.4 tag.

**Evidence convention.** `seen (file:symbol)` = read in this task's extraction/checkout. `inferred` = reasoned, not directly read. Line numbers are from the extracted upstream main (v0.35.5) / our main checkout and will drift; symbols are the stable reference. `unknown` = not established.

**Statuses (free build).**
- **WORKS** — fully local, functional in the free build.
- **PARTIAL** — present, degraded or incomplete in free.
- **HIDDEN** — implemented, UI hidden by `FREE_HIDDEN_PAGES`.
- **MISSING** — not in our tree (upstream-only).
- **VENDOR-CLOUD** — logic lives on `api.donutbrowser.com`; local code is wire/client only; unusable without the vendor cloud.

---

## 1. Tauri commands (invoke handlers)

Counts (`seen`: `lib.rs` `generate_handler![..]` lists, sorted, deduped): **upstream 383, ours 366** (`#[tauri::command]` attrs: 333 vs 316).

**Upstream-only since v0.34.0 (17 commands + 2 renames) — all MISSING in ours** (`seen`: sorted diff of handler lists):

| Command | What it is (evidence) | Free status | Port plan |
|---|---|---|---|
| `entity_trash::trash_entity` / `restore_trashed_entity` / `list_trashed_entities` / `purge_trashed_entity` | Recoverable delete for proxies/VPNs/groups/extensions/templates (`entity_trash.rs` module doc: "Recoverable delete for what profiles point at") | MISSING | Local feature, port as-is — no cloud dependency seen in module |
| `network_capture::start_network_capture` / `stop_network_capture` / `list_network_requests` / `get_network_request` | In-memory capture of what a profile's pages request/receive (`network_capture.rs` doc) | MISSING | Local; port together with the `traffic` MCP toolset + 4 MCP tools (§3) |
| `log_viewer::read_app_log` | Tail of app log for in-app viewer (`log_viewer.rs` doc) | MISSING | Local; trivial port |
| `ephemeral_dirs::get_ephemeral_storage_status` | RAM-backed ephemeral profile storage status (v0.35.0 "Windows RAM profiles") | MISSING | Local; port |
| `autostart::take_reopen_profiles` | Launch-at-login + reopen profiles (`autostart.rs` doc; ours still has `legacy_autostart.rs`) | MISSING | Local; port (module is new, refactors legacy) |
| `browser_runner::get_profile_debug_endpoint` | Per-profile debug endpoint (CDP) | MISSING | Local; port |
| `cancel_profile_import` | Cancel running profile import | MISSING | Local; port |
| `remote_session::start_profile_remote_session` | Start remote VM session from the desktop (`remote_session.rs` doc) | MISSING | VENDOR-CLOUD by design (cloud leases VMs); do not port — replace with self-hosted runner later |
| `recipes::run_agent_recipe_now` | Trigger cloud recipe now (`recipes.rs`: cloud client) | MISSING | Needs local recipe engine (see §4 agents) |
| `refresh_mcp_agents`, `show_inline_tip`, `release_inline_tip`, `note_tip_feature_used` | Connected-agent list refresh; coach-mark tips system (v0.35.0) | MISSING | `refresh_mcp_agents`: local once agents are local; tips: port as-is |
| `sync::account_server::get_sync_source` | Renamed from ours `sync::account_server::get_account_sync_server` (sync server choice) | rename | Re-apply rename in merge |
| `sync::health::get_sync_health` | Sync health probe (new `sync/health.rs`) | MISSING | Local/self-host; port |
| `tray::update_tray_menu` | Tray icon/menu (running, recent, stop all) — ours has an unqualified `update_tray_menu` handler instead (`tray.rs` new module upstream) | rename/refactor | Port module, drop our inline version |

**Ours-only (6):** `is_free_build`, `free_build_enabled` (`lib.rs:78,721` — `cfg!(feature = "free")`), `get_table_sorting_settings` / `save_table_sorting_settings` (local-only convenience), `sync::account_server::get_account_sync_server` (pre-rename name). Keep all in the merge.

---

## 2. Local REST API (`/v1/*`, axum + OpenAPI)

Counts (`seen`: path literals in `api_server.rs`, sorted unique): **upstream 80, ours 82 — ours is a strict superset of upstream paths.**

| Surface | Routes (evidence `api_server.rs` path literals) | Free status | Notes / port plan |
|---|---|---|---|
| Profiles CRUD, run/kill/open-url, batch run/stop, bulk-edit, import (+detect), distribute-proxies, fingerprint, storage-state, cookies/import | `/v1/profiles*` | WORKS | Local |
| Agent REST (per-profile) | `/v1/profiles/{id}/agent/{click,extract,perceive,pick,resolve-locator,type}` | WORKS | Local CDP-driven; same engine as MCP tools (comment at `api_server.rs` ~5451: two front doors share `mcp_server` ops) |
| Remote-run + cloud-sync flags | `/v1/profiles/{id}/run-remote`, `/v1/profiles/{id}/cloud-sync` | VENDOR-CLOUD | `run-remote` leases vendor VM (`remote_session.rs`); keep routes, back later with self-hosted runner |
| Proxies | `/v1/proxies`, `/{id}`, `/rotate`, `/import` | WORKS (local proxies) | Vendor *cloud* proxy is separate (§4) |
| VPNs | `/v1/vpns`, `/{id}`, `/import`, `/export` | WORKS | Local (xray/wireguard workers) |
| Extensions / groups / tags / templates / statuses | `/v1/extensions*`, `/v1/extension-groups*`, `/v1/groups*`, `/v1/tags`, `/v1/templates*`, `/v1/statuses*` | WORKS | Local |
| Browser downloads | `/v1/browsers/{browser}/versions…`, `/v1/browsers/download` | PARTIAL | Version manifest fetches `donutbrowser.com/wayfern.json` (`api_client.rs:287`); free build has cfg gates in `downloaded_browsers_registry.rs` (6 sites) — mirror the manifest locally for full offline |
| Cookie-bot | `/v1/cookie-bot/{conflicts,presets,runs,runs/{id},schedules,schedules/{profile_id},usage}` | VENDOR-CLOUD (routes exist in our build) | Thin REST mirror of cloud API (`cookie_bot.rs` is "the wire only" — module doc); needs local engine (t_7238baf3) |
| Remote sessions | `/v1/remote-sessions`, `/{id}`, `/{id}/cdp`; `/v1/remote-hours` | VENDOR-CLOUD | Cloud leases + `wss` CDP relay |
| MCP doors | upstream: only `/v1/e2e/mcp` (`#[cfg(feature = "e2e")]`, "Built into E2E builds only, never published" — comment above `e2e_mcp_message`); **ours adds `/v1/mcp`** (`free_mcp_message`, `api_server.rs:1070,1153-1154`) and **`/v1/health`** (`get_health`) | WORKS (ours-only) | Our `/v1/mcp` is the local MCP front door the upstream desktop does not ship — keep, it is the product's local-control surface |
| OpenAPI | `/openapi.json`, `/v1/openapi.json` | WORKS | Hand-maintained `ApiDoc`; must be updated in merge |

---

## 3. MCP tool engine

Counts (`seen`: unique `name:` literals inside `McpServer::get_tools()`): **upstream 117 tools, ours 110 (strict subset).**

- **7 upstream-only tools** (all MISSING in ours): `file_upload`, `get_diagnostics`, `get_network_request`, `list_network_requests`, `start_network_capture`, `stop_network_capture`, `select_option`. The 5 capture/diag tools belong to the new `traffic` toolset + `agent_uploads` module; `select_option` is a form-fill action.
- Toolsets (`seen` `mcp_toolsets.rs` TOOLSETS + `src/lib/mcp.ts` OPTIONAL_TOOLSETS): `core, organize, network, fingerprint, extensions, cookies, sync, remote, recipes`; upstream adds **`traffic`** (ours lacks it). `recipes` is the only **CLOUD toolset** (`mcp_toolsets.rs:180` `CLOUD_TOOLSETS = ["recipes"]`).
- Live connector advertises 107 tools for our build (Hermes `donut` connector, observed) vs 110 in source — gap of 3 **unknown** (likely toolset selection on the connecting client; not investigated here).
- Front doors: upstream serves MCP **only over the remote bridge** (`mcp_remote.rs` doc: app dials OUT to `wss://api.donutbrowser.com/api/mcp-bridge`; agents present `dmk_` keys to `https://api.donutbrowser.com/api/mcp`, `lib.rs:932` comment) plus the e2e-only door. Ours adds the local spec-clean streamable-HTTP door `/v1/mcp` (`free_mcp_message` — comment: body-wrapped e2e envelope is upstream-only shape; ours is "spec-clean").

| Feature | Free status | Port plan |
|---|---|---|
| 110 local MCP tools via `/v1/mcp` | WORKS | — |
| 7 missing tools (§ above) | MISSING | Port with `agent_uploads.rs`, `network_capture.rs`, `diagnostics.rs`; add `traffic` toolset |
| `recipes` cloud toolset | VENDOR-CLOUD | Local recipe engine (t_7238baf3): storage+replay move local; recorder (`recorder.rs`) is local already |
| Remote bridge / `dmk_` keys | not used (VENDOR-CLOUD) | Superseded by our local door; do not port |
| MCP client integrations page (22 AI clients) | PARTIAL | `mcp_integrations.rs` writes server entries pointing at `CLOUD_API_URL /api/mcp` with bearer key — must be repointed to our local `/v1/mcp` + local token |

---

## 4. Vendor cloud surface (`api.donutbrowser.com` and friends)

Base: `CLOUD_API_URL = "https://api.donutbrowser.com"` (`cloud_auth.rs:16`). All rows VENDOR-CLOUD unless noted. Evidence = `format!`/const literals read in the named file.

| # | Endpoint group | Literal (evidence) | What it does | Local/server port plan |
|---|---|---|---|---|
| 1 | Device auth | `/api/auth/device-code/challenge`, `/api/auth/device-code/exchange`, `/api/auth/token/refresh`, `/api/auth/logout`, `/api/auth/me` (`cloud_auth.rs`) | Account login (device-code flow), token lifecycle, profile | **Not portable** — no vendor account in free; our auth = local `config/automation-token.txt` (`settings_manager.rs:540-581`, `#[cfg(feature="free")]`) |
| 2 | Sync tokens | `/api/auth/sync-token`, `/api/auth/sync-profile-usage` (`cloud_auth.rs`) | Token for sync namespace at `sync.donutbrowser.com` | Replaced by self-hosted sync accounts (§5) |
| 3 | MCP keys | `/api/mcp/keys`, `/api/mcp/status` (`cloud_auth.rs`); agent door `/api/mcp` (`mcp_integrations.rs:39`; `src/lib/mcp.ts:2`); device bridge `/api/mcp-bridge` wss (`mcp_remote.rs:70`) | `dmk_` keys for remote agent control | Local: our `/v1/mcp` + portable token already covers it |
| 4 | Vendor proxy | `/api/proxy/config`, `/api/proxy/locations/countries`, `/api/proxy/usage` (`cloud_auth.rs`) | Cloud-managed proxy pool ("create_cloud_location_proxy") | Drop (user-supplied local proxies WORKS) or self-host pool manager |
| 5 | Cookie-bot engine | `{CLOUD}/api/cookie-bot` + `/conflicts /presets /runs /runs/{id} /schedules /schedules/{id} /usage /user-templates /user-templates/{id}` (`cookie_bot.rs` format! strings) | Overnight cookie warming on remote hosts; "schedule and everything the bot actually does are owned by the cloud API" (`cookie_bot.rs` doc) | **Core of t_7238baf3**: local schedule store + local execution on our own engines; presets → local JSON |
| 6 | Cookie-bot exit check | `cookie_bot_exit.rs` (new in v0.35.3) | Local pre-check that enrolled profiles' proxy/VPN still answers | Port as-is (local) |
| 7 | Agent recipes | `/api/agent/recipes`, `/api/agent/replays` (`recipes.rs`) | "Storage, validation rules and replay live in the cloud" (`recipes.rs` doc) | Local recipe store + executor (t_7238baf3) |
| 8 | Agent schedules | `/runs`, `/run-now`, `/test-notice` (`agent_schedules.rs`) | Cloud calendar for recipes ("The cloud stores the schedules, decides when they fire, and checks the plan" — module doc) | Local scheduler (cron-like) on our engines |
| 9 | Remote sessions | `/api/remote-sessions` (`remote_session.rs`), wss `/api/remote-sessions/cdp` (`remote_session.rs`, `cdp_target.rs`) | Cloud-leased remote VM browsers; `remote_handoff.rs` gates stale local copies; `remote_exit.rs` checks exit-node reachability | Self-host a session runner (our `donut-browser-daemon` is the natural base — exists upstream too, `seen` both trees) or drop |
| 10 | Remote hours | `/v1/remote-hours` REST + cloud metering (`get_remote_hours`) | Pooled remote-host budget | Only relevant with #9 |
| 11 | App updates | `https://api.donutbrowser.com/api/app-releases` (`app_auto_updater.rs:121` `DONUT_RELEASES_URL`); repos `repo.donutbrowser.com/{rpm,deb}` | Release manifest + packages; v0.35.2 added hourly check | Self-host the manifest (our own update server) — free build already cfg-gates updater paths (`auto_updater.rs` 8 gates) |
| 12 | Wayfern browser manifest | `https://donutbrowser.com/wayfern.json` (`api_client.rs:287`) + `/api/auth/wayfern-start` token (`cloud_auth.rs`) | Fingerprint-browser build list + licensed token | Mirror `wayfern.json` locally; token check disabled in free (gates seen) |
| 13 | Feedback | `/api/feedback` (`feedback.rs`) | In-app feedback + redacted logs (`diagnostics.rs`) | Drop or write to local file |
| 14 | Licensing / plans | `commercial_license.rs` (4 pub fns, no //! doc read); plan matrix `src/lib/entitlements.ts` (solo/pro/team/enterprise capabilities: browserAutomation, cookieBot, remoteControl, scheduledAutomation, cloudBackup, …) | Paid-plan entitlements | Replaced by free/unlimited local policy — keep entitlements plumbing, default everything on |
| 15 | Team features | `team_members.rs`, `team_roles.rs` (launcher/admin/owner roles), `team_activity.rs` (local share of log; "The cloud records sign-ins, membership changes and profile launches itself" — module doc), `team_lock.rs` | Team plan: group-scoped access, roles, activity log; group **ids** live in the cloud, the sync server "holds each member to them" (`team_members.rs` doc) | **Mixed cloud+sync** (`inferred`): access control is sync-server-backed and should port to self-hosted accounts; the cloud activity log share is dropped |
| 16 | Account web | `donutbrowser.com/auth/link`, `/account` (`src` frontend refs) | Login/link deep links | Remove links in free UI |

---

## 5. Sync

| Component | Evidence | Free status | Notes |
|---|---|---|---|
| Engine (client) | `src-tauri/src/sync/` = `account_server, client, config_entities, conflicts, encryption, engine, health, manifest, pending_deletes, preflight, restore_points, scheduler, subscription, types` + `synchronizer.rs` (3022 lines) | WORKS | Local, E2E-encrypted (encryption + vault-key tests seen as files) |
| Self-hosted server | `donut-sync/` (NestJS: `src/sync/sync.controller.ts`, `sync.service.ts`, `auth/donut-accounts.ts`, docker-compose, presign e2e tests) — **shipped by upstream since v0.34.0** ("own sync server", CHANGELOG) | WORKS | Our fork drift vs upstream: 7 files, +35/−42 (`git diff upstream/main main -- donut-sync`) — mostly `user-context.interface.ts` + `sync.service.ts`; re-merge is cheap |
| Vendor sync host | `sync.donutbrowser.com` (seen in `donut-sync/docker-compose.yml`, sync config default) | VENDOR-CLOUD | Not used in free; our deploy = `scripts/donut-sync/` (`.env` + `docker-compose.yml` + `dist`, ours-only deploy wrapper) |
| Upstream changes since base | rename `get_account_sync_server`→`get_sync_source`; new `sync/health.rs` + `get_sync_health` cmd; v0.35.3 "pull remote work" fix; cross-OS sync already in 0.34.0 | — | Wire-format compatibility detail is task t_10e11d9f's scope; no protocol-version constant found in `sync/types.rs`/`manifest.rs` (`unknown`) |

---

## 6. UI / rail-nav / pages

| Feature | Evidence | Free status | Port plan |
|---|---|---|---|
| Rail pages | `AppPage` union identical in both trees: profiles, proxies, extensions, groups, cookieBot, agent, vpns, settings, integrations, account, import, shortcuts, trash (12) | — | — |
| Cloud page hiding | ours: `FREE_HIDDEN_PAGES = {cookieBot, agent, account}` (`rail-nav.tsx:254`), driven by `src/lib/free-build.ts` (`detectFreeBuild` → invoke `is_free_build`; sync `isFreeBuildSync`) | HIDDEN | Un-hide per module as local replacements land (t_7238baf3 step 4) |
| Upstream rail refactor | `PAGE_ICONS` + `PAGE_ORDER` exports, motion (`useMotionAllowed`), dropdown menu, `FOCUS_RING` (upstream `rail-nav.tsx`) | MISSING | Comes free with the merge; keep our FREE_HIDDEN filter |
| Trash page | "audit UI and trash" (CHANGELOG v0.35.0) + `entity_trash` cmds | MISSING | Port |
| Shortcuts page | `AppPage` "shortcuts" exists in both | WORKS (assumed local; not runtime-verified — `inferred`) | — |
| Import page | `AppPage` "import"; upstream added `cancel_profile_import` | PARTIAL | Port cancel command |
| Coach-mark tips | `show_inline_tip`, `release_inline_tip`, `note_tip_feature_used` cmds; `coach-mark` lib | MISSING | Local; port or strip for lean UI |
| Agent console UI | `agent-console-*.tsx`, `agent-activity.tsx`, `agent-recipes*`, `agent-schedules.tsx`, `agent-profiles.tsx`, `agent-conversation.tsx` (components exist in our tree too — UI present, hidden) | HIDDEN | Reopen when local agents land |
| Cookie-bot UI | `cookie-bot-page.tsx` + enrol/activity/overview | HIDDEN | Same |
| Account page | `account-page.tsx`, `account-sync-server.tsx`, `device-code-verify-dialog.tsx` | HIDDEN | Sync-server picker should return (self-hosted); vendor account stays hidden |
| Integrations page | `mcp_integrations.rs` (22 AI clients) + Integrations page | PARTIAL | Repoint entries to local `/v1/mcp` (§3) |

---

## 7. Upstream modules absent from our tree (v0.35.x additions)

All `seen` in `ls` diff of `src-tauri/src` (upstream-only files); doc quotes from module headers.

| Module | Purpose (doc) | Status | Cloud dep |
|---|---|---|---|
| `entity_trash.rs` | Recoverable delete for referenced entities | MISSING | none seen |
| `network_capture.rs` | In-memory network capture for agents | MISSING | none |
| `agent_uploads.rs` | Files from `file_upload` tool, kept until browser stops | MISSING | none |
| `diagnostics.rs` | Redacted install facts + logs for bug reports | MISSING | feeds `/api/feedback` |
| `log_viewer.rs` | App-log tail for in-app viewer | MISSING | none |
| `tray.rs` | Tray icon + menu (running/recent/stop-all/quit) | MISSING (ours: inline handler) | none |
| `autostart.rs` | Launch-at-login + reopen profiles (replaces `legacy_autostart.rs`) | MISSING | none |
| `cookie_bot_exit.rs` | Local reachability pre-check for cookie-bot enrollees | MISSING | local check, cloud feature |
| `dialer.rs` | Outbound TCP dial visibility of proxy worker | MISSING | none |
| `file_stamp.rs` | Detect if a parsed file changed on disk | MISSING | none |
| `mcp_server_browser_tests.rs` | Test-only | MISSING | — |

Also new-but-local (in `routes`/handlers): ephemeral RAM storage status (Windows RAM profiles, v0.35.0), anchored sync clicks, hourly update check (v0.35.2 — needs #11 §4), "link and create dialogs" (v0.35.1), headless profile folder fix + wayfern token renewal (v0.35.4).

---

## 8. Free-build deltas (ours-only; must survive the merge — list for t_98a1608c)

1. `src/lib/free-build.ts` (28 lines) — free detection cache.
2. `rail-nav.tsx` — `FREE_HIDDEN_PAGES` filter + `detectFreeBuild/isFreeBuildSync` wiring.
3. Cargo feature `free = []` (`src-tauri/Cargo.toml:240`) + **43 `cfg(feature="free")` gate sites in 8 files** (counts by file: `cloud_auth.rs` 14, `auto_updater.rs` 8, `downloaded_browsers_registry.rs` 6, `app_auto_updater.rs` 4, `settings_manager.rs` 4, `api_server.rs` 3, `lib.rs` 2, `version_updater.rs` 2).
4. `/v1/mcp` `free_mcp_message` + `/v1/health` (`api_server.rs:1070,1153-1154,1676`).
5. Portable automation token `config/automation-token.txt` next to the executable (`settings_manager.rs:540-581`).
6. `scripts/donut-sync/` deploy wrapper (.env + compose + dist).
7. `get/save_table_sorting_settings` commands.
8. `README-FREE.md`, `DONUT-FREE-BRIEF.md` docs.

---

## Summary

Inventory rows above: **~70 feature rows** across 8 sections. By status in the free build (counting §1 delta rows 19, §2 route groups 11, §3 rows 5, §4 rows 16, §5 rows 4, §6 rows 11, §7 rows 11 — de-duplicated where sections overlap):

| Status | Count | Reading |
|---|---|---|
| WORKS (local today) | ~30 | Profiles/proxies/VPN/extensions/groups/tags/templates/statuses core, local REST + agent REST, 110 MCP tools via `/v1/mcp`, sync engine + self-hosted server, browser downloads (partial manifest dep) |
| PARTIAL | ~6 | Browser version manifest, updates (gated), integrations page (cloud-pointed), import (no cancel), MCP (110/117, no `traffic`), toolset gap 107 vs 110 unknown |
| HIDDEN (UI off, code present) | ~4 | cookieBot, agent, account pages (+ their components) |
| MISSING (upstream-only, worth porting) | ~19 | entity_trash, network_capture+traffic, agent_uploads, diagnostics, log_viewer, tray, autostart, dialer, file_stamp, sync health, tips, cancel import, RAM-ephemeral status, 7 MCP tools, rail/page refactor |
| VENDOR-CLOUD (needs local replacement by design) | ~10 | cookie-bot engine, recipes+schedules, remote sessions/hours, remote MCP bridge+dmk keys, vendor proxy, app-releases updates, wayfern token, feedback, account/device auth, licensing |

**Biggest product bets for local replacement** (feeds t_7238baf3): (1) cookie-bot engine — cloud owns schedule+behavior, wire already mapped; (2) recipes + scheduled automation — cloud storage/replay/calendar; (3) remote sessions — needs a self-hosted runner (candidate base: `donut-browser-daemon`, present in both trees); (4) integrations repoint to local `/v1/mcp`. Everything else in MISSING is plain local porting work with no cloud dependency observed.

*Compiled 2026-10-24 from `git archive upstream/main` @ v0.35.5 (post-0.35.4 bump) and local `main` @ 0.34.0 (merge-base `1a1b6ef3`).*
