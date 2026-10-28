# Server Mode — DonutOS self-hosted configuration

**Version:** 0.35.4+ (free)  
**Scope:** Local API + MCP for local and LAN clients, self-hosted sync, no vendor-cloud dependency

---

## Overview

DonutOS serves its automation surface from the app itself: one HTTP server
(the **local API**) carries both REST (`/v1/*`) and the MCP endpoint
(`/v1/mcp`, streamable-HTTP JSON-RPC). Profiles, cookies, proxies and data
stay on this machine. Nothing dials out to a vendor cloud; the vendor's
remote bridge (`mcp_remote`) is cloud-account machinery and stays out of the
free build's story.

- No vendor cloud: all logic is local or self-hosted
- Auth by default: every call — loopback included — needs the Bearer token
- Named API members with roles (`owner` / `operator` / `viewer`)

---

## Quick Start

### Same machine (default)

The API binds `127.0.0.1` on the port from settings (Integrations → Local
API; portable installs default to `10108`, the fleet prod install uses
`10109`).

```bash
# Health (401 without the header means ALIVE, not broken)
curl -H "Authorization: Bearer <token>" http://127.0.0.1:10109/v1/health

# MCP: initialize → notifications/initialized → tools/list → tools/call
curl -X POST http://127.0.0.1:10109/v1/mcp \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -H "Accept: application/json, text/event-stream" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{...}}'
```

The token lives in `config/automation-token.txt` next to the exe.

### LAN / remote machines (fleet)

1. In the app: **Integrations → Local API → Listen address → LAN**. This
   stores `api_listen = 0.0.0.0` and rebinds the running server.
2. Allow the app through Windows Firewall when prompted (private network).
3. Clients reach `http://<lan-ip>:<port>/v1/mcp` with the same Bearer token.

The bearer token and the role gate apply to every call, on- or off-host.
For fine-grained access hand out API-member tokens (Integrations → API
members) instead of the master token: `operator` can drive the fleet,
`viewer` only reads.

---

## Configuration

There is no separate server binary and no `config.json`: the desktop app is
the server. Everything is settings-owned.

| Setting | Default | Where |
|-----|---------|---|
| `api_enabled` | `false` | Integrations → Local API |
| `api_port` | `10108` | Integrations → Local API |
| `api_listen` | `127.0.0.1` (`0.0.0.0` = LAN) | Integrations → Local API |
| API token | generated | `config/automation-token.txt`, rotatable |
| API members + roles | — | Integrations → API members |

Values other than `127.0.0.1`/`0.0.0.0` for `api_listen` are clamped to
`127.0.0.1` on save — a bad address must never dead-end the bind.

### REST surface (verified routes)

| Method | Path | Description |
|--------|------|---|
| `GET` | `/v1/health` | Status, version, build, profile count |
| `GET` | `/v1/profiles` | List profiles (engine + version per profile) |
| `POST` | `/v1/profiles/{id}/run` | Launch a profile headless, returns CDP endpoint |
| `POST` | `/v1/profiles/{id}/kill` | Stop a running profile |
| `POST` | `/v1/browsers/download` | Install an engine (long client timeout) |
| `POST` | `/v1/profiles/{id}/cloud-sync` | Trigger a profile sync |
| `POST` | `/v1/mcp` | MCP streamable-HTTP endpoint (JSON-RPC, ~117 tools) |
| `DELETE` | `/v1/mcp?session_id=<id>` | End an MCP session |

### Environment variables (free builds)

| Var | Default | Description |
|-----|---------|---|
| `DONUT_SYNC_URL` | (none) | Self-hosted sync server URL; adopted once at startup |
| `DONUT_SYNC_TOKEN` | (none) | The server's shared token; sealed into the vault, never logged |

Both must be set, and the store must have neither a sync URL nor a stored
token — provisioning never overwrites an explicit choice.
`DONUTBROWSER_DATA_ROOT` isolates data+logs+cache for isolated test
instances.

---

## Self-hosted sync

Server side (Docker, works on the same host):

```bash
cd scripts/donut-sync && docker compose up -d   # donut-sync + rclone S3
# health: http://127.0.0.1:3929/health → 200
```

The image is the official `donutbrowser/donut-sync` release pinned to the
app's protocol version; the shared token lives in `scripts/donut-sync/.env`.

App side, two equivalent paths:

- **UI:** a profile → Sync settings → sync configuration dialog. In free
  builds it opens straight on the *self-hosted* tab: server URL + token,
  connection test (checks the storage endpoint too), Save restarts the sync
  service.
- **Environment (fleet installs):** set `DONUT_SYNC_URL` + `DONUT_SYNC_TOKEN`
  for the app process; the first launch adopts them.

A signed-out free install syncs with the self-hosted server only
(`sync_server_url` → SelfHosted source); no vendor account is involved.

---

## Security

- **Authentication:** every route requires `Authorization: Bearer <token>`
  — on loopback too. Dead = connection refused; a 401 means alive.
- **Roles:** the master token is an implicit `owner`. Member tokens carry
  their own role; destructive calls are owner-only, reads are for everyone.
- **Bind scope:** loopback by default; LAN exposure is an explicit,
  per-install setting.
- **Audit:** significant mutations are journaled (`api.mutation` records in
  the local activity journal) with the acting member's name.

### Data isolation

- Each profile is an isolated engine process, userdata dir and cookie store
- Sync payloads are sealed client-side before they leave the machine
- No telemetry, no phoning home to the vendor

Not present (and not needed for these flows): TLS termination, IP allowlists,
a standalone server binary. Terminate TLS in front of the app if you need
HTTPS beyond the LAN.

---

## Troubleshooting

- **"MCP server not responding"** — check the port with
  `netstat -ano | findstr <port>`; the log is
  `<install>/logs/DonutBrowser.log`.
- **Authentication failed** — the token is `config/automation-token.txt`;
  a 401 on `/v1/health` without a header is the server being alive.
- **Sync connection refused** — `docker ps` must show `donut-sync` and
  `donut-sync-s3`; the app-side URL must be reachable from the app host and
  use the same scheme (http/https).

---

## Reference

- **Upstream Donut:** https://github.com/zhom/donutbrowser
- **Feature Matrix:** [feature-matrix.md](./feature-matrix.md)
- **Sync Setup:** `scripts/donut-sync`
- **MCP Spec:** https://modelcontextprotocol.io
- **License:** AGPL-3.0 (see upstream attribution)
