/** Main-window reload keys use the selected Browser page and never reload the shell. Uses the rendered shell and typed fake core. */
import { act, cleanup, waitFor } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { registerPreviewTab } from "../../../ui/src/features/preview/register";
import type { PageTab, PreviewView } from "../../../ui/src/protocol/core";
import { snapshot } from "./support/fakeClient";
import { renderShell } from "./support/renderShell";

afterEach(cleanup);

async function rig(
  kind: "web" | "appPreview" | "empty",
  mac = false,
  state: PreviewView["state"] = "live",
) {
  const preview: PreviewView = {
    sessionId: "chat",
    url: kind === "empty" ? null : "http://127.0.0.1:28107/",
    title: "Game",
    state,
    server: null,
    loading: false,
    waitingForBrowser: false,
    width: "fit",
    downloads: [],
    refused: [],
    consoleCount: 0,
  };
  const tab: PageTab = {
    pageKey: kind === "web" ? "web-page" : "chat",
    kind: kind === "web" ? "web" : "appPreview",
    ownerKind: kind === "web" ? "user" : null,
    active: true,
    agentCurrent: false,
    preview,
    snapshot: null,
    faviconUrl: null,
  };
  const r = await renderShell({
    mac,
    setup(client) {
      client.answerWith("preview", (command) => {
        if (command.type !== "preview") throw Error("preview only");
        return command.request.op === "list"
          ? {
              type: "preview",
              result: {
                type: "previews",
                preview,
                targets: [],
                leasedPorts: [],
                busy: [],
              },
            }
          : { type: "preview", result: { type: "done" } };
      });
      client.answerWith("browser", (command) => {
        if (command.type !== "browser") throw Error("browser only");
        return command.request.op === "tabs"
          ? { type: "browser", result: { type: "tabs", tabs: [tab] } }
          : { type: "browser", result: { type: "done" } };
      });
    },
  });
  let stop = () => {};
  act(() => {
    stop = registerPreviewTab(r.app);
    r.client.emit(snapshot("chat"));
    r.app.shell.actions.selectSession("chat");
    r.app.shell.actions.openTab("browser");
  });
  await waitFor(() => {
    expect(
      r.client.commands.some(
        (c) => c.type === "browser" && c.request.op === "tabs",
      ),
    ).toBe(true);
  });
  await act(async () => {});
  return { ...r, stop };
}

const keys = [
  { key: "F5", code: "F5" },
  { key: "r", code: "KeyR", ctrlKey: true },
];

function press(
  key: KeyboardEventInit,
  target: EventTarget = window,
): KeyboardEvent {
  const event = new KeyboardEvent("keydown", {
    ...key,
    bubbles: true,
    cancelable: true,
  });
  act(() => {
    target.dispatchEvent(event);
  });
  return event;
}

it.each(keys)(
  "reloads the selected web tab and consumes $code",
  async (key) => {
    const r = await rig("web");
    expect(press(key).defaultPrevented).toBe(true);
    await waitFor(() => {
      expect(r.client.commands).toContainEqual({
        type: "browser",
        request: {
          op: "act",
          sessionId: "chat",
          tabId: "web-page",
          args: { action: "reload" },
        },
      });
    });
    r.stop();
  },
);

it("reloads the App preview with the toolbar's action, including Cmd+R on macOS", async () => {
  const r = await rig("appPreview", true);
  expect(
    press({ key: "r", code: "KeyR", metaKey: true }).defaultPrevented,
  ).toBe(true);
  await waitFor(() => {
    expect(r.client.commands).toContainEqual({
      type: "preview",
      request: { op: "reload", sessionId: "chat" },
    });
  });
  r.stop();
});

it("reloads a reopened page before the native visibility snapshot is refreshed", async () => {
  const r = await rig("web", false, "hidden");
  expect(press({ key: "F5", code: "F5" }).defaultPrevented).toBe(true);
  await act(async () => {});
  expect(r.client.commands).toContainEqual({
    type: "browser",
    request: {
      op: "act",
      sessionId: "chat",
      tabId: "web-page",
      args: { action: "reload" },
    },
  });
  r.stop();
});

it.each([
  ...keys,
  { key: "R", code: "KeyR", ctrlKey: true, shiftKey: true },
  { key: "F5", code: "F5", ctrlKey: true },
])(
  "keeps $code from reloading the window in a terminal, which still receives it",
  async (key) => {
    const r = await rig("web");
    const terminal = document.createElement("div");
    terminal.className = "vw-terminal";
    const input = document.createElement("textarea");
    terminal.append(input);
    document.body.append(terminal);
    let reachedTerminal = false;
    input.addEventListener("keydown", () => {
      reachedTerminal = true;
    });
    const before = r.client.commands.length;
    expect(press(key, input).defaultPrevented).toBe(true);
    // The terminal's program gets the key (Ctrl+R searches the shell history); the page is not reloaded.
    expect(reachedTerminal).toBe(true);
    await act(async () => {});
    expect(
      r.client.commands
        .slice(before)
        .some(
          (c) =>
            (c.type === "preview" && c.request.op === "reload") ||
            (c.type === "browser" && c.request.op === "act"),
        ),
    ).toBe(false);
    terminal.remove();
    r.stop();
  },
);

it("consumes hard reloads and reload keys while the Browser panel is closed", async () => {
  const r = await rig("web");
  const before = r.client.commands.length;
  expect(
    press({ key: "R", code: "KeyR", ctrlKey: true, shiftKey: true })
      .defaultPrevented,
  ).toBe(true);
  expect(press({ key: "F5", code: "F5", ctrlKey: true }).defaultPrevented).toBe(
    true,
  );
  act(() => {
    r.app.shell.actions.openTab("file");
  });
  for (const key of keys) expect(press(key).defaultPrevented).toBe(true);
  await act(async () => {});
  expect(
    r.client.commands
      .slice(before)
      .some((c) => c.type === "browser" && c.request.op === "act"),
  ).toBe(false);
  r.stop();
});
