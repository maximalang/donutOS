# Manual windows: no CDP, no automation channel

Owner decision of 2026-11-03 (card `t_9c964bb0`, option **C**): a profile
opened by a person from the app (`LaunchKind::Interactive`) runs **without any
DevTools/CDP involvement**, and automation treats it as occupied.

## Why

Vendor engine 155.0.8059.79 arms a session-enforcement timer on the **first
CDP attach** and exits the browser within ~2–5 minutes (120-second "free"
entitlements, validated in the background; full forensics in the card's
workspace: `FORENSICS-killguard.md`). Measured on the production install:

| experiment | lifetime |
|---|---|
| app launch (attach at setup), idle or driven | ≤ 305 s, always |
| bare engine, no DevTools port | ≥ 420 s (probe-stopped) |
| bare engine, **port open, zero attaches** (phase G) | ≥ 480 s (probe-stopped) |
| persistent DevTools websocket hold | ≤ 265 s |

So an Interactive launch neither opens `--remote-debugging-port` nor attaches
a client: `devtools_switches(None)` adds no socket at all. Identity travels
only through the launch identity file (`--wayfern-identity-file`), the
152+-era path; the engine's own stderr verdict (via the launch log tap) is
the post-launch check, and a silent tap is logged as "not verified", not
treated as a failure. Launch URLs ride the command line instead of
`Page.navigate`.

A profile whose device **cannot** ride the launch document — a stored
fingerprint (any engine), or an identity on a pre-152 engine or without a
timezone — is refused before the window opens with
`MANUAL_LAUNCH_NEEDS_AUTOMATION`, because committing it would require the
very attach a manual window must not do.

## What automation sees

`WayfernManager::manual_session(profile)` is the single occupancy predicate
(kind `Interactive`, process alive). Every automation surface answers a
manual window with `manual session active: …` and touches nothing:

- MCP `run_profile` / `batch_run_profiles`: error / per-profile "manual
  session active" line; the window is never restarted.
- MCP `kill_profile` / `batch_stop_profiles`: error / per-profile line; the
  window is never closed by automation ("kill_profile-guard").
- MCP driving tools (`navigate`, `open_tab`, `list_tabs`, clicks, extraction,
  …): `cdp_target::resolve` and `browser_tabs::port_of` report the busy
  session instead of "not running", without spending the launch-retry budget.
- REST `POST /profiles/{id}/run` and the batch run answer `409` / a
  per-profile "manual session active" error with the same message, instead of
  launching automation over the window (occupancy semantics only; no contract
  shape changed).
- REST `POST /profiles/stop` (single and batch) answers `409` / a
  per-profile error with the same message
  (occupancy semantics only; no contract shape changed).

The person's own controls are unaffected: the app UI's Stop button and the
open-URL paths still work for them.

## Known limitations (deliberate)

- **`open_url` into a manual window does not work.** There is no automation
  channel to open a tab through; the UI reports `MANUAL_SESSION_ACTIVE`
  (localized) with the reason instead of working around it. Open the URL in
  the window itself.
- **Stopping a manual window from the app UI is a process termination**, not
  a graceful `Browser.close` (there is no socket to send it over). The
  launch already passes `--hide-crash-restore-bubble`, and the profile's
  data is written by the engine as it goes, so nothing is lost that a crash
  would not already risk.
- A manual window cannot be driven at all until it is stopped from the app
  UI; agents are expected to wait (the message says so).
- An adopted browser found by process scan after an app restart counts as
  manual exactly when its command line has no DevTools port — i.e. windows
  this build opened. Older builds' manual windows (with a port) remain
  drivable, as before.

## What did not change

- Automation launches (`kind=automation`): identical behaviour, port,
  attach, CDP setup and page navigation as before — including the engine-155
  session cap, which is a vendor property this change does not touch
  (engine-patch options were explicitly declined).
- The engine patch ladder and `docs/engine-patches.md`.
