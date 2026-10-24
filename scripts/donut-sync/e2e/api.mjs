// Container-side REST helper for the donut-sync e2e test.
// Usage: node /tmp/e2e-api.mjs <objects-sub-path> [body-json-file]
// Reads SYNC_TOKEN from the container env; never prints it.
import { readFile } from 'node:fs/promises';

const [, , subPath, bodyFile] = process.argv;
if (!subPath) {
  console.error('usage: e2e-api.mjs <sub-path> [body-file]');
  process.exit(2);
}
let body = '{}';
if (bodyFile) body = await readFile(bodyFile, 'utf8');

const base = process.env.E2E_BASE_URL || 'http://127.0.0.1:3929';
const token = process.env.SYNC_TOKEN || '';
try {
  const res = await fetch(`${base}/v1/objects/${subPath}`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
    body,
  });
  const text = await res.text();
  console.log(JSON.stringify({ status: res.status, body: text.length > 4000 ? text.slice(0, 4000) : text }));
} catch (e) {
  console.log(JSON.stringify({ status: 0, error: String(e) }));
}
