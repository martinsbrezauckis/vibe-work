/** The fake transport must preserve the host's Debug API attribution for read and acting routes. */
import { expect, it } from "vitest";
import { fakeClient } from "./support/fakeClient";

it.each([
  ["controls", {}, "ui"],
  ["secrets", { action: "lock" }, "debug-api"],
  ["toasts", { action: "clear" }, "debug-api"],
  ["setup", { action: "start" }, "debug-api"],
  ["unknown-route", { action: "read" }, "debug-api"],
] as const)("attributes %s %j to %s", async (name, body, origin) => {
  const client = fakeClient();
  client.request({ id: "pending", name, body });
  await client.execute({ type: "listSessions" });
  expect(client.origins).toEqual([origin]);
});

it("validates every reviewed read and refuses added fields before queueing", async () => {
  const { default: specification } =
    await import("../fixtures/ui-read-contracts.json");
  expect(specification.reads.length).toBeGreaterThanOrEqual(40);
  for (const row of specification.reads) {
    const body: Record<string, unknown> =
      row.action === null ? {} : { action: row.action };
    for (const [name, kind] of Object.entries(row.fields)) {
      body[name] =
        kind === "bool"
          ? true
          : ["duration", "index"].includes(kind)
            ? 2
            : "fixture";
    }
    const client = fakeClient();
    client.request({ id: "read", name: row.route, body });
    await client.execute({ type: "listSessions" });
    expect(client.origins, row.route).toEqual(["ui"]);
    expect(() => {
      client.request({
        id: "invalid",
        name: row.route,
        body: { ...body, unexpected: true },
      });
    }).toThrow(/unsupported/);
    client.request({
      id: "write",
      name: row.route,
      body: { ...body, action: "unknown-action" },
    });
    await client.execute({ type: "listSessions" });
    expect(client.origins, row.route).toEqual(["ui", "debug-api"]);
  }
});
