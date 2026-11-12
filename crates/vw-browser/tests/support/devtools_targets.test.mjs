// Verify bounded startup discovery for disposable Chromium fixtures.
// Uses Node's test runner and fetch mocks; invoked by the browser regression gate.
import assert from "node:assert/strict";
import test from "node:test";
import { devtoolsTargets } from "./devtools_targets.mjs";

test("discovery tolerates a DevTools listener that starts after three refused connections", async context => {
  let attempts = 0;
  const targets = [{ type: "page", webSocketDebuggerUrl: "ws://127.0.0.1:1234/page" }];
  context.mock.method(globalThis, "fetch", async () => {
    if (++attempts <= 3) throw new Error("connect ECONNREFUSED");
    return Response.json(targets);
  });
  assert.deepEqual(await devtoolsTargets(1234), targets);
  assert.equal(attempts, 4);
});

test("unavailable discovery stays bounded and preserves the startup diagnosis", async context => {
  context.mock.method(globalThis, "fetch", async () => {
    throw new Error("connect ECONNREFUSED");
  });
  const started = performance.now();
  await assert.rejects(devtoolsTargets(1234), /Fixture DevTools discovery failed:.*ECONNREFUSED/);
  assert.ok(performance.now() - started < 8000, "startup discovery must finish within its bounded wait");
});
