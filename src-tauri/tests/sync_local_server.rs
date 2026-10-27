//! Integration tests for the embedded sync server (`sync::local_server`).
//!
//! Each test here mirrors a scenario `tests/sync_e2e.rs` plays against the
//! reference NestJS+MinIO stack, but against a server spawned in-process on a
//! throwaway folder with a pinned bearer token. Same routes, same JSON shapes:
//! if a test here passes while its e2e counterpart passes, the embedded
//! server is a wire-compatible stand-in and `SyncClient` cannot tell them
//! apart.
//!
//! Mirrored scenarios: health/readyz · bearer-auth 401 · stat-missing ·
//! upload/download roundtrip · `x-amz-meta-*` echo · signature forgery ·
//! path traversal · batch presign · list pagination · tombstones on delete ·
//! delete-prefix · device-to-device byte fidelity · snapshots with retention
//! and restore · SSE keep-alive · restart keeps token and store.

use donutbrowser_lib::sync::local_server::{self, LocalSyncHandle};
use reqwest::{Client, Response, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::TempDir;

/// The pinned token `SyncHarness`-style setups pass so tests know what to send.
const TEST_TOKEN: &str = "test-sync-token-0123456789abcdef";

/// A spawned server plus a client pointed at it. One per test: a fresh
/// TempDir store, a fresh loopback port, full isolation.
struct TestServer {
  dir: TempDir,
  handle: LocalSyncHandle,
  client: Client,
}

impl TestServer {
  async fn start() -> Self {
    Self::with_token(TEST_TOKEN).await
  }

  async fn with_token(token: &str) -> Self {
    let dir = TempDir::new().unwrap();
    let handle = local_server::start(dir.path(), Some(token)).await.unwrap();
    Self {
      client: Client::builder().no_proxy().build().unwrap(),
      dir,
      handle,
    }
  }

  fn base(&self) -> &str {
    self.handle.base_url()
  }

  fn token(&self) -> &str {
    self.handle.token()
  }

  fn url(&self, path: &str) -> String {
    format!("{}/v1/objects/{path}", self.base())
  }

  async fn post(&self, path: &str, body: Value) -> Response {
    self
      .client
      .post(self.url(path))
      .bearer_auth(self.token())
      .json(&body)
      .send()
      .await
      .unwrap()
  }

  async fn post_ok(&self, path: &str, body: Value) -> Value {
    let response = self.post(path, body).await;
    assert_eq!(response.status(), StatusCode::OK, "POST /{path}");
    response.json().await.unwrap()
  }

  async fn upload(&self, key: &str, body: &[u8]) {
    self
      .upload_with_headers(key, body, &[("content-type", "application/octet-stream")])
      .await
  }

  async fn upload_with_headers(&self, key: &str, body: &[u8], headers: &[(&str, &str)]) {
    let presign = self
      .post_ok(
        "presign-upload",
        json!({ "key": key, "contentType": "application/octet-stream" }),
      )
      .await;
    let url = presign["url"].as_str().unwrap().to_string();
    let mut request = self.client.put(&url);
    for (name, value) in headers {
      request = request.header(*name, *value);
    }
    let response = request.body(body.to_vec()).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "PUT {key}");
  }

  async fn download(&self, key: &str) -> Response {
    let presign = self
      .post_ok("presign-download", json!({ "key": key }))
      .await;
    let url = presign["url"].as_str().unwrap();
    self.client.get(url).send().await.unwrap()
  }

  async fn stat(&self, key: &str) -> Value {
    self.post_ok("stat", json!({ "key": key })).await
  }
}

/// Flip the last hex character of the `X-Amz-Signature` query parameter.
fn tamper_signature(url: &str) -> String {
  let (head, signature) = url.split_once("X-Amz-Signature=").expect("signed url");
  let mut signature = signature.to_string();
  let flipped = if signature.ends_with('f') { '0' } else { 'f' };
  signature.pop();
  format!("{head}X-Amz-Signature={signature}{flipped}")
}

fn new_id() -> String {
  uuid::Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------------------
// Health and auth
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_and_readyz_answer_without_auth() {
  let server = TestServer::start().await;

  let health = server
    .client
    .get(format!("{}/health", server.base()))
    .send()
    .await
    .unwrap();
  assert_eq!(health.status(), StatusCode::OK);
  let health = health.json::<Value>().await.unwrap();
  assert_eq!(health["status"], "ok");

  let ready = server
    .client
    .get(format!("{}/readyz", server.base()))
    .send()
    .await
    .unwrap();
  assert_eq!(ready.status(), StatusCode::OK);
  let ready = ready.json::<Value>().await.unwrap();
  assert_eq!(ready["status"], "ready");
  assert_eq!(ready["s3"], true);
  assert_eq!(ready["storageEndpoint"], server.base());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn object_api_requires_bearer_token() {
  let server = TestServer::start().await;

  // No token at all.
  let missing = server
    .client
    .post(server.url("stat"))
    .json(&json!({ "key": "profiles/x/manifest.json" }))
    .send()
    .await
    .unwrap();
  assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
  let body = missing.json::<Value>().await.unwrap();
  assert_eq!(body["statusCode"], 401);
  assert_eq!(body["message"], "Unauthorized");

  // A plausible but wrong token is just as shut out.
  let wrong = server
    .client
    .post(server.url("stat"))
    .bearer_auth("wrong-token-0123456789abcdef")
    .json(&json!({ "key": "profiles/x/manifest.json" }))
    .send()
    .await
    .unwrap();
  assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

  // The SSE subscription sits behind the same guard.
  let subscribe = server
    .client
    .get(server.url("subscribe"))
    .send()
    .await
    .unwrap();
  assert_eq!(subscribe.status(), StatusCode::UNAUTHORIZED);

  // With the token the very same call goes through.
  let ok = server
    .client
    .post(server.url("stat"))
    .bearer_auth(TEST_TOKEN)
    .json(&json!({ "key": "profiles/x/manifest.json" }))
    .send()
    .await
    .unwrap();
  assert_eq!(ok.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Object lifecycle
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stat_reports_missing_object_as_absent() {
  let server = TestServer::start().await;

  let stat = server
    .stat(&format!("profiles/{}/manifest.json", new_id()))
    .await;
  assert_eq!(stat["exists"], false);
  assert!(stat["size"].is_null());
  assert!(stat["lastModified"].is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_download_roundtrip_and_delete() {
  let server = TestServer::start().await;
  let key = format!("profiles/{}/manifest.json", new_id());
  let body = b"{ \"profile\": \"work\", \"version\": 7 }";

  // Presign responds in the client's expected shape.
  let presign = server
    .post_ok(
      "presign-upload",
      json!({ "key": key, "contentType": "text/plain", "expiresIn": 300 }),
    )
    .await;
  let url = presign["url"].as_str().unwrap();
  assert!(
    url.starts_with(&format!("{}/storage/", server.base())),
    "{url}"
  );
  assert!(url.contains("X-Amz-Expires="), "{url}");
  assert!(url.contains("X-Amz-Signature="), "{url}");
  assert!(presign["expiresAt"].as_str().is_some());

  // The signed PUT stores the body.
  let put = server
    .client
    .put(url)
    .header("content-type", "text/plain")
    .body(body.to_vec())
    .send()
    .await
    .unwrap();
  assert_eq!(put.status(), StatusCode::OK);
  let etag = put
    .headers()
    .get("etag")
    .and_then(|v| v.to_str().ok())
    .expect("PUT returns an ETag")
    .to_string();

  // stat sees it without downloading: size, timestamp, no metadata.
  let stat = server.stat(&key).await;
  assert_eq!(stat["exists"], true);
  assert_eq!(stat["size"], body.len() as u64);
  assert!(stat["lastModified"].as_str().is_some());

  // The signed GET returns the bytes verbatim.
  let get = server.download(&key).await;
  assert_eq!(get.status(), StatusCode::OK);
  assert_eq!(get.headers()["content-type"], "text/plain");
  assert_eq!(get.headers()["etag"], etag);
  let downloaded = get.bytes().await.unwrap();
  assert_eq!(downloaded.as_ref(), body);

  // Delete without a tombstone key: object gone, nothing planted.
  let deleted = server.post_ok("delete", json!({ "key": key })).await;
  assert_eq!(deleted["deleted"], true);
  assert_eq!(deleted["tombstoneCreated"], false);

  let stat = server.stat(&key).await;
  assert_eq!(stat["exists"], false);
  let get = server.download(&key).await;
  assert_eq!(get.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn put_metadata_headers_are_echoed_by_stat() {
  let server = TestServer::start().await;
  let key = format!("profiles/{}/manifest.json", new_id());

  server
    .upload_with_headers(
      &key,
      b"bundle",
      &[
        ("content-type", "application/json"),
        ("x-amz-meta-updated-at", "2026-10-27T10:00:00Z"),
        ("x-amz-meta-profile-name", "Work"),
      ],
    )
    .await;

  let stat = server.stat(&key).await;
  assert_eq!(stat["exists"], true);
  let metadata = stat["metadata"].as_object().expect("metadata present");
  assert_eq!(metadata.len(), 2, "{metadata:?}");
  assert_eq!(metadata["updated-at"], "2026-10-27T10:00:00Z");
  assert_eq!(metadata["profile-name"], "Work");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forged_or_absent_signatures_are_rejected() {
  let server = TestServer::start().await;
  let key = format!("profiles/{}/files/prefs.json", new_id());
  server.upload(&key, b"{}").await;

  // A flipped hex digit in the signature.
  let presign = server
    .post_ok("presign-download", json!({ "key": key }))
    .await;
  let tampered = tamper_signature(presign["url"].as_str().unwrap());
  let rejected = server.client.get(&tampered).send().await.unwrap();
  assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
  let body = rejected.json::<Value>().await.unwrap();
  assert_eq!(body["message"], "InvalidSignature");

  // No signature at all.
  let bare = server
    .client
    .get(format!("{}/storage/{key}", server.base()))
    .send()
    .await
    .unwrap();
  assert_eq!(bare.status(), StatusCode::FORBIDDEN);

  // A PUT signature presented to GET (method is signed into the tag).
  let upload_url = server
    .post_ok("presign-upload", json!({ "key": key }))
    .await["url"]
    .as_str()
    .unwrap()
    .to_string();
  let wrong_method = server.client.get(&upload_url).send().await.unwrap();
  assert_eq!(wrong_method.status(), StatusCode::FORBIDDEN);

  // A valid signature for a key that was never uploaded: 404, not 403 —
  // the engine distinguishes "gone" from "not allowed".
  let missing = format!("profiles/{}/files/never-there.bin", new_id());
  let missing_url = server
    .post_ok("presign-download", json!({ "key": missing }))
    .await["url"]
    .as_str()
    .unwrap()
    .to_string();
  let not_found = server.client.get(&missing_url).send().await.unwrap();
  assert_eq!(not_found.status(), StatusCode::NOT_FOUND);
  let body = not_found.json::<Value>().await.unwrap();
  assert_eq!(body["message"], "NotFound");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn path_traversal_keys_are_refused() {
  let server = TestServer::start().await;

  for key in [
    "../server.json",
    "a/../b",
    "/etc/passwd",
    "profiles/../../escape",
  ] {
    let stat = server.post("stat", json!({ "key": key })).await;
    assert_eq!(stat.status(), StatusCode::BAD_REQUEST, "stat {key:?}");
    let body = stat.json::<Value>().await.unwrap();
    assert_eq!(body["message"], "invalid key", "stat {key:?}");

    let presign = server.post("presign-upload", json!({ "key": key })).await;
    assert_eq!(presign.status(), StatusCode::BAD_REQUEST, "presign {key:?}");
  }

  let list = server.post("list", json!({ "prefix": "../" })).await;
  assert_eq!(list.status(), StatusCode::BAD_REQUEST);

  let delete_prefix = server
    .post("delete-prefix", json!({ "prefix": "profiles/../x" }))
    .await;
  assert_eq!(delete_prefix.status(), StatusCode::BAD_REQUEST);

  // The valid-but-tricky case the engine actually sends: a directory prefix
  // with a trailing slash must be accepted, not mistaken for an empty part.
  let list = server
    .post(
      "list",
      json!({ "prefix": format!("profiles/{}/", new_id()) }),
    )
    .await;
  assert_eq!(list.status(), StatusCode::OK);
  let list = list.json::<Value>().await.unwrap();
  assert_eq!(list["objects"].as_array().unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn batch_presign_upload_and_download() {
  let server = TestServer::start().await;
  let profile = new_id();
  let keys: Vec<String> = (1..=3)
    .map(|n| format!("profiles/{profile}/files/file-{n:02}.bin"))
    .collect();

  let batch = server
    .post_ok(
      "presign-upload-batch",
      json!({ "items": keys.iter().map(|k| json!({ "key": k })).collect::<Vec<_>>() }),
    )
    .await;
  let items = batch["items"].as_array().unwrap();
  assert_eq!(items.len(), 3);
  for item in items {
    assert!(item["url"]
      .as_str()
      .unwrap()
      .starts_with(&format!("{}/storage/", server.base())));
  }

  // The URLs are live: PUT through the batch-issued one.
  let second_url = items[1]["url"].as_str().unwrap();
  let put = server
    .client
    .put(second_url)
    .header("content-type", "application/octet-stream")
    .body(b"batch body".to_vec())
    .send()
    .await
    .unwrap();
  assert_eq!(put.status(), StatusCode::OK);

  let download_batch = server
    .post_ok("presign-download-batch", json!({ "keys": keys }))
    .await;
  let download_items = download_batch["items"].as_array().unwrap();
  assert_eq!(download_items.len(), 3);
  let by_key = download_items
    .iter()
    .map(|item| (item["key"].as_str().unwrap(), item["url"].as_str().unwrap()))
    .collect::<std::collections::HashMap<_, _>>();
  for key in &keys {
    assert!(by_key.contains_key(key.as_str()), "missing {key}");
  }
  let get = server
    .client
    .get(by_key[&keys[1].as_str()])
    .send()
    .await
    .unwrap();
  assert_eq!(get.status(), StatusCode::OK);
  assert_eq!(get.bytes().await.unwrap().as_ref(), b"batch body");

  // One poisoned key invalidates the whole batch.
  let bad = server
    .post(
      "presign-upload-batch",
      json!({ "items": [{ "key": "../escape" }] }),
    )
    .await;
  assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_walks_pagination_in_key_order() {
  let server = TestServer::start().await;
  let profile = new_id();
  let prefix = format!("profiles/{profile}/files/");

  let mut expected = Vec::new();
  for n in 1..=5 {
    let key = format!("{prefix}file-{n:02}.bin");
    let body = format!("contents-{n:02}").into_bytes();
    server.upload(&key, &body).await;
    expected.push((key, body.len() as u64));
  }

  // Pages of two: file-01/file-02, file-03/file-04, file-05.
  let mut collected = Vec::new();
  let mut token = None;
  loop {
    let mut request = json!({ "prefix": prefix, "maxKeys": 2 });
    if let Some(token) = token.take() {
      request["continuationToken"] = Value::String(token);
    }
    let page = server.post_ok("list", request).await;
    let objects = page["objects"].as_array().unwrap();
    for object in objects {
      collected.push((
        object["key"].as_str().unwrap().to_string(),
        object["size"].as_u64().unwrap(),
        object["lastModified"].as_str().is_some(),
      ));
    }
    if !page["isTruncated"].as_bool().unwrap() {
      break;
    }
    token = page["nextContinuationToken"].as_str().map(str::to_string);
    assert!(
      token.is_some(),
      "truncated page must carry a continuation token"
    );
  }

  assert_eq!(
    collected,
    expected
      .into_iter()
      .map(|(key, size)| (key, size, true))
      .collect::<Vec<_>>(),
    "pages must reconstruct the full sorted listing"
  );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_writes_tombstone_object() {
  let server = TestServer::start().await;
  let profile = new_id();
  let key = format!("profiles/{profile}/manifest.json");
  server.upload(&key, b"{}").await;

  let deleted_at = "2026-10-27T11:00:00Z";
  let tombstone_key = format!("tombstones/profiles/{profile}.json");
  let deleted = server
    .post_ok(
      "delete",
      json!({ "key": key, "tombstoneKey": tombstone_key, "deletedAt": deleted_at }),
    )
    .await;
  assert_eq!(deleted["deleted"], true);
  assert_eq!(deleted["tombstoneCreated"], true);

  // The tombstone is a first-class object: presence plus lastModified is
  // what conflict resolution reads, and the body carries the diagnosis.
  let stat = server.stat(&tombstone_key).await;
  assert_eq!(stat["exists"], true);
  let body = server.download(&tombstone_key).await;
  assert_eq!(body.status(), StatusCode::OK);
  let body = body.json::<Value>().await.unwrap();
  assert_eq!(body["id"], tombstone_key);
  assert_eq!(body["deletedAt"], deleted_at);

  // Deleting an already-gone key still refreshes the tombstone.
  let repeat = server
    .post_ok(
      "delete",
      json!({ "key": key, "tombstoneKey": tombstone_key }),
    )
    .await;
  assert_eq!(repeat["deleted"], false);
  assert_eq!(repeat["tombstoneCreated"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_prefix_removes_tree_and_writes_tombstone() {
  let server = TestServer::start().await;
  let profile = new_id();
  let other = new_id();

  for n in 1..=4 {
    server
      .upload(&format!("profiles/{profile}/files/file-{n:02}.bin"), b"x")
      .await;
  }
  server
    .upload(&format!("profiles/{profile}/manifest.json"), b"{}")
    .await;
  server
    .upload(&format!("profiles/{other}/keep.txt"), b"keep me")
    .await;

  let tombstone_key = format!("tombstones/profiles/{profile}.json");
  let response = server
    .post_ok(
      "delete-prefix",
      json!({ "prefix": format!("profiles/{profile}/"), "tombstoneKey": tombstone_key }),
    )
    .await;
  assert_eq!(response["deletedCount"], 5);
  assert_eq!(response["tombstoneCreated"], true);

  let stat = server
    .stat(&format!("profiles/{profile}/manifest.json"))
    .await;
  assert_eq!(stat["exists"], false);
  let stat = server.stat(&format!("profiles/{other}/keep.txt")).await;
  assert_eq!(stat["exists"], true);

  let listed = server
    .post_ok("list", json!({ "prefix": format!("profiles/{other}/") }))
    .await;
  assert_eq!(listed["objects"].as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_a_upload_survives_to_device_b() {
  let server = TestServer::start().await;
  let profile = new_id();

  // Device A: a manifest and a non-trivial binary bundle.
  let manifest = serde_json::to_vec(&json!({
    "id": profile,
    "name": "Work profile",
    "updatedAt": "2026-10-27T12:00:00Z",
  }))
  .unwrap();
  let mut bundle = Vec::new();
  for n in 0..2048u32 {
    bundle.push((n % 251) as u8);
  }
  server
    .upload_with_headers(
      &format!("profiles/{profile}/manifest.json"),
      &manifest,
      &[
        ("content-type", "application/json"),
        ("x-amz-meta-updated-at", "2026-10-27T12:00:00Z"),
      ],
    )
    .await;
  server
    .upload(&format!("profiles/{profile}/files/bundle.bin"), &bundle)
    .await;

  // Device B: a separate client that never saw A's requests, only the store.
  let device_b = Client::builder().no_proxy().build().unwrap();
  for (suffix, expected) in [
    ("manifest.json", manifest.as_slice()),
    ("files/bundle.bin", bundle.as_slice()),
  ] {
    let presign = server
      .post_ok(
        "presign-download",
        json!({ "key": format!("profiles/{profile}/{suffix}") }),
      )
      .await;
    let get = device_b
      .get(presign["url"].as_str().unwrap())
      .send()
      .await
      .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(
      get.bytes().await.unwrap().as_ref(),
      expected,
      "{suffix} corrupted in transit"
    );
  }

  // B also sees A's conflict-resolution timestamp without downloading.
  let stat = server
    .stat(&format!("profiles/{profile}/manifest.json"))
    .await;
  assert_eq!(stat["metadata"]["updated-at"], "2026-10-27T12:00:00Z");
}

// ---------------------------------------------------------------------------
// Snapshots
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshots_keep_three_per_kind_and_restore_older_point() {
  let server = TestServer::start().await;
  let profile = new_id();
  let profile_root = format!("profiles/{profile}/");
  let manifest_key = format!("profiles/{profile}/manifest.json");

  let create = |body: &'static [u8]| {
    let server = &server;
    let profile_root = profile_root.clone();
    let manifest_key = manifest_key.clone();
    async move {
      server
        .upload_with_headers(&manifest_key, body, &[("content-type", "text/plain")])
        .await;
      let point = server
        .post_ok(
          "snapshots/create",
          json!({ "profileRoot": profile_root, "kind": "manual" }),
        )
        .await;
      assert!(point["id"].as_str().is_some());
      point
    }
  };

  let first = create(b"v1").await;
  let second = create(b"v2").await;
  let third = create(b"v3").await;
  let fourth = create(b"v4").await;

  // Retention: three per kind, oldest pruned — the first is gone.
  let listed = server
    .post_ok("snapshots/list", json!({ "profileRoot": profile_root }))
    .await;
  let snapshots = listed["snapshots"].as_array().unwrap();
  assert_eq!(snapshots.len(), 3, "{snapshots:?}");
  let ids: Vec<&str> = snapshots
    .iter()
    .map(|s| s["id"].as_str().unwrap())
    .collect();
  assert!(
    !ids.contains(&first["id"].as_str().unwrap()),
    "oldest must be pruned"
  );
  assert!(ids.contains(&second["id"].as_str().unwrap()));
  assert!(ids.contains(&third["id"].as_str().unwrap()));
  assert!(ids.contains(&fourth["id"].as_str().unwrap()));
  for snapshot in snapshots {
    assert_eq!(snapshot["kind"], "manual");
    assert_eq!(snapshot["files"], 1);
    assert!(snapshot["bytes"].as_u64().unwrap() > 0);
    assert!(snapshot["createdAt"].as_str().is_some());
  }
  // Newest first.
  let created: Vec<&str> = snapshots
    .iter()
    .map(|s| s["createdAt"].as_str().unwrap())
    .collect();
  let mut sorted = created.clone();
  sorted.sort();
  sorted.reverse();
  assert_eq!(created, sorted);

  // Restore an older point: v2 comes back over whatever is live now.
  let restored = server
    .post_ok(
      "snapshots/restore-files",
      json!({ "profileRoot": profile_root, "id": second["id"] }),
    )
    .await;
  assert_eq!(restored["restored"], true);
  let manifest = server.download(&manifest_key).await;
  assert_eq!(manifest.status(), StatusCode::OK);
  assert_eq!(manifest.bytes().await.unwrap().as_ref(), b"v2");

  // Unknown snapshot id is a client error, not a 500.
  let unknown = server
    .post(
      "snapshots/restore-files",
      json!({ "profileRoot": profile_root, "id": new_id() }),
    )
    .await;
  assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);

  // Deleting a restore point removes it from the list.
  let deleted = server
    .post_ok(
      "snapshots/delete",
      json!({ "profileRoot": profile_root, "id": third["id"] }),
    )
    .await;
  assert_eq!(deleted["deleted"], true);
  let listed = server
    .post_ok("snapshots/list", json!({ "profileRoot": profile_root }))
    .await;
  let ids: Vec<&str> = listed["snapshots"]
    .as_array()
    .unwrap()
    .iter()
    .map(|s| s["id"].as_str().unwrap())
    .collect();
  assert!(!ids.contains(&third["id"].as_str().unwrap()));

  // Profile roots are validated like keys.
  let bad = server
    .post("snapshots/list", json!({ "profileRoot": "../elsewhere" }))
    .await;
  assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Subscription
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_stream_keeps_alive_with_comments() {
  let server = TestServer::start().await;

  let mut response = server
    .client
    .get(server.url("subscribe"))
    .bearer_auth(TEST_TOKEN)
    .send()
    .await
    .unwrap();
  assert_eq!(response.status(), StatusCode::OK);
  let content_type = response.headers()["content-type"]
    .to_str()
    .unwrap()
    .to_string();
  assert!(
    content_type.starts_with("text/event-stream"),
    "{content_type}"
  );

  // A single-machine server has no events to push; what it must do is prove
  // the connection is alive before the client's 60 s read timeout fires.
  // The keep-alive interval is 20 s, so the first comment lands well inside 30.
  let chunk = tokio::time::timeout(Duration::from_secs(30), response.chunk())
    .await
    .expect("keep-alive comment within 30 s")
    .expect("chunk read")
    .expect("stream stays open");
  let text = String::from_utf8(chunk.to_vec()).unwrap();
  assert!(
    text.contains("keepalive"),
    "expected a keep-alive comment, got {text:?}"
  );
}

// ---------------------------------------------------------------------------
// Lifecycle across restarts
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restart_reuses_persisted_token_and_store() {
  let server = TestServer::start().await;
  let key = format!("restart/{}/probe.bin", new_id());
  let body = b"persisted across restarts";
  server.upload(&key, body).await;
  let dir = server.dir;

  let TestServer { handle, client, .. } = server;
  handle.stop().await;

  // Boot the same folder again: the pinned token is honoured and the store
  // survived the restart.
  let handle = local_server::start(dir.path(), Some(TEST_TOKEN))
    .await
    .unwrap();
  assert_eq!(handle.token(), TEST_TOKEN);
  let stat_url = format!("{}/v1/objects/stat", handle.base_url());
  let stat = client
    .post(&stat_url)
    .bearer_auth(TEST_TOKEN)
    .json(&json!({ "key": key }))
    .send()
    .await
    .unwrap();
  assert_eq!(stat.status(), StatusCode::OK);
  let stat = stat.json::<Value>().await.unwrap();
  assert_eq!(stat["exists"], true);
  assert_eq!(stat["size"], body.len() as u64);
  handle.stop().await;

  // A different pinned token overwrites the persisted one.
  let replacement = "replacement-token-0123456789ab";
  let handle = local_server::start(dir.path(), Some(replacement))
    .await
    .unwrap();
  assert_eq!(handle.token(), replacement);
  handle.stop().await;
}
