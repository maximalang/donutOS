import assert from "node:assert/strict";
import test from "node:test";
import { createWindowResizeWarningQueue } from "./window-resize-warning-queue.ts";

test("single park settles with the dialog answer", async () => {
  const queue = createWindowResizeWarningQueue();
  const proceed = queue.park();
  assert.equal(queue.pending, 1);
  queue.answer(true);
  assert.equal(await proceed, true);
  assert.equal(queue.pending, 0);
});

test("double park settles both waiters with one answer", async () => {
  const queue = createWindowResizeWarningQueue();
  const first = queue.park();
  const second = queue.park();
  assert.equal(queue.pending, 2);
  const released = queue.answer(true);
  assert.equal(released, 2);
  assert.equal(await first, true);
  assert.equal(await second, true);
  assert.equal(queue.pending, 0);
});

test("double park cancel settles both waiters with false", async () => {
  const queue = createWindowResizeWarningQueue();
  const first = queue.park();
  const second = queue.park();
  queue.answer(false);
  assert.equal(await first, false);
  assert.equal(await second, false);
  assert.equal(queue.pending, 0);
});

test("a park after an answer waits for the next answer only", async () => {
  const queue = createWindowResizeWarningQueue();
  const first = queue.park();
  queue.answer(true);
  const second = queue.park();
  assert.equal(queue.pending, 1);
  queue.answer(false);
  assert.equal(await first, true);
  assert.equal(await second, false);
});

test("answer with no waiters is a no-op", () => {
  const queue = createWindowResizeWarningQueue();
  assert.equal(queue.answer(true), 0);
  assert.equal(queue.pending, 0);
});

test("a settled waiter is never answered twice", async () => {
  const queue = createWindowResizeWarningQueue();
  let calls = 0;
  const proceed = queue.park().then((value) => {
    calls += 1;
    return value;
  });
  queue.answer(true);
  queue.answer(false);
  assert.equal(await proceed, true);
  assert.equal(calls, 1);
});
