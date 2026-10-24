#!/usr/bin/env bash
# End-to-end donut-sync protocol test (mirrors app client 0.34.0 flow from src-tauri/src/sync/client.rs+engine.rs).
# Usage: run_e2e.sh <label>   (container name fixed: donut-sync)
# Secret handling: all authorized API calls run via `docker exec donut-sync node /tmp/e2e-api.mjs`
# which reads SYNC_TOKEN inside the container. The host never sees the token.
# Presigned URLs are written to files and never printed (they embed an S3 signature).
set -u
LABEL="${1:?label}"
CT="donut-sync"
EV="$PWD/evidence-${LABEL}.log"
RUN_ID="e2e-$(date +%Y%m%d-%H%M%S)"
KEY="profiles/${RUN_ID}/manifest.json"
TOMB="tombstones/e2e/${RUN_ID}.json"
UPDATED_AT="$(date +%s)"
mkdir -p "$PWD/out-${LABEL}"
OUT="$PWD/out-${LABEL}"

log() { printf '%s | %s\n' "$(date +%H:%M:%S)" "$*" | tee -a "$EV"; }

# --- fixture: manifest.json-like blob ~4KB ---
python - "$(cygpath -m "$OUT/blob.json")" <<'PYEOF'
import json, sys
files = [{"path": f"Default/{i:03d}.txt", "size": 1024 + i, "hash": f"{i:064x}"} for i in range(24)]
doc = {"version": 1, "profileId": "e2e-test-profile", "createdAt": "2026-10-24T00:00:00Z",
       "files": files, "notes": "donutOS sync e2e round-trip fixture (app 0.34.0 protocol emulation)"}
open(sys.argv[1], "w").write(json.dumps(doc, indent=1))
PYEOF
BLOB_SHA="$(sha256sum "$OUT/blob.json" | cut -d' ' -f1)"
BLOB_SIZE="$(wc -c < "$OUT/blob.json" | tr -d ' ')"
log "SETUP label=$LABEL run_id=$RUN_ID key=$KEY blob_bytes=$BLOB_SIZE blob_sha256=$BLOB_SHA"

api() { # api <sub-path> <body-string>  -> prints JSON {status, body}
  printf '%s' "$2" > "$OUT/req.json"
  docker cp "$(cygpath -m "$OUT/req.json")" "$CT:/tmp/e2e-req.json" >/dev/null
  docker exec "$CT" node /tmp/e2e-api.mjs "$1" /tmp/e2e-req.json
}

jget() { printf '%s' "$1" | python "$(cygpath -m "$PWD/jget.py")" "${@:2}"; }

# --- 1. negative auth (host curl, no secret involved) ---
S1=$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:3929/v1/objects/stat" -H 'content-type: application/json' -d '{"key":"x"}')
S2=$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:3929/v1/objects/stat" -H 'content-type: application/json' -H 'Authorization: Bearer e2e-wrong-token-0123456789abcdef' -d '{"key":"x"}')
log "STEP1 auth-negative: no-token=$S1 wrong-token=$S2 (expect 401/401)"

# --- 2. presign-upload (with metadata, as upload_config_json_with does) ---
R=$(api presign-upload "{\"key\":\"$KEY\",\"contentType\":\"application/json\",\"expiresIn\":3600,\"metadata\":{\"updated-at\":\"$UPDATED_AT\"}}")
ST=$(jget "$R" status)
printf '%s' "$R" > "$OUT/presign-up.json"
jget "$R" url > "$OUT/up.url"
UP_URL=$(head -c 200000 "$OUT/up.url")
META_VAL=$(jget "$R" meta-updated)
log "STEP2 presign-upload: http=$ST url_host=$(printf '%s' "$UP_URL" | sed -E 's#(https?://[^/]+).*#\1#') signed_updated-at='${META_VAL}'"

# --- 3. PUT blob to presigned URL (host -> S3_PUBLIC_ENDPOINT 127.0.0.1:9000) ---
if [ -n "$UP_URL" ]; then
  CODE=$(curl -s -o "$(cygpath -m "$OUT/put.resp")" -w '%{http_code}' -X PUT --data-binary "@$(cygpath -m "$OUT/blob.json")" -H "x-amz-meta-updated-at: $META_VAL" "$UP_URL")
  log "STEP3 s3-put: http=$CODE (curl exit=$?)"
else
  log "STEP3 s3-put: SKIP no url"
fi

# --- 4. stat (expect exists, size, metadata.updated-at) ---
R=$(api stat "{\"key\":\"$KEY\"}")
ST=$(jget "$R" status)
EX=$(jget "$R" exists)
SZ=$(jget "$R" size)
UA=$(jget "$R" meta-updated)
log "STEP4 stat: http=$ST exists=$EX size=$SZ metadata.updated-at=$UA (expect exists=True size=$BLOB_SIZE updated-at=$UPDATED_AT)"

# --- 5. presign-download + GET + sha256 compare ---
R=$(api presign-download "{\"key\":\"$KEY\",\"expiresIn\":3600}")
printf '%s' "$R" > "$OUT/presign-dn.json"
jget "$R" url > "$OUT/dn.url"
DN_URL=$(head -c 200000 "$OUT/dn.url")
if [ -n "$DN_URL" ]; then
  CODE=$(curl -s -o "$(cygpath -m "$OUT/downloaded.json")" -w '%{http_code}' "$DN_URL")
  GOT_SHA=$(sha256sum "$OUT/downloaded.json" | cut -d' ' -f1)
  MATCH=$([ "$GOT_SHA" = "$BLOB_SHA" ] && echo YES || echo NO)
  log "STEP5 round-trip: http=$CODE sha256_match=$MATCH (expect YES; src=$BLOB_SHA got=$GOT_SHA)"
else
  log "STEP5 round-trip: SKIP no url"
fi

# --- 6. list prefix ---
R=$(api list "{\"prefix\":\"profiles/${RUN_ID}/\",\"maxKeys\":50}")
ST=$(jget "$R" status)
N=$(jget "$R" list-count "$KEY")
log "STEP6 list: http=$ST key_present=$N (expect 1)"

# --- 7. delete with tombstone, verify both ---
NOW=$(date -u +%Y-%m-%dT%H:%M:%SZ)
R=$(api delete "{\"key\":\"$KEY\",\"tombstoneKey\":\"$TOMB\",\"deletedAt\":\"$NOW\"}")
ST=$(jget "$R" status)
R=$(api stat "{\"key\":\"$KEY\"}")
EX=$(jget "$R" exists)
R=$(api stat "{\"key\":\"$TOMB\"}")
TEX=$(jget "$R" exists)
log "STEP7 delete+tombstone: http=$ST key_exists=$EX tombstone_exists=$TEX (expect False/True)"

# --- 8. cleanup tombstone ---
R=$(api delete "{\"key\":\"$TOMB\"}")
ST=$(jget "$R" status)
R=$(api stat "{\"key\":\"$TOMB\"}")
TEX=$(jget "$R" exists)
log "STEP8 cleanup-tombstone: http=$ST tombstone_exists=$TEX (expect False)"

log "DONE label=$LABEL"
