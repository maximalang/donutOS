# Server Mode — donutOS self-hosted configuration

**Version:** 0.35.4 (free)  
**Date:** 2026-10-25  
**Scope:** Local MCP remote server, self-hosted sync, no vendor-cloud dependency

---

## Overview

DonutOS **server-mode** runs Donut Browser as a **local MCP server** for remote clients (Claude, Cursor, Codex, Hermes agents) over a network or LAN. All profiles, cookies, proxies, and data stay on your machine or your self-hosted server.

- **No vendor cloud:** all logic local or self-hosted
- **Safe by default:** explicit allowlist for remote clients, password-protected
- **Full privacy:** end-to-end encryption optional for sync
- **Production-ready CI:** automated Windows x64 releases via GitHub Actions

---

## Quick Start

### Local MCP Server (same machine)

```bash
# Build donutOS
cd src-tauri
cargo build --release --features free

# Run with MCP remote enabled (listen on localhost:3000)
./target/release/donut_free --mcp-listen 127.0.0.1:3000

# In Claude Desktop or Hermes agent:
# Connect to stdio or TCP:
#   "type": "stdio"
#   "command": "donut_free"
#   "args": ["--mcp-listen-tcp", "127.0.0.1:3000"]
```

### LAN/Network Server

```bash
# Listen on LAN interface (0.0.0.0:3000, protected by API token)
donut_free --mcp-listen 0.0.0.0:3000 --mcp-auth-token "your-secret-token-here"

# Client connects with token header:
#   Authorization: Bearer your-secret-token-here
```

---

## Configuration

### Environment Variables

| Var | Default | Description |
|-----|---------|---|
| `DONUT_MCP_LISTEN` | `127.0.0.1:3000` | MCP server bind address:port |
| `DONUT_MCP_AUTH_TOKEN` | (none) | Optional API token for remote auth |
| `DONUT_PROFILE_DIR` | `~/.donut-profiles` | Local profile storage |
| `DONUT_SYNC_SERVER` | (none) | Self-hosted sync URL (optional) |
| `DONUT_SYNC_E2E_PASSWORD` | (none) | E2E sync password (optional) |

### Config File (`~/.donut/config.json`)

```json
{
  "server_mode": {
    "enabled": true,
    "listen": "0.0.0.0:3000",
    "auth_token": "your-secret-here",
    "tls_cert": "/path/to/cert.pem",
    "tls_key": "/path/to/key.pem"
  },
  "sync": {
    "server_url": "https://sync.example.com",
    "e2e_password": "your-sync-password",
    "auto_sync": true,
    "sync_interval_sec": 300
  },
  "profiles": {
    "storage_path": "/var/donut-profiles",
    "max_profiles": 1000,
    "auto_cleanup_ephemeral": true
  },
  "security": {
    "mcp_ip_allowlist": ["127.0.0.1", "192.168.1.0/24"],
    "enforce_https": false,
    "require_auth": true
  }
}
```

---

## MCP API Reference

### REST Endpoints (localhost)

| Method | Path | Description |
|--------|------|---|
| `GET` | `/v1/mcp` | MCP server status (200 OK) |
| `DELETE` | `/v1/mcp?session_id=<id>` | End MCP session |
| `POST` | `/v1/profiles/list` | List local profiles |
| `POST` | `/v1/profiles/launch` | Launch profile, get CDP endpoint |
| `POST` | `/v1/profiles/close` | Close profile |
| `GET` | `/v1/sync/status` | Sync health & status |

### MCP Wire Format (no vendor envelope)

```json
{
  "jsonrpc": "2.0",
  "method": "tools/list",
  "id": 1
}
```

**NOT:**
```json
{
  "session_id": "...",
  "body": { "jsonrpc": "2.0", "method": "..." }
}
```

---

## Security

### Authentication

- **Local (127.0.0.1):** no auth required by default
- **LAN/remote:** require `DONUT_MCP_AUTH_TOKEN` header
- **TLS:** optional, self-signed or CA-signed cert (`tls_cert`, `tls_key`)

### IP Allowlist

```json
{
  "security": {
    "mcp_ip_allowlist": [
      "127.0.0.1",
      "::1",
      "192.168.1.0/24",
      "10.0.0.0/8"
    ]
  }
}
```

### Data Isolation

- Each profile has isolated Chromium process, userdata dir, cookies
- Sync data encrypted client-side before leaving the machine
- No telemetry, no phoning home to vendor

---

## Self-Hosted Sync

Run `donut-sync` container or binary for profile/proxy/group sync across devices:

```bash
# Docker (local)
docker run -d \
  --name donut-sync \
  -p 8080:8080 \
  -v donut-data:/data \
  maximalang/donut-sync:v0.35.4

# Then point donutOS to it:
export DONUT_SYNC_SERVER=http://127.0.0.1:8080
```

See [`scripts/donut-sync`](../scripts/donut-sync) for full setup.

---

## Integration: Hermes Agent

```yaml
# Hermes profile config
mcp_servers:
  donut:
    type: sse
    command: donut_free
    args: ["--mcp-listen-tcp", "127.0.0.1:3000"]
    env:
      DONUT_MCP_AUTH_TOKEN: "your-token"
```

Agent can then:
- Launch profiles on demand
- Connect Playwright/Puppeteer to CDP
- Run browser automation scripts
- Sync profiles across agent runs

---

## Troubleshooting

### "MCP server not responding"

```bash
# Check if server is listening
netstat -tuln | grep 3000

# Check logs
tail -f ~/.donut/app.log | grep mcp
```

### "Authentication failed"

```bash
# Verify token header
curl -H "Authorization: Bearer your-token" \
  http://127.0.0.1:3000/v1/mcp
```

### "Sync connection refused"

Ensure `DONUT_SYNC_SERVER` is reachable and has same protocol (http/https).

---

## Reference

- **Upstream Donut:** https://github.com/zhom/donutbrowser
- **Feature Matrix:** [feature-matrix.md](./feature-matrix.md)
- **Sync Setup:** `scripts/donut-sync`
- **MCP Spec:** https://modelcontextprotocol.io
- **License:** AGPL-3.0 (see upstream attribution)
