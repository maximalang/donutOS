# Sync engine and team roles

> Moved from AGENTS.md (deep dive; read when the task touches this area).

## Sync (cloud / self-hosted)

Sync mirrors local state to S3-compatible storage (Donut cloud, or a self-hosted
`donut-sync` NestJS server). Two distinct mechanisms live in `src-tauri/src/sync/`:

- Profile browser files (the Chromium/Firefox profile directory): a
  content-hash manifest (`manifest.rs` `generate_manifest`/`compute_diff`) does a
  per-file hash+size diff, so only changed files transfer. `sync_profile` in
  `engine.rs`.
- Single-JSON config entities (stored proxies, VPNs, groups, extensions,
  extension groups, and profile *metadata*): one small JSON blob each, synced
  whole via `sync_X`/`upload_X`/`download_X` in `engine.rs`.

### Conflict resolution: one rule everywhere, `updated_at` last-write-wins

Every config entity carries `updated_at: Option<u64>` (unix seconds;
`extension_manager` uses a non-Optional `u64`). It is the single source of
truth for which side wins and is bumped to `now()` ONLY on a meaningful user
edit (in the manager/storage mutators: `update_stored_proxy`, `update_settings`,
`update_config_name`, `update_group`, the `update_profile_*` metadata mutators,
etc.), NEVER by sync bookkeeping. Use `crate::proxy_manager::now_secs()`.

`last_sync` is display/bookkeeping only ("last synced at"). It is written on
every upload/download and must NOT decide sync direction. (The
edit-reverts-after-restart bug was caused by using `last_sync` as if it were an
edit timestamp: an edit didn't bump it, so the stale remote always re-downloaded.)

Reconcile (`engine.rs::remote_updated_at` + each `sync_X`):
1. `stat` (HEAD) the remote object. Its `updated_at` is read from S3 object
   metadata (`x-amz-meta-updated-at`), with no body download when nothing changed.
2. Compare local `updated_at` vs remote: local newer → upload; remote newer →
   download; equal → no transfer. Legacy objects with no timestamp resolve to 0,
   so any real edit wins.
3. Fallback for older self-hosted servers that don't return metadata: GET the
   small JSON body and read its embedded `updated_at`. Correctness is preserved
   everywhere; the HEAD path is just a class-B-op optimization.

Uploads go through `engine.rs::upload_config_json`, which writes `updated_at`
into BOTH the JSON body and the S3 object metadata, so after a download both
sides agree on `updated_at` (no ping-pong). Adding a new synced config field?
Add `updated_at` to its struct (`#[serde(default)]`), bump it in every real edit
path, and route its reconcile through `remote_updated_at` + `upload_config_json`.

### Server (`donut-sync/`) metadata passthrough

`presignUpload` signs request `metadata` into the PUT as `x-amz-meta-*` and
echoes back what it signed (the Rust client must send exactly those headers on
the PUT or S3 rejects it, hence the echo). `stat` returns `response.Metadata`.
Older servers omit `metadata` → client falls back to the body-GET path. DTOs:
`donut-sync/src/sync/dto/sync.dto.ts`; logic: `sync.service.ts`.

## Team roles and the activity log

- A `launcher` seat opens the team's profiles and keeps what a session changes, never the team's
  settings. The sync server enforces it (`donut-sync` `LAUNCHER_WRITABLE`: only a profile's
  `files/` and `manifest.json`); the app mirrors it so an edit never appears and then reverts.
- `team_roles.rs` lists every Tauri command and MCP tool that changes team data (profiles,
  proxies, groups, VPNs, extensions, templates, statuses). A new command or tool of that kind goes
  in the list; the tests check every name is registered. On a launcher seat the sync engine takes
  the team's copy of every setting and skips its own uploads and deletes of them.
- `team_activity::record` notes a person's edit of a synced entity, next to the `updated_at` bump.
  Only a team seat with team collaboration reports; names leave the app only when E2E encryption
  is off. Launches, stops and membership are written by the cloud itself, not reported.
