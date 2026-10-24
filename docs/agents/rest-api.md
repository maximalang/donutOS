# REST API, OpenAPI spec, error conventions

> Moved from AGENTS.md (deep dive; read when the task touches this area).

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
