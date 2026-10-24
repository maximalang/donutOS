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
