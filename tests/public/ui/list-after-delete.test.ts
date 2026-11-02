/** The window's chat list is not read while one of its own deletes is still on its way: Vibe removes the chat's
 * folder while it answers a listing that reads the same files, and on Windows the delete then fails because the
 * listing holds one of them open. A refresh asked meanwhile (the window regaining the focus when the delete's
 * confirmation closes) lists once the delete has answered. Synthetic data. */
import { describe, expect, it } from "vitest";
import type { Command, CommandResult } from "../../../ui/src/protocol/core";
import { createCore } from "../../../ui/src/state/coreStore";
import { fakeClient } from "./support/fakeClient";

function setup() {
  const client = fakeClient();
  let answerDelete: (result: CommandResult) => void = () => undefined;
  const scripted = client.execute.bind(client);
  client.execute = (command: Command, options) => {
    if (command.type === "call" && command.method === "session/delete") {
      client.commands.push(command);
      return new Promise<CommandResult>((resolve) => {
        answerDelete = resolve;
      });
    }
    return scripted(command, options);
  };
  const core = createCore(client, () => undefined);
  const lists = () =>
    client.commands.filter((c) => c.type === "listSessions").length;
  return {
    core,
    lists,
    answerDelete: () => {
      answerDelete({ type: "reply", result: {} } as CommandResult);
    },
  };
}

describe("the chat list and the window's own delete", () => {
  it("lists only after the delete has answered", async () => {
    const { core, lists, answerDelete } = setup();
    await core.start();
    const before = lists();
    const deleting = core.run({
      type: "call",
      method: "session/delete",
      params: { sessionId: "s1" },
    });
    const refreshed = core.refresh();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(lists()).toBe(before);
    answerDelete();
    await deleting;
    await refreshed;
    expect(lists()).toBe(before + 1);
  });

  it("lists at once when no delete is on its way", async () => {
    const { core, lists } = setup();
    await core.start();
    const before = lists();
    await core.refresh();
    expect(lists()).toBe(before + 1);
  });
});
