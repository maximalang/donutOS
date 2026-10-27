import assert from "node:assert/strict";
import test from "node:test";
import { runCargoGuard } from "./cargo-guard.mjs";

const gib = 1024 ** 3;
const freeSpaceResult = (bytes) => ({ stdout: `${bytes}\n`, status: 0 });

test("blocks a compile below the free-space threshold without starting Cargo", () => {
  const calls = [];
  const status = runCargoGuard({
    args: ["--", "build", "--features", "free"],
    platform: "win32",
    spawn(command, args, options) {
      calls.push({ command, args, options });
      return freeSpaceResult(54 * gib);
    },
  });

  assert.equal(status, 71);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "powershell.exe");
});

test("fails closed if the free-space probe fails", () => {
  const calls = [];
  const status = runCargoGuard({
    args: ["test", "--lib"],
    platform: "win32",
    spawn(command, args, options) {
      calls.push({ command, args, options });
      return { stdout: "", status: 1 };
    },
  });

  assert.equal(status, 72);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "powershell.exe");
});

test("passes Cargo arguments and bounded build environment through", () => {
  const calls = [];
  const status = runCargoGuard({
    args: ["--", "build", "--features", "free"],
    platform: "win32",
    env: { PATH: "test-path", CARGO_BUILD_JOBS: "16", CARGO_INCREMENTAL: "1" },
    spawn(command, args, options) {
      calls.push({ command, args, options });
      return command === "powershell.exe"
        ? freeSpaceResult(56 * gib)
        : { status: 23 };
    },
  });

  assert.equal(status, 23);
  assert.equal(calls.length, 2);
  assert.equal(calls[1].command, "cargo");
  assert.deepEqual(calls[1].args, ["build", "--features", "free"]);
  assert.equal(calls[1].options.env.CARGO_BUILD_JOBS, "2");
  assert.equal(calls[1].options.env.CARGO_INCREMENTAL, "0");
  assert.equal(calls[1].options.env.CARGO_TARGET_DIR, "D:/ZDownloads/donut-free-target");
  assert.equal(calls[1].options.env.PATH, "test-path");
  assert.match(calls[1].options.cwd, /donut-free[\\/]src-tauri$/);
});

test("an explicit CARGO_TARGET_DIR wins over the data-drive default", () => {
  const calls = [];
  const status = runCargoGuard({
    args: ["--", "clippy"],
    platform: "win32",
    env: { CARGO_TARGET_DIR: "E:/custom-target" },
    spawn(command, args, options) {
      calls.push({ command, args, options });
      return command === "powershell.exe"
        ? freeSpaceResult(56 * gib)
        : { status: 0 };
    },
  });

  assert.equal(status, 0);
  assert.equal(calls[1].options.env.CARGO_TARGET_DIR, "E:/custom-target");
});

test("allows exactly 55 GiB and forwards Cargo exit status", () => {
  const calls = [];
  const status = runCargoGuard({
    args: ["build"],
    platform: "win32",
    spawn(command, args, options) {
      calls.push({ command, args, options });
      return command === "powershell.exe"
        ? freeSpaceResult(55 * gib)
        : { status: 9 };
    },
  });

  assert.equal(status, 9);
  assert.equal(calls.length, 2);
  assert.equal(calls[1].command, "cargo");
});

test("fails closed if the free-space output is malformed", () => {
  const calls = [];
  const status = runCargoGuard({
    args: ["build"],
    platform: "win32",
    spawn(command, args, options) {
      calls.push({ command, args, options });
      return { stdout: "free space unavailable", status: 0 };
    },
  });

  assert.equal(status, 72);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "powershell.exe");
});

test("returns a controlled error if Cargo cannot be spawned", () => {
  let calls = 0;
  const status = runCargoGuard({
    args: ["build"],
    platform: "win32",
    spawn(command) {
      calls += 1;
      return command === "powershell.exe"
        ? freeSpaceResult(56 * gib)
        : { error: new Error("spawn failed"), status: null };
    },
  });

  assert.equal(status, 127);
  assert.equal(calls, 2);
});

test("allows non-compiling clean and fmt commands without a disk probe", () => {
  for (const subcommand of ["clean", "fmt"]) {
    const calls = [];
    const status = runCargoGuard({
      args: [subcommand],
      platform: "win32",
      spawn(command, args, options) {
        calls.push({ command, args, options });
        return { status: 0 };
      },
    });

    assert.equal(status, 0);
    assert.equal(calls.length, 1);
    assert.equal(calls[0].command, "cargo");
    assert.deepEqual(calls[0].args, [subcommand]);
  }
});
