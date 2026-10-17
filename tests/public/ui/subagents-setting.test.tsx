/**
 * Settings tests for the Subagents switch. A synthetic core checks that the switch changes
 * settings through session tools while ordinary settings still use the open chat.
 */
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import type {
  Command,
  CommandResult,
  CoreError,
} from "../../../ui/src/protocol/core";
import { snapshot } from "./support/fakeClient";
import { renderShell } from "./support/renderShell";

const CHAT = "chat-1";
const reply = (result: unknown): CommandResult => ({
  type: "reply",
  result: result as never,
  clientUserMessageId: null,
  userEntryIds: [],
});

/** What `sessionTools setSubagents` answers: a result, or an error. */
type SetAnswer =
  { enabled: boolean | null; leftoverSessionId: string | null } | CoreError;

interface Script {
  /** What the open chat's own `config/fields/read` reports for Subagents. */
  subagentsRead: boolean;
  backgroundRead: boolean;
  setAnswer: SetAnswer;
}

const refusal: CoreError = {
  code: "serverError",
  message: "Vibe did not accept the change.",
  method: "config/write",
  summary: null,
  serverCode: null,
};

function fieldsResult(script: Script) {
  return {
    fields: [
      { name: "default_agent", value: "ask", layerValues: [] },
      {
        name: "enable_background_processes",
        value: script.backgroundRead,
        layerValues: [],
      },
      {
        name: "enable_subagents",
        value: script.subagentsRead,
        layerValues: [{ layer: "user-toml", value: script.subagentsRead }],
      },
      { name: "api_stream_idle_timeout", value: 0, layerValues: [] },
      { name: "models", value: [] },
    ],
  };
}

const isCall = (c: Command, method: string): boolean =>
  c.type === "call" && c.method === method;
const setSubagents = (c: Command): boolean =>
  c.type === "sessionTools" && c.request.op === "setSubagents";

async function rig(over: Partial<Script> = {}) {
  const script: Script = {
    subagentsRead: true,
    backgroundRead: false,
    setAnswer: { enabled: false, leftoverSessionId: null },
    ...over,
  };
  const r = await renderShell({
    setup(client) {
      client.answerWith("call", (cmd) => {
        if (cmd.type !== "call") throw new Error("expected a call");
        if (cmd.method === "config/fields/read")
          return reply(fieldsResult(script));
        return reply({ runtime: {} });
      });
      client.answerWith("sessionTools", (cmd) => {
        if (cmd.type !== "sessionTools")
          throw new Error("expected sessionTools");
        if (cmd.request.op === "temporary")
          return {
            type: "sessionTools",
            result: { kind: "temporary", sessionIds: [] },
          };
        if (cmd.request.op !== "setSubagents")
          throw new Error(`unexpected op ${cmd.request.op}`);
        const answer = script.setAnswer;
        if ("code" in answer) return answer;
        return {
          type: "sessionTools",
          result: { kind: "subagentsSet", ...answer },
        };
      });
    },
  });
  act(() => {
    r.client.emit(snapshot(CHAT));
    r.app.shell.actions.selectSession(CHAT);
    r.app.shell.actions.openSettings("models");
  });
  return { ...r, script };
}

async function subagentsSwitch() {
  const sw = await screen.findByRole("switch", { name: "Subagents" });
  await waitFor(() => expect(sw).toBeEnabled());
  return sw;
}

const row = (id: string): HTMLElement => {
  const found = document.querySelector<HTMLElement>(`[data-setting="${id}"]`);
  if (found === null) throw new Error(`no row ${id}`);
  return found;
};

describe("Settings: the Subagents switch", () => {
  it("asks the core and never writes through the open chat", async () => {
    const r = await rig();
    const sw = await subagentsSwitch();
    expect(sw).toHaveAttribute("aria-checked", "true");

    await userEvent.setup().click(sw);
    // Whatever the switch sent (the core, or a write through the open chat on the old code) has been sent.
    await waitFor(() => {
      expect(
        r.client.commands.some(
          (c) => setSubagents(c) || isCall(c, "config/write"),
        ),
      ).toBe(true);
    });

    expect(r.client.commands.filter(setSubagents)).toEqual([
      {
        type: "sessionTools",
        request: { op: "setSubagents", enabled: false },
      },
    ]);
    // No write names the setting, so the open chat's id is in no `config/write` for this switch.
    const writes = r.client.commands.filter((c) => isCall(c, "config/write"));
    expect(JSON.stringify(writes)).not.toContain("enable_subagents");
    await waitFor(() => expect(sw).toHaveAttribute("aria-checked", "false"));
    // The window starts and deletes no chat itself: the core does, out of sight.
    expect(
      r.client.commands.filter(
        (c) =>
          c.type === "startSession" ||
          (c.type === "sessionTools" && c.request.op === "startTemporary") ||
          isCall(c, "session/delete"),
      ),
    ).toEqual([]);
  });

  it("Background processes still writes through the open chat and reloads it", async () => {
    const r = await rig();
    const sw = await screen.findByRole("switch", {
      name: "Background processes",
    });
    await waitFor(() => expect(sw).toBeEnabled());

    await userEvent.setup().click(sw);
    await waitFor(() => {
      expect(
        r.client.commands.filter((c) => isCall(c, "config/write")),
      ).toHaveLength(1);
    });

    expect(r.client.commands.filter((c) => isCall(c, "config/write"))).toEqual([
      {
        type: "call",
        method: "config/write",
        params: {
          sessionId: CHAT,
          ops: [
            { op: "set", path: "/enable_background_processes", value: true },
          ],
          reason: "background processes",
          reloadRuntime: true,
        },
      },
    ]);
    expect(r.client.commands.filter(setSubagents)).toEqual([]);
    expect(
      within(row("subagents")).queryByText(/Saved for new chats/),
    ).toBeNull();
  });

  it("says the change is for new chats and shows the value the core reported", async () => {
    // The open chat's own read still answers the old value (true); the core reported what a new chat gets.
    const r = await rig({ subagentsRead: true });
    const sw = await subagentsSwitch();
    await userEvent.setup().click(sw);
    await waitFor(() =>
      expect(
        within(row("subagents")).getByText(/Saved for new chats/),
      ).toBeInTheDocument(),
    );
    expect(
      within(row("subagents")).getByText(
        "Saved for new chats. Sessions you already have open keep the setting they started with.",
      ),
    ).toBeInTheDocument();
    expect(sw).toHaveAttribute("aria-checked", "false");
    expect(r.script.subagentsRead).toBe(true);
  });

  it("when the core could not read the value back, the re-read decides", async () => {
    await rig({
      subagentsRead: false,
      setAnswer: { enabled: null, leftoverSessionId: null },
    });
    const sw = await subagentsSwitch();
    expect(sw).toHaveAttribute("aria-checked", "false");
    await userEvent.setup().click(sw);
    await waitFor(() =>
      expect(
        within(row("subagents")).getByText(/Saved for new chats/),
      ).toBeInTheDocument(),
    );
    // The flip asked for "on"; the re-read through the open chat still says off, and that is what is shown.
    expect(sw).toHaveAttribute("aria-checked", "false");
  });

  it("a change Vibe refuses shows its error, no note, and the switch keeps its value", async () => {
    const r = await rig({ setAnswer: refusal });
    const sw = await subagentsSwitch();
    await userEvent.setup().click(sw);
    expect(
      await screen.findByText("Vibe did not accept the change."),
    ).toBeInTheDocument();
    expect(sw).toHaveAttribute("aria-checked", "true");
    expect(
      within(row("subagents")).queryByText(/Saved for new chats/),
    ).toBeNull();
    expect(
      JSON.stringify(
        r.client.commands.filter((c) => isCall(c, "config/write")),
      ),
    ).not.toContain("enable_subagents");
  });

  it("a helper chat that could not be deleted is shown as a notification", async () => {
    const r = await rig({
      setAnswer: { enabled: false, leftoverSessionId: "helper-1" },
    });
    const sw = await subagentsSwitch();
    const before = r.client.commands.filter(
      (c) => c.type === "sessionTools" && c.request.op === "temporary",
    ).length;
    await userEvent.setup().click(sw);
    await waitFor(() => {
      expect(r.app.toasts.store.get().map((t) => t.text)).toEqual([
        expect.stringContaining("could not be deleted"),
      ]);
    });
    const toast = r.app.toasts.store.get()[0];
    expect(toast?.kind).toBe("warn");
    expect(toast?.sessionId).toBe("helper-1");
    expect(toast?.text).toContain("deleted when Vibe Work quits");
    // The saved value is still shown, with the note, and the temporary list is read again so the chat is marked.
    expect(
      within(row("subagents")).getByText(/Saved for new chats/),
    ).toBeInTheDocument();
    await waitFor(() => {
      expect(
        r.client.commands.filter(
          (c) => c.type === "sessionTools" && c.request.op === "temporary",
        ).length,
      ).toBeGreaterThan(before);
    });
  });

  it("shows no note before any change", async () => {
    await rig();
    await subagentsSwitch();
    expect(
      within(row("subagents")).queryByText(/Saved for new chats/),
    ).toBeNull();
  });
});
