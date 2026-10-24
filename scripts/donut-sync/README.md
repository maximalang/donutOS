# donut-sync — self-hosted сервер синхронизации Donut Free

Docker compose: [donutbrowser/donut-sync](https://hub.docker.com/r/donutbrowser/donut-sync)
(пин `v0.35.4`, image `43b704bae7d0`) + `rclone serve s3` как S3-хранилище.

| Порт | Назначение |
|------|-----------|
| 3929 | sync API (`/v1/objects/*`) — управление, presign, stat, list |
| 9000 | S3 — прямой перенос файлов браузером (presigned PUT/GET) |

Данные бакета: `D:/ProgramData/donut-sync/s3data` (bind mount).

## Первый запуск

1. Рядом с `docker-compose.yml` создать `.env`:
   ```
   DONUT_SYNC_TOKEN=<случайный секрет >= 24 символов>
   DONUT_S3_SECRET=<случайный секрет S3>
   ```
   Сервер падает на старте при плейсхолдер-токене или длине < 24 (`main.ts validateEnv`).
2. `docker compose up -d`
3. Smoke: `curl -X POST http://127.0.0.1:3929/v1/objects/stat -H 'content-type: application/json' -d '{"key":"x"}'`
   → `401` (AuthGuard работает, сервер жив).

## Подключение приложения (только через UI)

Токен app хранит зашифрованным на диске (`settings_manager.rs`, `encrypt_to_file` c
`DBSYN`) — REST-провижининга нет, поэтому подключение вручную:

1. Donut Free → Settings (Account) → **Sync** → вкладка **Self-hosted**
   (поля `self-hosted-server-url` / `self-hosted-token`).
2. **Server URL**: `http://127.0.0.1:3929` (если app на другой машине — IP этого хоста).
3. **Token**: значение `DONUT_SYNC_TOKEN` из `.env`.
4. **Test Connection** → должен показать успех → **Save**.

После сохранения в `%USERPROFILE%/Desktop/Donut-Free/data/settings/app_settings.json`
`sync_server_url` перестаёт быть `null` и профили получают sync.

Если app запускается не на этом хосте, поменяйте `S3_PUBLIC_ENDPOINT` (адрес, который
браузер использует для presigned-трансферов) на IP хоста; `S3_ENDPOINT` (`http://s3:9000`)
не трогать — это внутренний docker-DNS.

## Протокол (app 0.34.0; в 0.35.4 не менялся)

- Auth: `Authorization: Bearer <SYNC_TOKEN>` на каждый вызов (timing-safe compare; без/с
  неверным токеном — `401`).
- `POST /v1/objects/{stat,presign-upload,presign-download,delete,list,
  presign-upload-batch,presign-download-batch,delete-prefix,
  snapshots/list,snapshots/create,snapshots/delete,snapshots/restore-files}`
  + `GET /v1/objects/subscribe` (SSE).
- Upload: `presign-upload {key, contentType, expiresIn, metadata}` → `PUT` на
  presigned-URL, отправляя заголовки `x-amz-meta-*` ровно те, что сервер вернул в
  `metadata` (иначе S3 отклоняет подпись) → подтверждение через `stat`.
- Download: `presign-download {key}` → `GET` presigned-URL.
- Layout ключей: `profiles/{id}/manifest.json`, `profiles/{id}/metadata.json`,
  `profiles/{id}/files/{relpath}`, `proxies/{id}.json`, `tombstones/{kind}/{id}.json`.
- Delete: `{key, tombstoneKey?, deletedAt}` — tombstone пишется для конфликт-резолюции.

## Совместимость 0.34.0 ↔ 0.35.4 (verified 2026-10-24)

- Сервер `donut-sync/src` v0.34.0 → v0.35.4: diff только 3 файла; единственное
  функциональное изменение — лимит профилей теперь enforcement только в `mode=cloud`
  (self-hosted не затронут); состав маршрутов идентичен (13).
- Клиент `src-tauri/src/sync/client.rs` идентичен между v0.34.0 и v0.35.4; новые файлы
  клиента 0.35.x (`health.rs` — локальный UI-индикатор без HTTP, `conflicts.rs`) новых
  серверных эндпоинтов не требуют.
- e2e-прогон обеих версий сервера зелёный (`v0.34.0` = `5ce3beff547d`, `v0.35.4` =
  `43b704bae7d0`): round-trip blob через presign → PUT → stat → GET (sha256 match),
  list, delete+tombstone, 401-negative.
- Вывод: **сервер v0.35.4 совместим с app 0.34.0; апгрейд app до 0.35.x не требует
  пересборки/смены сервера.** При будущих апгрейдах гонять `e2e/run_e2e.sh` до и после.

## e2e-проверка

```bash
cd scripts/donut-sync/e2e
docker cp api.mjs donut-sync:/tmp/e2e-api.mjs   # после каждого пересоздания контейнера
bash run_e2e.sh <label>                          # 8 шагов, лог evidence-<label>.log
```

Все авторизованные вызовы выполняются внутри контейнера (`docker exec node`,
`SYNC_TOKEN` из env) — токен не покидает контейнер и не попадает в логи; presigned URL
не печатаются (подпись S3).

## Обслуживание

- Бэкап: весь бакет — каталог `D:/ProgramData/donut-sync/s3data`.
- Смена токена: новое `DONUT_SYNC_TOKEN` в `.env` → `docker compose up -d` →
  переподключить app в UI.
- Апгрейд образа: сменить тег в `docker-compose.yml` → `docker compose pull donut-sync`
  → `docker compose up -d` → `bash e2e/run_e2e.sh upgrade-check`.
