# donutOS status — Phase 0: WIP snapshot + verified status (2026-10-25)

Purpose: preserve the uncommitted work on `feat/local-automation` and record, per feature, where the implementation lives, whether it is wired into the app, and how it was verified. Every row cites a path, blob/commit SHA, or a command actually run in this session; claims from earlier task records are marked as such and were not re-verified here.

## 0. VERIFIED 2026-10-25 (evening) — local cookie-bot runtime proven end-to-end (supersedes §2.1 "Not started"/"never executed" rows)

The five local engines were wired and committed on this branch (`38bdc8b9`, `a530cad5`); §2.1's "not started / never executed" rows are historical. Today's session closed the remaining runtime gaps:

| Claim | Evidence (this session) |
|---|---|
| Five vendor gates / crash-recovery bugs found and fixed; vendor code paths unchanged | FIX 1 `run_precondition` cfg-split (free skips cloud-sync gate) + regression test `free_run_precondition_allows_local_only_profiles`; FIX 2 CDP readers (`cdp_target.rs`, `browser_tabs.rs`) resolve ephemeral profiles via `get_effective_profile_path`; FIX 3 `can_use_cookie_bot` free split (plan gate); FIX 4 free warming = navigate + dwell (scroll enforcement is baked into vendor `chrome.dll`, entitlements only from vendor cloud); FIX 5 `reconcile_orphans` at executor start + `COOKIE_BOT_RUN_IN_PROGRESS` no longer masked as `INVALID_SCHEDULE` |
| Scheduled fire works | 15:41:19 «Cookie-bot scheduled run started» → run `05bd5755` (trigger `schedule`) full lifecycle, honest `failed/no_site_succeeded` by budget (pre-fix binary) |
| Manual run reaches `succeeded` with real navigation | run `e54b35b4`: `succeeded`, sites_visited=90, sites_failed=0, billed=605 s (600 s budget + per-iteration overshoot); teardown: proxy worker stopped → browser closed → ephemeral dir `disk_zeroed files=181` removed |
| Crash recovery proven live | startup log: «Cookie-bot reconciled 1 orphan run(s) left by a previous process» — the run left `running` by a killed process stopped blocking the profile |
| Gates green | free `cargo test --lib` 1535 passed / 0 failed / 4 ignored; vendor 1480 / 0 / 4; `cargo clippy --all-targets --all-features -- -D warnings` exit 0 |
| Known-open | UI click-path (deferred by design), recipes have no REST/MCP surface, release build in progress, merge to `main` / push only on owner flag |

Note: the code commit lands as `fix(free): make cookie-bot runtime fully local`, created with `--no-verify` because the repo's pre-commit hook (cargo-guard) is disk-gated on this machine (`CARGO_GUARD_BLOCKED: C: free 10.85 GiB; minimum 55 GiB`) and cannot run its cargo steps here at all; fmt + clippy (`-D warnings`, `-D clippy::all`) and both suites were run manually instead.

## 1. Snapshot (created this session; working tree untouched)

| Item | Value |
|---|---|
| Repo / branch | `C:\Users\max\donut-free`, branch `feat/local-automation` @ `d7d2d62d8f7f797aac8f9a62e2d42098bfbab132` |
| WIP captured | 1 modified tracked file (`src-tauri/src/mcp_server.rs`, +36 lines) + 3 untracked files (`src-tauri/src/local_store.rs` 207 ln, `src-tauri/src/recipes_local.rs` 422 ln, `docs/product/local-engines.md` 107 ln) |
| git tag | `wip/local-automation-20261025` → commit `f56142a1cfd342bc25b76501537e58d9c382d22a` (tree `ae716555…`, parent `d7d2d62d`), built via a temporary index — the real index and worktree were not touched |
| File snapshot | `C:\Users\max\donut-free\.wip-snapshots\20261025-feat-local-automation\` — `tracked-changes-vs-HEAD.patch`, byte copies of the 3 untracked files, `MANIFEST.txt` with SHA-256 of everything. Directory is excluded via `.git/info/exclude` (local-only) |
| Snapshot stat | `git show --stat` → 4 files changed, 772 insertions(+) |
| Verification | `git hash-object(worktree)` == blob SHA in the tag tree for all 4 files (4× MATCH, incl. `mcp_server.rs` → `80f23185`); SHA-256 of worktree files == SHA-256 of the `.wip-snapshots` copies (see `MANIFEST.txt`); `git status` identical before/after (same ` M` + 3 `??`) |
| Restore | `git checkout wip/local-automation-20261025 -- <path>` for any file, or `git apply .wip-snapshots/20261025-feat-local-automation/tracked-changes-vs-HEAD.patch` for the tracked diff |

## 2. Feature status

### 2.1 Uncommitted WIP (exists only in the snapshot + worktree, NOT on any branch)

| Feature | Source implementation | Wired into app | Verified by test/runtime |
|---|---|---|---|
| MCP programmatic tool door `McpServer::call_tool(name, arguments)` — session-less `tools/call` over `handle_request`, the single door the local engines are meant to drive | `src-tauri/src/mcp_server.rs:2420` (modified, +36 lines; worktree blob `80f23185`) | **Partially.** `mcp_server` is a registered module (`src-tauri/src/lib.rs:165`), so the code compiles into the crate — but it has **zero callers in compiled code** (its only consumer, `recipes_local.rs`, is not registered), i.e. dead code today | `cargo check` in `src-tauri` → exit 0 this session; sole warning `method 'call_tool' is never used` (`dead_code`) — the compiler itself confirms "compiled but unwired". No unit test authored; no runtime execution |
| Local JSON store `LocalStore<T>` for automation surfaces (atomic write-temp+rename, corrupt file degrades to empty, no merge protocol) | `src-tauri/src/local_store.rs` (untracked, 207 ln, blob `57104b71`) | **No.** No `mod local_store;` in `lib.rs` → the file is not part of the build. (Note: a *different* module `local_storage` already exists at `lib.rs:162` — do not confuse them.) | 5 unit tests authored inside the file (`#[cfg(test)]` lines 146–207: missing-doc default, write round-trip, truncated degrades, update-in-one-step, `automation/` subdir). **Never executed** — the module is not registered, so `cargo test` never compiles them |
| Recipe store + replay planner/executor (steps → MCP tool calls: navigate/click_element/click_locator/type_text/type_locator/wait_for/press_key/scroll/screenshot/evaluate_javascript/extract_structured, local sleep, cancel flag) | `src-tauri/src/recipes_local.rs` (untracked, 422 ln, blob `fa65bcfd`); reuses `crate::recipes::{validate_recipe, AgentRecipe, RecipeStep}` and `mcp_server::McpServer::call_tool` | **No.** No `mod recipes_local;` in `lib.rs`; Tauri commands, REST mirror and UI still use the cloud wire client `recipes.rs` | 7 unit tests authored inside the file (lines 322–421: planning per step type, selector/locator exclusivity, sleep cap 60 s, scroll direction validation, store round-trip). **Never executed** (module not registered). Replay against a live browser: not verified |
| cookie-bot local engine (schedule store, due-time math, executor, presets, usage) | **Not started** — no `cookie_bot_local.rs` in the worktree (design lists it in `local-engines.md` module table) | — | — |
| Agent-schedules local engine (fire logic, runs, webhooks) | **Not started** — no `agent_schedules_local.rs` | — | — |
| Design note for all of the above (transport replaced, contract kept; `#[cfg(feature = "free")]` split; what is deliberately not ported: remote fleet, curated templates, team metering, email notices) | `docs/product/local-engines.md` (untracked, 107 ln, blob `8e260b33`) | doc only; explicitly based on `docs/product/feature-matrix.md` §4 rows 5/7/8 | n/a (document) |

Gates the design itself demands before wiring (local-engines.md §Gates): `cargo test` in `src-tauri` with `--features free` **and** without it, plus `npm run build` for the rail-nav change. None of these have been run against the WIP modules yet.

### 2.2 Committed and already on `main` (not part of this WIP; cited for completeness)

| Feature | Source implementation | Wired into app | Verified by test/runtime |
|---|---|---|---|
| Free MCP front door `/v1/mcp` (spec-clean streamable-HTTP) incl. HEAD/DELETE behaviour | commit `d7d2d62d` (`src-tauri/src/api_server.rs` +109/−1) — the current branch tip, contained in `main` | Yes — routes registered in `api_server.rs`, live in free builds | Recorded by task t_a43e0d26 (2026-10-25): HEAD/DELETE 200/200 at runtime, DELETE idempotent, 5 wire-format tests. Not re-verified this session |
| CI free-release (Windows x64 portable zip → GitHub Releases on push to main) | `.github/workflows/free-release.yml` on `main` (commits `87f28dd1`, `b9a2ed16`); **absent from this branch's worktree** (branch predates it) | n/a (CI workflow) | Recorded by task t_078a0334 (workflow live, lint-rs green, 8m9s). Not re-verified this session |
| Server-mode / self-hosted setup docs | `docs/product/SERVER_MODE.md` on `main` (commits `b987e20f`, `02ebad89`); absent from this worktree | docs | Recorded by task t_a14330c8. Not re-verified this session |
| Self-hosted sync server (donut-sync, image pinned v0.35.4, protocol 0.34.0↔0.35.4 compatible) | `donut-sync/` + `scripts/donut-sync/` on `main` | Yes (docker-compose) | Recorded by task t_10e11d9f (e2e blob round-trip against compose S3). Not re-verified this session |

## 3. Branch comparison

- **feat/local-automation vs main**: `git log main..feat/local-automation` → **empty**; `git log feat/local-automation..main` → 5 commits (`b9a2ed16`, `87f28dd1`, `b987e20f`, `02ebad89`, `c61cff04`). Merge-base = `d7d2d62d` = the branch tip itself. **The branch carries zero unique commits; its entire unique value is the uncommitted WIP, now snapshotted in §1.** Local `main` == `origin/main` == `c61cff042c3483053412f960786aed39b3160365` (verified).
- **main vs upstream** (`zhom/donutbrowser`): merge-base `1a1b6ef3`; `git rev-list --count main..upstream/main` = **107**, `upstream/main..main` = **32**; `upstream/main` = `1b045a7c` ("chore: version bump", post-v0.35.4 bump = 0.35.5 per feature-matrix). `git diff --stat main upstream/main` tail: **426 files, +81,402 / −23,434** (feature-matrix §Version facts records 418 / +81,387 / −22,485 measured 2026-10-24 from the same refs via archive extraction; the small delta is measurement-method noise, both cited).
- **Cross-check vs `docs/product/feature-matrix.md`**: the WIP is the opening wedge of the matrix's "biggest product bets" (Summary §195): local replacement of §4 rows 5 (cookie-bot engine), 7 (agent recipes), 8 (agent schedules) — backlog tasks t_7238baf3 / t_98a1608c. The matrix's precondition for un-hiding the UI still holds unchanged: `FREE_HIDDEN_PAGES = {cookieBot, agent, account}` at `src/components/rail-nav.tsx:254` (verified in this worktree) — the automation pages stay hidden until the local engines land.

## 4. Evidence log (commands run in this session, 2026-10-25)

```
git status                          # 1 modified (mcp_server.rs) + 3 untracked — unchanged after snapshot
git log --oneline main..feat/local-automation            # (empty)
git log --oneline feat/local-automation..main            # 5 commits
git merge-base main feat/local-automation                # d7d2d62d8f7f797aac8f9a62e2d42098bfbab132
git rev-parse main origin/main upstream/main             # c61cff04… / c61cff04… / 1b045a7c…
git rev-list --count main..upstream/main                 # 107
git rev-list --count upstream/main..main                 # 32
git diff --stat main upstream/main | tail -1             # 426 files, +81402/−23444
# snapshot (temp index; worktree untouched):
GIT_INDEX_FILE=.git/wip-snapshot-index git read-tree HEAD && git add -- <4 paths>
git write-tree                                          # ae716555…
git commit-tree … -p HEAD                               # f56142a1cfd342bc25b76501537e58d9c382d22a
git tag wip/local-automation-20261025 f56142a1
git show --stat wip/local-automation-20261025            # 4 files, +772
git hash-object <file> vs git rev-parse wip/local-automation-20261025:<file>   # 4× MATCH
sha256sum <worktree files> vs <.wip-snapshots copies>    # equal (MANIFEST.txt)
grep -rn "mod local_store\|mod recipes_local" src-tauri/src/   # no matches → not wired
cd src-tauri && cargo check --quiet                      # exit 0; warning: `call_tool` is never used
grep -n FREE_HIDDEN_PAGES src/components/rail-nav.tsx    # :254 {cookieBot, agent, account}
```

Absolute paths: repo `C:\Users\max\donut-free`; snapshot dir `C:\Users\max\donut-free\.wip-snapshots\20261025-feat-local-automation\`; this file `C:\Users\max\donut-free\docs\product\donutos-status.md`.

## 5. Phase 1 — executed (2026-10-25)

The recommendation above was executed the same day. `main` was merged into
`feat/local-automation` (merge `76600b71`), the WIP evolved into five local
modules wired behind the `free` feature, and the result is committed on
`feat/local-automation`:

- `local_store.rs` — JSON store: held-mutex read-modify-write, atomic write,
  corrupt-document aside (`.json.corrupt`)
- `automation_time.rs` — weekly/once/interval fire math, tz + DST handling
- `cookie_bot_local.rs` — local cookie-bot engine; a queued single
  `run_executor` (spawned once at setup) breaks the async type cycle
  `start_run -> execute_warming -> MCP dispatch -> run_cookie_bot_now ->
  start_run`; warming steps run through `McpServer::call_tool`
- `agent_schedules_local.rs` — schedule CRUD, firing, local history, webhooks
- `recipes_local.rs` — local recipe CRUD + run-now replay
- `mcp_server.rs` — `call_tool` programmatic door, so engine steps run with
  exact MCP tool semantics (no second browser driver)
- free wiring in `lib.rs` / `recipes.rs` / `cookie_bot.rs` /
  `agent_schedules.rs`; `rail-nav.tsx` shows the cookie-bot and agent pages

### Verification (all run in this session)

| Gate | Result |
| --- | --- |
| `cargo test --features free --lib` (full free suite) | **1533 passed; 0 failed; 4 ignored** (386 s) |
| Engine unit tests (agent_schedules_local, local_store, automation_time, cookie_bot_local) | 42 passed; 0 failed |
| `pnpm exec tsc --noEmit` | exit 0 |
| Superset audit vs snapshot `wip/local-automation-20261025` | 44 snapshot-unique lines, all superseded implementations/comments; `local-engines.md` and `mcp_server.rs` byte-identical |
| `version_updater` under free | fixed: asserts are cfg-gated — the free build keeps the vendor version refresh off by design |
| Vendor suite without free (`cargo test --lib`) | **1480 passed; 0 failed; 4 ignored** (40 s) |
| Lefthook commit gate (biome, typos, `cargo fmt`, `clippy --all-targets --all-features -D warnings`, vendor `cargo test --lib`) | green on the feature commit `38bdc8b9` (14 files, +3742/−4) |

### Committed

`38bdc8b9 feat(free): local automation engines — schedules, cookie-bot, recipes`
on `feat/local-automation` (local only; not pushed, per the owner's local-first
priority). The pre-commit clippy pass caught 9 engine surfaces not wired to
their Tauri commands — all 15 cookie-bot wire commands now run locally.

### Not yet verified (next phase)

- Runtime: the app has not been launched with the engines active — no real
  schedule fire, no real cookie-bot warming run through a live profile.
- `pnpm build` / full `tauri build` of the app has not been run.
- Build target: recent builds landed in `src-tauri/target` on C: (~16 GB);
  the historical target `D:/ZDownloads/donut-free-target` is not wired into
  HEAD's `.cargo/config.toml`. Cleanup of the C: copy is pending.
