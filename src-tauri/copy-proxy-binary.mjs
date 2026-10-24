import { execFileSync, execSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { downloadXray } from "./download-xray.mjs";

const MANIFEST_DIR = dirname(fileURLToPath(import.meta.url));
const PROFILE =
  process.argv.includes("--release") || process.env.PROFILE === "release"
    ? "release"
    : "debug";

function getTarget() {
  if (process.env.TARGET) return process.env.TARGET;
  try {
    const output = execSync("rustc -vV", { encoding: "utf-8" });
    const match = output.match(/host:\s*(.+)/);
    if (match) return match[1].trim();
  } catch {}
  return "unknown";
}

function getHostTarget() {
  try {
    const output = execSync("rustc -vV", { encoding: "utf-8" });
    const match = output.match(/host:\s*(.+)/);
    if (match) return match[1].trim();
  } catch {}
  return "unknown";
}

const TARGET = getTarget();
const HOST_TARGET = getHostTarget();
const isWindows = TARGET.includes("windows");

// Determine source directory: honour CARGO_TARGET_DIR / workspace target dir
// (the nested `cargo build` below inherits this process's environment, so
// CARGO_TARGET_DIR is authoritative when set; falls back to cargo metadata,
// then to the in-repo target/ dir).
function getTargetDir() {
  if (process.env.CARGO_TARGET_DIR) return process.env.CARGO_TARGET_DIR;
  try {
    const output = execSync("cargo metadata --format-version 1 --no-deps", {
      encoding: "utf-8",
      cwd: MANIFEST_DIR,
    });
    return JSON.parse(output).target_directory;
  } catch {}
  return join(MANIFEST_DIR, "target");
}

let srcDir;
if (TARGET === HOST_TARGET || TARGET === "unknown") {
  srcDir = join(getTargetDir(), PROFILE === "release" ? "release" : "debug");
} else {
  srcDir = join(
    getTargetDir(),
    TARGET,
    PROFILE === "release" ? "release" : "debug",
  );
}

const destDir = join(MANIFEST_DIR, "binaries");
mkdirSync(destDir, { recursive: true });

function copyBinary(baseName) {
  const binName = isWindows ? `${baseName}.exe` : baseName;
  const source = join(srcDir, binName);

  let destName = `${baseName}-${TARGET}`;
  if (isWindows) destName += ".exe";
  const dest = join(destDir, destName);

  const buildArgs = ["build", "--bin", baseName];
  if (PROFILE === "release") buildArgs.push("--release");
  if (TARGET !== "unknown" && TARGET !== HOST_TARGET) {
    buildArgs.push("--target", TARGET);
  }
  execFileSync("cargo", buildArgs, {
    cwd: MANIFEST_DIR,
    stdio: "inherit",
  });

  if (!existsSync(source)) {
    console.error(`Error: Failed to build ${baseName} binary`);
    process.exit(1);
  }
  // macOS kills an executable that was overwritten in place.
  rmSync(dest, { force: true });
  copyFileSync(source, dest);
  console.log(`Built and copied ${binName} to ${dest}`);
}

copyBinary("donut-proxy");
await downloadXray(TARGET);
