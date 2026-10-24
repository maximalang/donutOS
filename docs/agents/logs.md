# Logs: surfaces and line format

> Moved from AGENTS.md (deep dive; read when the task touches this area).

## Logs (when debugging a running app)

Three log surfaces, in order of usefulness:

- Donut Browser GUI: `~/Library/Logs/com.donutbrowser/DonutBrowser.log` on macOS (newest = active session; older `DonutBrowser_<date>.log` are rotated). The GUI, Tauri, `browser_runner`, `proxy_manager`, and `sync` all log here, and uncaught frontend errors arrive with the `webview` target. Search for `profile=<uuid>` to follow a launch: `Proxy worker started`, then `Browser launched` (or `Browser launch failed` / `Browser launch blocked`), then `Browser stopped` or `Browser exited`. Dev builds write to `DonutBrowserDev.log` instead.
- donut-proxy worker: `$TMPDIR/donut-proxy-<config_id>.log`. One file per worker process (each profile launch spawns a fresh one); same line shape as the app log, with UTC timestamps. Its first line is `Worker started kind=proxy|vpn id= pid= listen= upstream=<scheme://host:port|direct> ...` (never credentials) and, when it stops on its own, its last is `Worker exiting ... reason=`; a worker the app kills writes no last line, so read the app's `Proxy worker stopped id=<id>` instead. Map a worker to its launch via `Proxy worker started id=<id> ... profile=<uuid>` and `Proxy worker stopped id=<id> ... reason=...` (`browser_exited(<browser pid>)`, `orphan_at_startup`, ...) in DonutBrowser.log. Upstream rejects (status lines like `HTTP/1.1 402 user reached limit`) and tunnel errors are at WARN as `CONNECT tunnel failed err="..." suppressed_30s=N`, at most once per 30s per message, with the destination host replaced by `<target>`. Destinations (accepted CONNECT requests, per-request detail) are at DEBUG or TRACE on purpose, so the log is not a list of visited sites; set `RUST_LOG=donutbrowser_lib::proxy_server=debug` (or `=trace`) in the app's environment to see them; it overrides the INFO default.

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
