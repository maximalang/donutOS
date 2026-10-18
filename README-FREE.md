# Donut-Free (внутренняя сборка)

Бесплатный локальный клон Donut Browser v0.34.0 (upstream `zhom/donutbrowser`, AGPL-3.0).
Режим сборки: cargo-фича `free`. **Без тест-флага e2e** — все локальные функции (включая
proactive-скачивание движка/xray/GeoIP) работают в проде. Облако (backup, team, cookie bot,
remote sessions) отключено честно. Апдейтер выключен compile-time; обновления — свой pipeline from upstream.

## Сборка (артефакты на D:, исходники на C:)
```
cd C:/Users/max/donut-free
pnpm tauri build -- --features free      # CARGO_TARGET_DIR=D:/ZDownloads/donut-free-target
```

## Деплой портабла
```
scripts\deploy-free.cmd                   # -> Desktop\Donut-Free (порт 10109, токен автоген)
scripts\deploy-free.cmd D:\ZDownloads\Donut-Free
```

## Управление
```
Desktop\Donut-Free\donut-freectl.cmd status|start|stop|restart|token|doctor|backup|restore
```

- API: `http://127.0.0.1:10109` (прод v0.29.6 остаётся на 10108)
- Auth: `Authorization: Bearer <config\automation-token.txt>` (пин через env `DONUT_AUTOMATION_TOKEN`, ставится ctl start)
- Health: `GET /v1/health` → `{status, version, build:"free", profiles_total}`
- MCP: `POST /v1/mcp` (JSON-RPC; `mcp-session-id` из заголовка ответа)
- Данные/кэш движка: рядом с exe (`data\`), маркер `.portable`

## Отличия от upstream (патчи)
1. `free`-фича: entitlements локальные, rate-limit 10M/ч, Wayfern-токен встроен (без облака).
2. `/v1/mcp` — прод-роут локального MCP (в оригинале только облако).
3. `/v1/health` — аутентифицированный health для мониторинга.
4. Апдейтер: обе проверки → `Ok(None)`.
5. `DONUT_AUTOMATION_TOKEN` — пиннед токен автоматизации (env, до файлового).
6. Identity: `DonutFree.exe` / `com.donutbrowser.free` — параллельная работа с оригиналом.
7. Release-профиль: fat LTO + strip symbols.
8. Фронт: локальный пользователь вместо облачного логина (`use-cloud-auth.ts`).

## Правки конфигурации Hermes (MCP флота)
`hermes config set mcp_servers.donutfree.url http://127.0.0.1:10109/v1/mcp` (токен — из ctl token).
