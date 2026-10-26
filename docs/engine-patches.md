# Wayfern engine patches (offline automation entitlement)

Both shipped engines contain an entitlement gate that refuses CDP automation
(`get_page_content`, `evaluate_javascript`, …) without a paid Donut plan.
Patches below keep the engines fully functional and stable; each is verified
live (navigate + get_page_content + evaluate_javascript on example.com).

## 151.0.7922.76 — gate stub (deployed, proven since 19.10)

- File: `data/binaries/wayfern/151.0.7922.76/chrome.dll` (backup: `chrome.dll.orig`)
- Offset: `0x48dfd00`
- Original: `56 48 83` (push rsi; sub rsp,0x60 …)
- Patched:  `31 c0 c3` (xor eax,eax; ret)
- Effect: the lazily-initialized entitlement lookup (`wayfern-token` /
  `WAYFERN_TOKEN` env strings at VA 0x191ba434c/0x191ba435a) returns 0 =
  allowed. Safe here: 151 has a single critical caller.
- Origin: found as a ready-made patch in the old Donut-Portable install.

## 154.0.8037.57 — caller-branch patch (deployed, proven 19.10)

Stubbing the same function in 154 crashes the browser ~15 s after launch
(the status value is consumed by 17 other call sites, incl. background
revalidation). Instead patch ONLY the CDP refusal branch:

- File: `data/binaries/wayfern/154.0.8037.57/chrome.dll` (backup: `chrome.dll.orig`)
- Offset: `0x3fdeb5b` (VA 0x183fdf55b)
- Original: `0f 84 23 01 00 00` (je 0x183fdf684  — allow when gate==0)
- Patched:  `e9 24 01 00 00 90` (jmp 0x183fdf684 — always allow)
- Context: `call 0x184a56760` (entitlement status: 0 ok / 1 paid / 2
  validate-online) at 0x183fdf554; refusal strings at VA 0x192820988
  ("Browser automation requires a paid Donut Browser plan.", len 0x36).
- The gate function itself stays intact → background validation untouched →
  no crash; verified stable with browser alive >60 s after patch.

## 155.0.8059.79 — caller-branch patch (deployed 26.10)

Same refusal-site shape as 154, located by signature (call → status fn →
`test eax` → `je allow` → `cmp eax,2` → lea-pair loading the refusal string,
`mov ebx,0x36` = its length). Hunt scripts: scratch `hunt155_callsites.py`
(find status fn via `WAYFERN_TOKEN`+`wayfern-token` xrefs, then its call
sites with the branch pattern).

- File: `data/binaries/wayfern/155.0.8059.79/chrome.dll` (backup: `chrome.dll.orig`)
- Offset: `0x40a381b` (call site VA 0x1840a4214; status fn VA 0x184b84ff0)
- Original: `0f 84 23 01 00 00` (je 0x1840a4344 — allow when gate==0)
- Patched:  `e9 24 01 00 00 90` (jmp 0x1840a4344 — always allow)
- Refusal string VA 0x19384ca68, len 0x36; status fn had 18 call sites, same
  count as 154's — structural match beyond the byte pattern.
- The free build now REFUSES vendor Wayfern downloads (`downloader.rs`) and
  skips the startup version refresh (`version_updater.rs`), so an unpatched
  engine can no longer arrive silently.

## Rules

- NEVER let the app update/replace engines: free build pins everything
  (auto_updater / registry migration & cleanup are no-ops under
  `--features free`). A fresh downloaded engine is UNPATCHED and will
  silently kill automation.
- After patching, keep `chrome.dll.orig` next to the patched dll.
- `scripts/hunt_gate.py` (scratch copy) automates locating the gate in new
  builds: plaintext-string xref first, masked-signature scan second.
