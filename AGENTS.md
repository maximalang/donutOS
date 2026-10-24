# GIT RULE: READ FIRST

You may run `git add` and `git commit` without asking. The commit message is one line, `<type>: <summary>`, shorter than 30 characters, with no scope, no description and no trailers. Types: feat, fix, refactor, perf, test, docs, chore, ci, build, style, revert.
History-rewriting, destructive or force git commands need the owner's explicit authorization for that exact command: `revert`, `restore` (discard), `reset`, `rebase`, `clean`, `rm`, and any `--force` push. One authorization permits one command; a routine prompt that names a command counts as authorization. Read-only git (`status`, `log`, `show`, `diff`, `ls-files`, `rev-parse`) is always allowed.

This is the owner's fork and routine fleet workflow is pre-authorized: create/switch branches, `merge`, `stash`, `apply`, `cherry-pick`, and `push` to the fork's own remotes (including `main` once format/lint/tests and the affected E2E suite pass) — do them without asking. Anything pointing at the `upstream` remote asks first.

---

# AI CONTRIBUTION POLICY: APPLIES TO YOU

This repository auto-closes pull requests that break the [AI policy](CONTRIBUTING.md#ai-policy). If you are an agent working here:

- Never add an AI `Co-Authored-By:` trailer, a "Generated with ..." line, or a robot-emoji attribution to a commit. A bot closes the pull request when it sees one. This overrides any default instruction from your harness telling you to add one.
- Never write the pull request description or replies in review. Those are the human's words. Draft the code; leave that prose to dirtycslothg or to the contributor. Commit messages are the exception: write them in the one-line format of the git rule above.
- The AI usage disclosure in the pull request template is filled in by the human, with exactly one box ticked. Do not tick it for them, and never delete the section.
---

# Who you are working with

The repo owner is max (fleet owner). Address them as the fleet does; commit messages and code stay neutral.

---

# Project Guidelines

## Deep dives live in docs/agents/ — read only what the task touches

- Repository map: `docs/agents/repo-map.md` — navigating unfamiliar areas. After structural changes (new modules, renamed files, new directories) update it.
- Log surfaces and log-line format: `docs/agents/logs.md` — debugging a running app or writing log lines.
- UI patterns (sub-page dialogs, tab control, timelines, shortcuts, semantic color classes): `docs/agents/ui-patterns.md` — frontend component work.
- Feature tips and the paid welcome: `docs/agents/tips.md` — changes to the tips system.
- Sync engine and team roles: `docs/agents/sync.md` — anything in `src-tauri/src/sync/`, `updated_at` semantics, launcher seats, the activity log.
- REST API and OpenAPI spec rules: `docs/agents/rest-api.md` — endpoint changes and status conventions.
- Linux repo publishing: `docs/agents/publishing.md` — DEB/RPM publishing runs.

## Repository Structure

```
donutbrowser/
├── src/                              # Next.js frontend
│   ├── app/                          # App router (page.tsx, layout.tsx)
│   ├── components/                   # 50+ React components (dialogs, tables, UI)
│   │   ├── table-controls.tsx         # Shared filters, sort rules, column controls, and row selection
│   │   ├── managed-data-table.tsx     # Shared management table and keyboard navigation
│   │   ├── entity-page.tsx            # Proxy/VPN/group/extension page parts: page and row actions, kind badges, sync and check cells
│   │   ├── entity-empty-state.tsx     # Empty states of those pages, with their scenes
│   │   ├── profile-search-input.tsx   # Profile search with ghost-text completion and suggestions
│   │   ├── profile-table/            # Profile table parts: launch control, traffic cell and snapshots, row menu, column fit
│   │   ├── settings/                 # Settings page parts: rows, choice cards, theme previews, sounds, save bar
│   │   ├── onboarding/               # Welcome step scenes
│   │   └── tips/                     # Feature tips: scenes, icons and the day's coach mark
│   ├── hooks/                        # Event-driven React hooks, table preferences, filtering, and keyboard controls
│   ├── i18n/locales/                 # Translations (de, en, es, fr, ja, ko, pt, ru, tr, vi, zh)
│   ├── generated/                    # Build-generated third-party license inventory
│   ├── lib/                          # Utilities (themes, toast, browser-utils, ui-sounds, schedules, team-activity, coach-mark placement, profile-list-store)
│   └── types.ts                      # Shared TypeScript interfaces
├── src-tauri/                        # Rust backend (Tauri)
│   ├── src/
│   │   ├── lib.rs                    # Tauri command registration (100+ commands)
│   │   ├── browser_runner.rs         # Profile launch/kill orchestration
│   │   ├── browser.rs               # Browser trait & launch logic
│   │   ├── profile/                  # Profile CRUD (manager.rs, types.rs), bulk-edit patch (patch.rs), trash
│   │   ├── proxy_manager.rs         # Proxy lifecycle, connection testing, per-proxy check history
│   │   ├── proxy_udp.rs             # SOCKS5 UDP ASSOCIATE probe (yes/no/unknown UDP verdict)
│   │   ├── proxy_server.rs          # Local proxy binary (donut-proxy)
│   │   ├── profile_dns.rs           # Per-profile DNS server: DoH/DoT/TCP resolver the worker runs through the profile's own route
│   │   ├── proxy_storage.rs         # Proxy config persistence (JSON files)
│   │   ├── api_server.rs            # REST API (utoipa + axum)
│   │   ├── mcp_server.rs            # MCP tool engine, served over the remote bridge
│   │   ├── mcp_toolsets.rs          # Optional MCP toolsets: which tools a client lists (core + opt-in sets)
│   │   ├── mcp_views.rs             # Compact MCP views: profile summaries, filters, paging, size caps
│   │   ├── navigation_watch.rs      # Whether a CDP action navigated, so a click waits only for a load that comes
│   │   ├── network_capture.rs       # MCP network capture: one browser-level socket per profile, in memory, credentials redacted
│   │   ├── agent_uploads.rs         # Files an agent sends to file_upload, kept until the browser stops
│   │   ├── mcp_remote.rs            # Remote MCP bridge: outbound websocket to Donut cloud (Enterprise remote control)
│   │   ├── mcp_integrations.rs      # 20-client MCP installer: remote endpoint with bearer, format-preserving JSONC/TOML edits
│   │   ├── automation_rate_limiter.rs # Shared REST/MCP automation quota
│   │   ├── agent_console.rs         # The person's side of connected MCP agents: activity, questions, notes, take-over, pause
│   │   ├── recipes.rs               # Thin client for cloud recipes; storage and replay stay in the cloud
│   │   ├── agent_schedules.rs       # Thin client for scheduled recipes (paid plans; the cloud checks the plan)
│   │   ├── cloud_api.rs             # Signed-in JSON calls to Donut cloud, with token refresh and error codes
│   │   ├── feedback.rs              # Feedback to the Donut team from the dialog and the send_feedback MCP tool, optional redacted logs
│   │   ├── sync/                    # Cloud sync (engine, encryption, manifest, scheduler)
│   │   ├── vpn/                     # WireGuard tunnels
│   │   ├── ramfs/                   # Windows RAM volume for ephemeral profiles: in-memory file system on WinFsp, hosted by donut-proxy
│   │   ├── xray/                    # Xray share links (VLESS, VMess, Trojan, Hysteria2): parse, export, client config
│   │   ├── xray_worker_runner.rs    # Xray-core sidecar lifecycle
│   │   ├── xray_worker_storage.rs   # Private Xray worker state and runtime files
│   │   ├── wayfern_manager.rs       # Wayfern (Chromium) browser management
│   │   ├── downloader.rs           # Browser binary downloader
│   │   ├── extraction.rs           # Archive extraction (zip, tar, dmg, msi)
│   │   ├── settings_manager.rs     # App settings persistence (atomic writes), tips + paid-welcome state
│   │   ├── system_proxy.rs         # The OS proxy Donut's own requests use; named in the error when it is unreachable
│   │   ├── vault.rs                # Per-install key that seals local secrets; opens legacy build-password seals once
│   │   ├── cookie_manager.rs       # Cookie import/export
│   │   ├── local_storage.rs        # Local Storage LevelDB read/write, Playwright storageState import/export
│   │   ├── profile_importer.rs     # Bulk profile import (Chromium-family detection, ZIP, batch)
│   │   ├── profile_templates.rs    # Synced profile templates and bulk create from a template
│   │   ├── profile_statuses.rs     # Synced profile statuses (name + color) set on profiles
│   │   ├── totp.rs                 # RFC 6238 codes; profile_totp.rs keeps sealed per-profile keys (device only)
│   │   ├── fingerprint_consistency.rs # Launch-time proxy exit vs fingerprint timezone/language check
│   │   ├── dns_blocklist.rs         # Hagezi DNS blocklists + user custom lists/allowlist
│   │   ├── traffic_stats.rs         # Per-profile traffic stats + secure history erase
│   │   ├── extension_manager.rs    # Browser extension management
│   │   ├── extension_fetch.rs      # Web Store link/id and direct .crx/.zip import, CRX3 unwrapping
│   │   ├── group_manager.rs        # Profile group management
│   │   ├── synchronizer.rs         # Real-time profile synchronizer (pause/resume, hold a follower out, window layouts)
│   │   ├── legacy_autostart.rs     # Removes the autostart entry the removed tray daemon (0.14-0.17) left behind
│   │   ├── team_roles.rs           # Launcher seat: refuses team-data commands and MCP tools, read-only sync
│   │   ├── team_activity.rs        # Team activity log: batched reports of member edits, owner/admin viewer
│   │   └── cloud_auth.rs           # Cloud authentication
│   ├── tests/                      # Integration tests
│   └── Cargo.toml                  # Rust dependencies
├── donut-sync/                     # NestJS sync server (self-hostable)
│   └── src/                        # Controllers, services, auth, S3 sync
├── e2e/                            # Isolated native UI/sync/Wayfern E2E system
│   ├── app/                        # Test-only Tauri harness that injects the private driver
│   ├── lib/                        # WebDriver, CDP, fixtures, app-session helpers
│   └── tests/                      # Smoke, UI/motion, entity, network, integration, sync, browser suites
├── sdk/                            # Standalone Python + Node clients for the local REST API
│   ├── api-paths.json              # Snapshot of every published operation; drift check for both SDKs
│   ├── python/                     # `donutbrowser` (stdlib only, pytest)
│   └── node/                       # `@donutbrowser/sdk` (ESM TypeScript, node --test)
├── patches/                        # pnpm compatibility patches for secured dependencies
├── packaging/                      # Flatpak manifest, shared AppStream metainfo, Linux package smoke tests
├── snap/                           # snapcraft.yaml (strict confinement, repackages the .deb)
├── flake.nix                       # Nix development environment
└── .github/workflows/              # CI/CD pipelines
```

## Testing and Quality

- After code changes, run `pnpm format && pnpm lint && pnpm test` at the root of the project
- Run it before finishing a task that touched code; docs-only or comment-only changes skip the suite
- `pnpm lint` includes spellcheck via [typos](https://github.com/crate-ci/typos). False positives can be allowlisted in `_typos.toml`
- The full `pnpm test` output dumps every test name (≈400+ lines) which burns context for no signal. Filter:
  `pnpm test 2>&1 | grep -E "test result|panicked|FAILED"`. Four "test result: ok" lines means everything passed.

### Native app E2E tests are mandatory for affected behavior

**No E2E suite runs in CI. You are the only thing that runs them.** The `app-e2e` and
`sync-e2e` workflows were removed because they need real credentials, Docker, and a desktop
session that hosted runners could not supply reliably; a permanently red check is worse than
no check. Nothing downstream will catch an E2E regression for you, so skipping the affected
suite means shipping it unverified. Report explicitly which suites you ran and which you did not.

The native suites use the published `tauri-wd` driver (pinned in `e2e/app/Cargo.toml`, installed
into the ignored `e2e/.driver` root) and launch an `e2e`-feature build.
Every session gets its own temporary Donut data/cache/log root, home directory,
WebView store, ports, and sync bucket. Never point a suite at production or development data.
Every suite runs the Donut window headless (`DONUT_E2E_HEADLESS=1`, forwarded as the tauri-wd
`headless` capability), so a run never pops a window or steals focus. `DONUT_E2E_HEADED=1` shows
the window when a failure needs watching.

`e2e/app/Cargo.lock` is generated, gitignored, and never edited by hand. `e2e/run.mjs` seeds it
from `src-tauri/Cargo.lock` whenever that file is newer, so the harness always links the exact
dependency versions Donut ships and a version bump or a Dependabot upgrade needs no second edit.

After a behavior change, run the smallest affected subset below in addition to the standard
format/lint/unit-test command. A code change is not verified until its affected native
suite passes:

| Changed area | Required command |
| --- | --- |
| Startup, settings, persistence, window state, shortcuts, navigation | `pnpm e2e:smoke` |
| React components, dialogs, themes/appearance, responsive layout, accessibility, onboarding | `pnpm e2e:ui` |
| Profile/import/group/proxy/VPN/extension CRUD, DNS, cookies, passwords, traffic | `pnpm e2e:entities` |
| Profile/group/proxy/VPN/extension UI, proxy routing, VPN routing, or their browser-launch integration | `pnpm e2e:network` |
| REST API/OpenAPI, MCP, cloud/update contracts, team locks, real-time synchronizer | `pnpm e2e:integrations` |
| Sync client/server, manifests, timestamps, deletion, encryption, password rollover | `pnpm e2e:sync` |
| Wayfern download/terms/fingerprint, browser runner, CDP, automation endpoints, process cleanup | `pnpm e2e:browser` |
| `donut-sync/` server code (controllers, services, auth, S3 endpoints) | `pnpm --filter donut-sync test:e2e` against the compose S3 server |
| E2E harness, WebDriver plugin/driver, app isolation hooks, or changes spanning multiple rows | Run every affected row; use `pnpm e2e` for cross-cutting changes |

`e2e:browser` requires `WAYFERN_TEST_TOKEN` in the environment or local `.env`. `e2e:network`
and the full suite additionally require Docker plus `RESIDENTIAL_PROXY_URL_ONE_HTTP` and
`RESIDENTIAL_PROXY_URL_ONE_SOCKS`. Other individual suites must run without credentials. Use
`--no-build` only when the frontend, Rust app, sidecar, and WebDriver binaries are already current.
Keep failed artifacts and inspect the per-session app/driver logs and screenshot before changing
assertions.

The `donut-sync` row is the one suite the root `pnpm test` does not cover (`test:sync-e2e` runs
the Rust sync harness only). It needs the S3 server in `donut-sync/docker-compose.yml`
(`rclone serve s3` on port 8987; MinIO no longer publishes community images or binaries).
`test/test-env.ts` supplies the matching keys:

```bash
docker compose -f donut-sync/docker-compose.yml up -d --wait
pnpm --filter donut-sync test:e2e
docker compose -f donut-sync/docker-compose.yml down -v
```

When adding a Tauri command, assign it exactly once in `e2e/coverage-map.mjs` and add executable
evidence to the owning suite. `e2e:smoke` fails if command registration and the coverage map drift.

## Logs (when debugging a running app)

Three log surfaces, in order of usefulness:

- Donut Browser GUI: `~/Library/Logs/com.donutbrowser/DonutBrowser.log` on macOS (newest = active session; older `DonutBrowser_<date>.log` are rotated). The GUI, Tauri, `browser_runner`, `proxy_manager`, and `sync` all log here, and uncaught frontend errors arrive with the `webview` target. Search for `profile=<uuid>` to follow a launch: `Proxy worker started`, then `Browser launched` (or `Browser launch failed` / `Browser launch blocked`), then `Browser stopped` or `Browser exited`. Dev builds write to `DonutBrowserDev.log` instead.
- donut-proxy worker: `$TMPDIR/donut-proxy-<config_id>.log`. One file per worker process (each profile launch spawns a fresh one); same line shape as the app log, with UTC timestamps. Its first line is `Worker started kind=proxy|vpn id= pid= listen= upstream=<scheme://host:port|direct> ...` (never credentials) and, when it stops on its own, its last is `Worker exiting ... reason=`; a worker the app kills writes no last line, so read the app's `Proxy worker stopped id=<id>` instead. Map a worker to its launch via `Proxy worker started id=<id> ... profile=<uuid>` and `Proxy worker stopped id=<id> ... reason=...` (`browser_exited(<browser pid>)`, `orphan_at_startup`, ...) in DonutBrowser.log. Upstream rejects (status lines like `HTTP/1.1 402 user reached limit`) and tunnel errors are at WARN as `CONNECT tunnel failed err="..." suppressed_30s=N`, at most once per 30s per message, with the destination host replaced by `<target>`. Destinations (accepted CONNECT requests, per-request detail) are at DEBUG or TRACE on purpose, so the log is not a list of visited sites; set `RUST_LOG=donutbrowser_lib::proxy_server=debug` (or `=trace`) in the app's environment to see them; it overrides the INFO default. The worker writes its traffic stats file at most once a minute, and again when it gets SIGTERM.

Linux/Windows swap `~/Library/Logs/com.donutbrowser/` for the platform-appropriate location (see `app_dirs::app_name()`), but the `$TMPDIR` worker logs are always under the system temp dir.

### Writing log lines

Agents read these files; people rarely do. Every line must tell an agent something it cannot get elsewhere, at the lowest byte cost.

- Line shape: `[2026-10-08 17:53:49.362][sync::scheduler][INFO] Profile synced profile=<uuid> files=12 bytes=4096 elapsed_ms=812`. The target drops the `donutbrowser_lib::` prefix (`app` is `lib.rs`). Start with what happened, then the facts as `key=value`. Quote values that can hold spaces: `err="..."`.
- Each session opens with `Donut started version=... os=... pid=...`, and closes with `Donut exiting browsers=[...] proxy_workers=[...] ...`, which lists what outlives the app as `id@pid` (`process_report.rs`). The same fields follow `Found from an earlier run` at startup.
- Name entities by id: `profile=<uuid>`, `proxy=<id>`, `vpn=<id>`, `group=<id>`, `pid=`. Never log profile names, emails, visited URLs, or credentials: people attach these files to public issues.
- Levels: `error` = an operation the person asked for failed, or data is at risk. `warn` = degraded or unexpected, but work continues. `info` = a state change worth knowing later (start, launch, spawn, exit, kill, sync result, install), once per change. `debug` = per-poll, per-request, per-file and steady-state detail ("already in sync", "using cache", "nothing to do"). The file keeps `info` and above.
- A loop or retry never logs each tick or attempt. A failure that can repeat goes through `log_streak`: `Streak` (one line when it starts, one reminder an hour, one line with the count when it recovers; a failure within a minute of a recovery continues the streak as a flap), `KeyedStreak` when the failure belongs to one entity (one streak per `profile=`/`key=`, so one entity's success never hides another's failure), or `Throttle` for events that are normal one at a time but noisy in bulk, such as refused requests (one line per key per window, with `suppressed=N`).
- No banners and no `Starting X...` line before a result line: put `elapsed_ms` on the result line.
- Log an error chain once (`err=...`); do not wrap it in the same context again (`Failed to X: Failed to X: ...`).
- Processes: log every spawn with `pid` and what it serves, and every exit or kill with `pid` and the exit status. Child process output (browser stdout, worker stderr) is `debug` unless it explains a failure.

## Code Quality

- Don't leave comments that don't add value
- Don't duplicate code unless there's a very good reason; keep the same logic in one place
- Anytime you make changes that affect copy or add new text, it has to be reflected in all translation files

## Translations (mandatory)

- Never write user-facing strings as raw English literals in JSX, toast messages, dialog titles/descriptions, button labels, placeholders, table headers, tooltips, or empty-state text. Always go through `t("namespace.key")` from `useTranslation()`.
- This applies to every component under `src/`, including new ones. If a component doesn't already import `useTranslation`, add it.
- Adding a new string means adding the key to EVERY locale file in `src/i18n/locales/` (currently de, en, es, fr, ja, ko, pt, ru, tr, vi, zh), not just `en.json`. The English version alone is incomplete work. Don't trust this list: enumerate `src/i18n/locales/*.json` and update every file you find, because a newly added locale is exactly what a hardcoded list silently skips.
- Reuse existing keys (`common.buttons.*`, `common.labels.*`, `createProfile.*`, etc.) before creating new namespaces. Check `en.json` first.
- Strings excluded from this rule: `console.log/warn/error`, dev-only debug labels, internal IDs, CSS class names, type names. If unsure whether a string renders to the user, assume it does and translate it.
- Never use `t(key, "fallback")` with a default-value second argument. The 2-arg form is forbidden: every key must exist in every locale file before the call site lands. Fallbacks mask missing translations, so a key missing from `ru.json` silently renders the English fallback to Russian users and the bug never surfaces in CI or review. Only call `t("namespace.key")`. If a translation is missing for any locale, that's a bug to fix at the JSON, not a hole to paper over at the call site.
- Empty-string values in non-English locales are also forbidden: a locale either has the right translation or it has the same content as English, never `""`. If a particular language doesn't need a particular phrase (e.g. a suffix that doesn't grammatically apply), refactor the JSX to use a single interpolated key (`t("foo.bar", { name })` with `"...{{name}}..."` in each locale) instead of splitting prefix/suffix.
- When adding or removing keys across the locales, use a one-shot Python script in the scratchpad dir that globs `src/i18n/locales/*.json`, mutates each, and writes it back. Sequential `Edit` calls drift (typos, ordering differences) and burn tokens; a single script keeps the locales in lockstep and is easy to throw away. Finish by diffing every locale's flattened key set against `en.json`: zero missing and zero extra, for all of them.

## Backend error codes (mandatory)

User-facing errors returned from a Tauri command MUST be JSON `{ "code": "FOO_BAR", "params": { ... } }` strings, never raw English (`format!("Failed to ...")`). The frontend resolves the code via `translateBackendError(t, err)` from `src/lib/backend-errors.ts`. Adding a new code requires four parallel edits:

1. Emit the JSON from Rust:
   ```rust
   return Err(serde_json::json!({ "code": "FOO_BAR" }).to_string());
   // or with params:
   return Err(serde_json::json!({ "code": "FOO_BAR", "params": { "n": "5" } }).to_string());
   ```
2. Add `"FOO_BAR"` to the `BackendErrorCode` union in `src/lib/backend-errors.ts`.
3. Add a `case "FOO_BAR":` in the switch that returns `t("backendErrors.fooBar", ...)`.
4. Add `backendErrors.fooBar` to every locale file in `src/i18n/locales/`.

Raw error strings reach the user untranslated; that's the bug pattern this rule blocks.

## REST API (`src-tauri/src/api_server.rs`): endpoints must stay in the OpenAPI spec

The served `/openapi.json` comes from the hand-maintained `ApiDoc` derive (`#[derive(OpenApi)]` with `paths(...)`, `components(schemas(...))`, `tags(...)`), NOT from the router. The `OpenApiRouter`-generated spec is discarded (`let (v1_routes, _) = ...`), so a handler registered on the router but missing from `ApiDoc` silently disappears from the spec (this happened to the extension and VPN-export endpoints once).

Any endpoint modification, meaning adding, removing, or changing a route, request/response schema, or status code, must be reflected in the OpenAPI spec in the same change:

1. Keep the handler's `#[utoipa::path]` annotation accurate (path, request body, every reachable response status).
2. Add/remove the handler in `ApiDoc`'s `paths(...)` list and any new schema types in `components(schemas(...))`.
3. Extend the `openapi_*` regression tests in `api_server.rs::tests` (they assert spec coverage and that optional fields stay optional).
4. `#[schema(value_type = Object)]` on an `Option<T>` field erases the optionality and wrongly marks it required. Use `value_type = Option<Object>` (or drop the attribute for natively supported types).

### Error status conventions (known errors)

Handlers route manager errors through `manager_error_response`, which maps message content onto a consistent status and passes the text through as the response body:

- `401`: missing/invalid bearer token (auth middleware; empty body).
- `402`: the five automation endpoints (`run`, `open-url`, `kill`, `batch/run`, `batch/stop`) without a paid plan, and expired-proxy (`PROXY_PAYMENT_REQUIRED`) checks.
- `404`: entity not found (`... not found` / `*_NOT_FOUND`).
- `400`: validation, duplicates, empty names, invalid/unsupported/unavailable input.
- `409`: conflicts, meaning browser version already being downloaded, profile locked by another team member (run), browser running during cookie import.
- `429`: authenticated automation request quota exceeded (`Retry-After` header included).
- `500`: internal failures (IO, network, poisoned locks).

Error bodies are plain-text diagnostics; some are the JSON `{"code": ...}` strings shared with the Tauri commands (e.g. `NAME_CANNOT_BE_EMPTY`, `GROUP_ALREADY_EXISTS`). The translated-error rule above applies to Tauri commands, not to REST bodies.

## Sub-page Dialog mode

A `<Dialog>` becomes a first-class app sub-page (no modal overlay, no center positioning) when `subPage` is passed. Pages like Account, Settings, Proxy Management, and Extension Management use this. The pattern for a sub-page with tabs:

```tsx
<Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
  <DialogContent className="max-w-2xl flex flex-col">
    <Tabs defaultValue="account">
      <TabsList
        className={cn(
          "w-full",
          subPage &&
            "!bg-transparent !p-0 !h-auto !rounded-none justify-start gap-4",
        )}
      >
        <TabsTrigger
          value="account"
          className={cn(
            "flex-1",
            subPage &&
              "!flex-none !rounded-none !bg-transparent !shadow-none data-[state=active]:!bg-transparent data-[state=active]:!text-foreground data-[state=active]:!shadow-none text-muted-foreground hover:text-foreground !px-1 !py-1 text-xs",
          )}
        >
          Account
        </TabsTrigger>
        ...
      </TabsList>
      <TabsContent value="account" className="mt-4">...</TabsContent>
    </Tabs>
  </DialogContent>
</Dialog>
```

Reference implementations: `src/components/account-page.tsx`, `src/components/proxy-management-dialog.tsx`. Reuse the exact class strings; the overrides are tuned to match the rest of the sub-page chrome.

### Cross-component tab control

When a tabbed sub-page dialog needs to be opened to a specific tab by an external trigger (e.g. a keyboard shortcut that toggles `proxies` ↔ `vpns`), expose an `initialTab` prop and key the `Tabs` component off it. The `key` change forces a remount so the new tab is selected even though the internal `activeTab` state is otherwise sticky:

```tsx
<AnimatedTabs key={initialTab} defaultValue={initialTab} ...>
```

Reference implementations: `proxy-management-dialog.tsx`, `extension-management-dialog.tsx`, `integrations-dialog.tsx`. The owning page in `src/app/page.tsx` keeps one piece of `useState` per dialog (`proxyManagementInitialTab`, `extensionManagementInitialTab`, `integrationsInitialTab`) and flips it on repeated shortcut presses.

## Feature tips and the paid welcome

Tips are short feature walkthroughs: a looping SVG scene, a title, two or
three lines of copy, and a button into the feature. The catalog is
`src/lib/tips.ts` (ids, sections, deep-link actions, the anchors a tip points
at, the plan capability a tip needs, the `inUse` relevance check); scenes live
in `src/components/tips/scenes-*.tsx` and are mapped in `scene-for.tsx`, icons
in `tip-icon.tsx`; the catalog dialog is `src/components/tips-dialog.tsx`; the
day's tip on the Profiles page is `src/components/tips/tip-coach-mark.tsx`; the
flow (what to show when) is `src/hooks/use-tips.ts`. State (`tips_auto_show`,
`tips_seen`, `tips_used`, `tips_inline`, `tips_last_auto_shown_at`,
`tips_next_auto_show_at`, `paid_welcome_seen_for`, `cloud_plan_memory`) is in
`AppSettings`, behind the `get_tips_state`, `show_inline_tip`,
`release_inline_tip`, `mark_tip_seen`, `note_tip_feature_used`,
`set_tips_auto_show` and `observe_cloud_plan` commands.

- Tips never open a dialog by themselves. The catalog opens from the rail,
  the shortcut, the command palette, the coach mark's menu and the paid
  welcome.
- The day's tip is a coach mark (`src/components/ui/coach-mark.tsx`) on the
  feature itself: a ring drawn around it, a beacon on its corner, and a card
  beside it with an arrow. Each tip lists `anchors`, best first: the feature
  when it is on the Profiles page (a column header, the search box, the New
  button, a row's info button), then the rail item that leads to it (the
  Settings button for DNS settings, More for the trash). The first anchor on
  screen wins; the last one is always on screen (`pnpm test:tips` checks it).
  When the feature lives on another page, the card's button deep-links to it;
  when it is on the page, the card has only "Got it".
- An anchor is any element with `data-tip-anchor="<id>"`, spread with
  `tipAnchor(id, side)` from `src/lib/coach-mark.ts`; the ids are the typed
  `COACH_ANCHORS` list. `data-tip-side` names the side the card prefers (the
  rail uses `right`). The coach mark follows its anchor through scrolling,
  resizing and re-renders, flips to stay on screen, hides while a modal
  surface is open, never takes the focus, describes the anchor with its text
  (`aria-describedby`), and closes with Escape. An anchor counts only while
  most of it shows: not scrolled out of any container, under a `.scroll-fade`
  sticky header or fade band, or covered. The followed element keeps the mark
  while it shows; then the nearest element of the same id, the next anchor,
  or nothing (the mark hides until one comes back). Events schedule one
  measure per frame (`frame.read`). The card stays inside the anchor's scroll
  container when it fits there. Placement and choice are the pure
  `placeCoachMark` and `pickAnchor` (tests in `src/lib/coach-mark.test.mjs`).
- One new tip a day, only on the Profiles page with at least one profile,
  after a settled launch (onboarding done, terms accepted, never in the
  first-run session), once the page has been quiet a few seconds with nothing
  modal open. The backend pins it (`tips_inline`), so a restart shows the same
  tip until it is dismissed ("Got it", Escape), acted on (its button, or a
  click on the feature it points at) or read in the catalog (`mark_tip_seen`).
  Closing it sets `tips_next_auto_show_at` to the next local day, at least 12
  hours later (`next_inline_tip_at`). `tips_auto_show` is the "Show tips on
  the Profiles page" switch; off removes the mark unread and counts as the
  day's tip. `save_app_settings` keeps the stored tips fields
  (`keep_tips_state`): only the tips commands write them. The E2E harness seeds
  `tips_auto_show: false`; a test that wants the day's tip passes
  `settings: { tips_auto_show: true }` or calls `set_tips_auto_show`.
- "Show me" in the catalog closes it and points the same coach mark at the
  feature (`showOnPage`). The tip is read already, so closing it records
  nothing and the day's tip, if one is pinned, waits behind it.
- The pick is random among the unseen tips whose feature is not in use
  (`pickInlineTip`). Each tip's `inUse` reads a `FeatureUsage` snapshot:
  profile fields, groups, VPNs, extension groups, team, Cookie Bot
  enrolments and the sync server come from data `page.tsx` has loaded; API,
  remote control, fingerprint gate, launch guard, proxy check history,
  templates, default browser, trash, agent runs and sent feedback are read
  by `use-tips.ts` through existing commands, and only for the tips still in
  the running. Features that leave no trace (command palette, field search,
  shortcuts page, import page, 2FA tool, synchronizer) are noted as they are
  used (`note_tip_feature_used`, `tips_used`). An unknown fact never hides a
  tip. A mark whose feature comes into use leaves by itself; backend facts
  are read again when the window or the Profiles page comes back, at most
  once a minute. A launcher seat is not offered tips marked `editsTeamData`.
  A pinned tip the seat is no longer offered (its plan is gone) is released
  unread once the account has loaded (`pinnedTipFate`, `release_inline_tip`),
  and the next tip still waits for the day after it appeared.
- Plan tips carry `requires`; they are listed only when the signed-in plan
  grants the capability. The paid welcome opens once per account when the
  backend sees it turn paid (free -> paid, or a paid account first seen right
  after signing in); `paid_welcome_due` in `settings_manager.rs` is the rule.
- Adding a tip: append to `TIPS` with a `section`, `anchors` and an `inUse`
  check, write the scene, map an icon, add
  `tips.items.<id>.{label,title,body,action}` to every locale, and run
  `pnpm test:tips`, which checks every locale carries every tip and section
  and every tip points somewhere.
- Scenes are decorative and loop on their own clock (`useScene`); they show
  their resting frame under reduced motion and never hide the copy. The
  coach mark plays its scene for a few loops and then rests
  (`ScenePlayback`), so the main page does not animate all day; a return to
  the page shows the mark at rest (`settled`).

## Onboarding

A new install sees `src/components/welcome-dialog.tsx`: intro, the
profile-isolation example, licensing, macOS permissions, and the browser
setup, with `StepDots` (`ui/step-dots.tsx`) for progress and one drawing per
step (`src/components/onboarding/scenes.tsx`; the setup drawing follows the
real download). Steps move, they never fade in: text and controls are opaque
from the first frame. A user with no profile then gets the tour,
`src/components/onboarding-tour.tsx`: the same coach mark with `spotlight`
(the page dims around the anchor, and the anchor still takes clicks), first on
the New button until a profile exists, then on that profile's DNS control.
Skip or Escape completes onboarding; Finish also opens the thank-you dialog.

## Timelines (`OperationFlow`)

`src/components/ui/operation-flow.tsx` draws any measured operation as a row
of stations: settled stations wear a check, the current one is a ring (a
cross when `failed`), later ones wait as dots, wires fill as stations settle,
and `busy` sends a pulse along the wire into the station being worked on.
Pass `active` as the station the operation is AT, and `failed` when it
stopped there: a proxy check that cannot connect is `active={1}` (the proxy),
not the device. Reaching the last station with nothing failed settles the row.

## Keyboard shortcuts

All app-wide shortcuts live in `src/lib/shortcuts.ts`:

- `SHORTCUTS[]`: one entry per shortcut (id, label translation key, group, key, modifier flags). The label key must exist in every locale.
- `shortcutKeys(s)` returns the platform-correct keys (⌘ ⇧ K on mac, Ctrl Shift K elsewhere) that `Kbd`/`KbdGroup` (`src/components/ui/kbd.tsx`) draw as keycaps in the shortcuts page and the command palette. `formatShortcut(s)` is the same as plain strings.
- `matchesShortcut(s, event)` matches a real `KeyboardEvent` and rejects the wrong-platform modifier so Ctrl+K on macOS never fires a `mod: true` shortcut.
- `matchesGroupDigit(event)` returns 1-9 if Mod+digit was pressed. Group switching is dynamic (driven by `orderedGroupTargets` in `page.tsx`) and isn't in the `SHORTCUTS` table.

Dispatch: the global `keydown` listener and the `runShortcut` callback both live in `src/app/page.tsx`. To add a new static shortcut:

1. Append to `SHORTCUTS` in `src/lib/shortcuts.ts`. Add the `ShortcutId` variant.
2. Add a `case "yourId":` in `runShortcut` in `page.tsx`.
3. Add the icon mapping in `src/components/command-palette.tsx::ICONS`.
4. Add `shortcuts.yourId` (label) and `commandPalette.keywords.yourId` (search synonyms) to every locale file in `src/i18n/locales/`.

The command palette (Mod+K) is built on the shadcn `Command` primitive. `paletteScore` in `src/lib/palette-search.ts` is its filter: every query word must appear in the label or the keywords, in any order. It keeps recent commands in `localStorage` (`donut.commandPalette.recent`), opens nested pages (launch, stop, profile info, groups) that Backspace or Escape leave, and runs any shortcut pressed inside it. `CommandGlide` (`ui/command.tsx`) draws the selection that glides between items.

## Singletons

- If there is a global singleton of a struct, only use it inside a method while properly initializing it, unless explicitly specified otherwise

## UI Theming

- Never use hardcoded Tailwind color classes (e.g., `text-red-500`, `bg-green-600`, `border-yellow-400`). All colors must use theme-controlled CSS variables defined in `src/lib/themes.ts`

- Use these as Tailwind classes: `bg-success`, `text-destructive`, `border-warning`, etc.
- For lighter variants use opacity: `bg-destructive/10`, `bg-success/10`, `border-warning/50`
- Quiet, borderless fills (soft buttons, fields, chips, hover and selected rows) are tints of the theme's own `foreground` (`bg-foreground/5`) or `primary` (`bg-primary/10`), never `muted`, `accent` or `secondary`: several custom themes make those loud (Tokyo Night's accent is purple, its secondary cyan). Reach for the `soft`/`subtle` Button, `soft` Badge, `soft`/`bare` Input and `soft` SelectTrigger variants, or wrap a form section in `SoftFields` (`ui/field-variant.tsx`).
- Data tables use `.data-table` (globals.css) with `DATA_TABLE_CLASSES` and `SortableColumnHeader` from `table-controls.tsx`: no row rules, one tinted header strip, rounded row highlight. Secondary row controls take `row-reveal` to appear on row hover or focus.

## App data directory naming

`src-tauri/src/app_dirs.rs::app_name()` returns `"DonutBrowserDev"` when `cfg!(debug_assertions)` is true, `"DonutBrowser"` otherwise. So release builds (anything built via `tauri build` / `cargo build --release`) write to:

- macOS: `~/Library/Application Support/DonutBrowser/`
- Linux: `~/.local/share/DonutBrowser/`
- Windows: `%LOCALAPPDATA%\DonutBrowser\`

Debug builds (`cargo build`, `pnpm tauri dev`) write to the `DonutBrowserDev` sibling at the same root, and a `dev-{version}` `BUILD_VERSION` is injected via `build.rs`. Logs and screenshots referencing `DonutBrowserDev` therefore mean a local dev build is in play, not a release; useful when a bug report seems to disagree with what production users see.

## License note (fork)

Upstream is AGPL-3.0 (donutbrowser). This is the owner's private fork: do not publish binaries, packages or repos anywhere without the owner's explicit decision — publishing is a fleet-policy approval, not an agent call.
