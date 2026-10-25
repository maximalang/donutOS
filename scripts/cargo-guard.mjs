import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const minimumFreeGiB = 55;
const repoRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const cargoCwd = join(repoRoot, "src-tauri");
const args = process.argv.slice(2);
if (args[0] === "--") args.shift();

if (args.length === 0) {
  console.error("Usage: node scripts/cargo-guard.mjs -- <cargo arguments>");
  process.exit(64);
}

const nonCompilingCommands = new Set(["help", "locate-project", "metadata", "pkgid", "read-manifest", "version"]);
const requiresSpaceGuard = !nonCompilingCommands.has(args[0]);

if (requiresSpaceGuard && process.platform === "win32") {
  const probe = spawnSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      "[Console]::Write((Get-PSDrive -Name C -ErrorAction Stop).Free.ToString([System.Globalization.CultureInfo]::InvariantCulture))",
    ],
    { encoding: "utf8", windowsHide: true },
  );
  const match = /^\s*(\d+)\s*$/.exec(probe.stdout ?? "");
  if (probe.error || probe.status !== 0 || !match) {
    console.error("CARGO_GUARD_ERROR: could not read free space on C:; refusing to start Cargo.");
    process.exit(72);
  }

  const freeBytes = Number(match[1]);
  const requiredBytes = minimumFreeGiB * 1024 ** 3;
  console.log(`CARGO_GUARD: C: free ${(freeBytes / 1024 ** 3).toFixed(2)} GiB; minimum ${minimumFreeGiB} GiB.`);
  if (!Number.isSafeInteger(freeBytes) || freeBytes < requiredBytes) {
    console.error("CARGO_GUARD_BLOCKED: not enough free space; no Cargo process was started.");
    process.exit(71);
  }
}

const env = {
  ...process.env,
  CARGO_BUILD_JOBS: "2",
  CARGO_INCREMENTAL: "0",
};
const result = spawnSync("cargo", args, {
  cwd: cargoCwd,
  env,
  stdio: "inherit",
  windowsHide: true,
});

if (result.error) {
  console.error(`CARGO_GUARD_ERROR: could not start Cargo: ${result.error.message}`);
  process.exit(127);
}
process.exit(result.status ?? 1);
