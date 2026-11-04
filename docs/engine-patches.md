# Wayfern engine patches (offline automation entitlement)

Both shipped engines contain an entitlement gate that refuses CDP automation
(`get_page_content`, `evaluate_javascript`, …) without a paid Donut plan.
The entitlement-gate patches below have live-smoke evidence (navigate plus
`get_page_content` and `evaluate_javascript`). The separate 155 session-cap
patch is version-pinned and statically read back; it is not considered stable
until a live automation session exceeds the five-minute boundary and passes the
required soak.

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

## Compatibility with app 0.35.4 (re-audit 2026-10-25)

Verified during the upstream v0.35.4 merge: between 0.34.0 and 0.35.4 the
upstream did not change the entitlement protocol these patches bypass —
`wayfern_manager.rs` diffs are per-version launch switches and tests only,
and the free build still pins engines (`auto_updater` free no-ops survived
the merge), so a patched 151/154 binary is never replaced or revalidated.
No new patch offsets needed.

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
- The free build patches freshly downloaded Wayfern engines automatically
  (`downloader.rs` → `engine_patch.rs`) and skips the startup version refresh
  (`version_updater.rs`), so an unpatched engine can no longer arrive
  silently.

- The free build ships the **anchor auto-patcher** below (`src-tauri/src/engine_patch.rs`).
  A Wayfern engine is patched and smoke-tested before a new download is
  promoted or profiles are migrated. Failures preserve the engine and `.orig`
  backup; a new version is not registered, and an existing registered version
  is never deleted. Already-patched engines get static readback, not a repeated
  live smoke.

## 155.0.8059.79 — five-minute automation cap

The binary also caps the automation deadline for the free plan at 300 seconds.
Static disassembly of the original `chrome.dll.orig` found two `cmp edi,2; je`
plan branches guarding `now + 0x12c` calculations. Both write the same deadline
RVA `0x1d86e308`; the paid-plan branch skips the addition. The pinned patch
forces only those two branches:

- File offsets: `0x4b9ca45` (`0f 84 06 01 00 00` → `e9 07 01 00 00 90`) and
  `0x4b9f0e1` (`74 46` → `eb 46`).
- The patcher validates the two timer-arm byte sequences, shared deadline, and
  both no-cap branch targets before writing. Any other 155+ engine version,
  including an unverified rebuild suffix such as `155.0.8059.79-2`, fails
  closed; versions older than 155 have no TTL patch.
- Readback is idempotent. On an installed 155 engine both branches must be
  forced before it is accepted. The original engine remains in `chrome.dll.orig`.

The current production `155.0.8059.79/chrome.dll` has these two branch edits;
the source integration now applies them before an engine is registered or
profiles are migrated. The ignored real-binary test is
`the_real_155_session_cap_matches_and_patches_only_the_two_known_branches`.

## Auto-patching v2 — anchor discovery + live smoke (`engine_patch.rs`)

The patcher no longer trusts one build's bytes. A new version is taken
through a ladder, and only a version that clears every rung stays installed:

1. **Anchor discovery.** The refusal message itself ("Browser automation
   requires a paid …", matched by its stable opening) is located in the
   image; every rip-relative `lea` that loads it is found in the code; the
   one enclosed by the entitlement shape — `call rel32` + `test eax,eax` +
   a conditional branch whose target skips the whole refusal — is the gate.
   Register allocation, instruction order, the message tail and its length
   can all change; the shape holds. Noise loads of the same string (logs)
   are not gates: no enclosing branch, no site.
2. **Uniqueness.** Exactly one site or nothing happens. Zero anchored sites
   falls back to the classic 154/155 byte signature; several sites is an
   ambiguity and the version is not registered (`ENGINE_PATCH_FAILED`). Files
   and backups are retained for diagnosis; the app does not delete the engine.
3. **Force.** `je rel32` → `jmp rel32` + nop (displacement +1), original six
   bytes kept as `chrome.dll.orig`. Idempotent: a forced branch is
   recognized, not re-patched.
4. **Live smoke (`verify_gate_open`).** The patched engine is launched
   headless on a throwaway profile and `Runtime.evaluate 6*7` must answer 42
   over CDP — the exact surface the gate exists to block. Anything else —
   refused evaluate or stalled socket — refuses registry promotion
   (`ENGINE_SMOKE_FAILED`).

Proof on the real engines: the anchor lands on `0x3fdeb5b` (154) and
`0x40a381b` (155) — the same sites the manual hunts found — and the installed
patched 155 passes the smoke (ignored tests
`the_real_engine_anchor_matches_the_manual_hunt` /
`the_real_engine_passes_the_smoke`, run with `WAYFERN_ORIG`/`WAYFERN_EXPECT`
and `WAYFERN_SMOKE`).

What still needs a human: a version that rewords the refusal opening *and*
recompiles the shape into something new fails the ladder honestly and is
left unregistered. The hunt scripts and `.orig` beside every install are the
starting point for a new anchor; the patcher never patches blind.

## Engine policy (free build)

- **Wayfern (antidetect, primary): pinned, update on demand only.** No
  periodic auto-update: every new version must pass the patch ladder (anchor →
  uniqueness → force → smoke), so an update is a deliberate act (download
  command → patch → smoke), never a background one. Explicit engine actions
  may refresh the manifest through `api_client`; startup and periodic checks do
  not.
- **Chromium (open engine, now with antidetect): fingerprint-chromium.** The
  free build downloads fingerprint-chromium 148.0.7778.215 (BSD-3) from its
  public GitHub release instead of plain Chrome-for-Testing — pinned version
  and SHA256 (`browser_version_manager.rs`). Launch adds
  `--fingerprint=<seed> --fingerprint-platform=windows`, where the seed is
  FNV-1a of the profile id (`browser_runner.rs`): every profile keeps its own
  deterministic synthetic machine — canvas noise, WebGL GPU strings, hardware
  concurrency, brands — proven by the spike (canvas identical across restarts
  and fresh profiles per seed, GPU strings per seed, codecs present, CDP
  unmetered). Known gaps: AudioContext is not per-seed, fonts are not masked.
  v150 stays unpinned (canvas noising did not fire on it).
- **Unpatchable or smoke-failing Wayfern versions never register**. The engine
  directory and `.orig` are retained for diagnosis; a failed patch never
  deletes an existing registered version.

## Rules

- Engines never update in the background: the free build pins profiles and
  keeps installed binaries (`--features free` disables periodic checks,
  automatic profile migration, and unused-binary removal). Legacy pending
  updates are ignored. A Wayfern install runs the patch ladder before registry
  promotion; if the gate or smoke fails, its files remain unregistered for
  diagnosis. Updates are deliberate, on demand, and verified (see Engine
  policy).
- Startup repair may restore the pinned open Chromium engine, but never fetches
  a Wayfern manifest or auto-downloads a missing Wayfern engine in the free
  build; Wayfern catalog/download requests require an explicit engine action.
- After patching, keep `chrome.dll.orig` next to the patched dll.
- `scripts/hunt_gate.py` (scratch copy) automates locating the gate in new
  builds: plaintext-string xref first, masked-signature scan second.
