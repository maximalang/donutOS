# Repository Structure

> Moved from AGENTS.md (deep dive; read when the task touches this area).

## Repository Structure

```
donutbrowser/
├── src/                              # Next.js frontend
│   ├── app/                          # App router (page.tsx, layout.tsx)
│   ├── components/                   # 50+ React components (dialogs, tables, UI)
│   │   ├── table-controls.tsx         # Shared filters, sort rules, column controls, and row selection
│   │   ├── managed-data-table.tsx     # Shared management table and keyboard navigation
│   │   └── tips/                     # Feature tips: SVG scene primitives and one looping scene per tip
│   ├── hooks/                        # Event-driven React hooks, table preferences, filtering, and keyboard controls
│   ├── i18n/locales/                 # Translations (de, en, es, fr, ja, ko, pt, ru, tr, vi, zh)
│   ├── generated/                    # Build-generated third-party license inventory
│   ├── lib/                          # Utilities (themes, toast, browser-utils, ui-sounds, schedules, team-activity)
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
