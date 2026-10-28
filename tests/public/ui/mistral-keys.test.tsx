/** Account key rows with synthetic metadata; real controller, typed client and focus behavior. */
import { act, render, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";

import { KeySettings } from "../../../ui/src/features/setup/KeySettings";
import { MistralKeys } from "../../../ui/src/features/settings/MistralKeys";
import { AppProvider } from "../../../ui/src/features/shell/AppContext";
import type { CommandResult, CoreError } from "../../../ui/src/protocol/core";
import { renderShell } from "./support/renderShell";

function present<T>(value: T | null | undefined): T {
  if (value === null || value === undefined)
    throw new Error("Missing test fixture");
  return value;
}

async function setup(locked = false, count = 2) {
  let rows = Array.from({ length: count }, (_, i) => ({
    itemId: String(i),
    label: i === 0 ? "Personal key" : "Work key",
    inUse: i === 0,
    lastTest: { modelCount: 54 + i, code: null, text: null },
  }));
  let decline = false;
  const requests: unknown[] = [];
  const rig = await renderShell({
    setup(client) {
      client.answerWith("keys", (command) => {
        if (command.type !== "keys") throw new Error("wrong command");
        const r = command.request;
        requests.push(r);
        const op: string = r.op;
        const input = r as unknown as { itemId: string; name: string };
        if (op === "readStatus")
          return {
            type: "keyStatus",
            status: {
              vaultState: locked ? "locked" : "unlocked",
              vaultKind: "vibeWork",
              availableVaults: ["vibeWork"],
              vaultSelected: true,
              vaultPath: "/synthetic/vault",
              rememberedDevice: false,
              lockVaultOnQuit: false,
              keyState: "ready",
              keyItemId: "0",
              ownKey: true,
              activeKey: {
                label: rows.find((k) => k.inUse)?.label ?? "",
                source: "vault",
                addedMs: 1,
              },
              rememberNote: null,
            },
          };
        if (op === "listKeyCandidates")
          return { type: "keyCandidates", candidates: [] };
        if (op === "listMistralKeys")
          return {
            type: "mistralKeys",
            keys: rows,
          } as unknown as CommandResult;
        if (op === "useMistralKey") {
          if (decline)
            return {
              code: "confirmationDeclined",
              message: "The Mistral key was kept.",
              method: null,
              summary: null,
              serverCode: null,
            } satisfies CoreError;
          rows = rows.map((k) => ({ ...k, inUse: k.itemId === input.itemId }));
        }
        if (op === "renameMistralKey")
          rows = rows.map((k) =>
            k.itemId === input.itemId ? { ...k, label: input.name } : k,
          );
        if (op === "removeMistralKey")
          rows = rows.filter((k) => k.itemId !== input.itemId);
        if (op === "addMistralKey")
          rows.push({
            itemId: "new",
            label: input.name,
            inUse: false,
            lastTest: { modelCount: 61, code: null, text: null },
          });
        return { type: "done" };
      });
    },
  });
  await act(() => present(rig.app.keys).start());
  const view = render(
    <AppProvider app={rig.app}>
      <MistralKeys />
    </AppProvider>,
  );
  return {
    ...rig,
    section: within(view.container),
    user: userEvent.setup(),
    requests,
    decline: () => {
      decline = true;
    },
    rows: () => rows,
  };
}

it("shows metadata, in-use badge, exact copy and the status tooltip", async () => {
  const { section, view, app } = await setup();
  expect(await section.findByText("Personal key")).toBeInTheDocument();
  expect(section.getByText("In use")).toBeInTheDocument();
  // No explaining line: the list is the whole section.
  expect(section.queryByText(/Keep a key for each/)).toBeNull();
  expect(section.getAllByText("Key works").length).toBeGreaterThan(0);
  expect(section.getAllByRole("button", { name: "Remove…" })).toHaveLength(1);
  act(() => {
    present(app.keys).store.set((s) => ({
      ...s,
      test: {
        phase: "failed",
        reason: "unauthorized",
        text: "Synthetic test failure",
      },
    }));
  });
  expect(view.getByTestId("statusbar-key")).toHaveAttribute(
    "title",
    "Mistral key: Personal key",
  );
});

it("one key can add a named second key without using it; an empty list defaults to Personal key", async () => {
  const { section, user, requests } = await setup(false, 1);
  await user.click(section.getByRole("button", { name: "Add key" }));
  expect(section.getByRole("textbox", { name: "Name" })).toHaveValue(
    "Work key",
  );
  await user.type(
    section.getByLabelText("Paste your Mistral API key"),
    "synthetic-additional-key",
  );
  await user.click(
    section.getByRole("button", { name: "Save and test the key" }),
  );
  expect(await section.findByText("Work key")).toBeInTheDocument();
  expect(section.getAllByText("Key works").length).toBeGreaterThan(0);
  expect(requests).toContainEqual({
    op: "addMistralKey",
    name: "Work key",
    apiKey: "synthetic-additional-key",
  });
  expect(requests).not.toContainEqual(
    expect.objectContaining({ op: "setupStoreKey" }),
  );
});

it("switches with a toast and leaves focus on the row", async () => {
  const { section, user, app } = await setup();
  await user.click(
    await section.findByRole("button", { name: "Use this key" }),
  );
  await waitFor(() => {
    expect(app.toasts.store.get().at(-1)?.text).toBe("Now using “Work key”.");
    expect(app.toasts.history.get().at(-1)?.text).toBe("Now using “Work key”.");
  });
  await waitFor(() =>
    expect(
      document.activeElement?.closest("[data-mistral-key]"),
    ).toHaveTextContent("Work key"),
  );
});

it("decline keeps the current key and uses the specified toast", async () => {
  const { section, user, decline, rows, app } = await setup();
  decline();
  await user.click(
    await section.findByRole("button", { name: "Use this key" }),
  );
  await waitFor(() => {
    expect(app.toasts.store.get().at(-1)?.text).toBe(
      "The Mistral key was kept.",
    );
  });
  expect(rows()[0]?.inUse).toBe(true);
});

it("busy disables use with its reason; switching labels the row", async () => {
  const { section, app } = await setup();
  act(() => {
    present(app.core).store.set((s) => ({
      ...s,
      sessions: {
        busy: {
          id: "busy",
          session: { status: "running" },
          state: null,
          localOnly: true,
          openCallbackIds: [],
        },
      },
    }));
  });
  const button = await section.findByRole("button", { name: "Use this key" });
  expect(button).toBeDisabled();
  expect(button).toHaveAttribute(
    "title",
    "Wait until the chats finish, then switch keys.",
  );
});

it("locked names the vault action and shows no rows", async () => {
  const { section } = await setup(true);
  expect(
    section.getByText("Unlock your vault to see your Mistral keys."),
  ).toBeInTheDocument();
  expect(section.getByRole("button", { name: "Unlock" })).toBeInTheDocument();
  expect(section.queryByText("Personal key")).toBeNull();
});

it("rename is inline, Escape cancels, removal asks and focuses Add after the last row", async () => {
  const { section, user } = await setup();
  await user.click(
    present((await section.findAllByRole("button", { name: "Rename" }))[1]),
  );
  const name = section.getByRole("textbox", { name: "Name" });
  await user.clear(name);
  await user.type(name, "Company{Enter}");
  expect(await section.findByText("Company")).toBeInTheDocument();
  await user.click(section.getByRole("button", { name: "Remove…" }));
  expect(document.querySelector('[role="dialog"]')).toHaveTextContent(
    "Remove the key “Company” from your vault? Vibe Work can no longer use it. The key itself stays valid at Mistral.",
  );
  await user.keyboard("{Escape}");
  expect(section.getByText("Company")).toBeInTheDocument();
  await user.click(section.getByRole("button", { name: "Remove…" }));
  await user.click(
    within(present(document.querySelector('[role="dialog"]'))).getByRole(
      "button",
      {
        name: "Remove",
      },
    ),
  );
  await waitFor(() => {
    expect(section.queryByText("Company")).toBeNull();
  });
  await waitFor(() =>
    expect(section.getByRole("button", { name: "Add key" })).toHaveFocus(),
  );
});

it("empty vault defaults to Personal key; many keys and failed tests remain manageable", async () => {
  const { section, user, app } = await setup(false, 0);
  await user.click(section.getByRole("button", { name: "Add key" }));
  expect(section.getByRole("textbox", { name: "Name" })).toHaveValue(
    "Personal key",
  );
  await user.keyboard("{Escape}");
  act(() => {
    present(app.keys).store.set((s) => ({
      ...s,
      mistralKeys: Array.from({ length: 30 }, (_, i) => ({
        itemId: String(i),
        label:
          i === 1
            ? "Long workspace name ".repeat(20)
            : `Workspace ${String(i)}`,
        inUse: i === 0,
        lastTest:
          i === 1
            ? {
                modelCount: null,
                code: 401,
                text: "Mistral did not accept this key (401). Check that it is complete and not revoked.",
              }
            : null,
      })),
    }));
  });
  expect(
    section.getByText(
      "Mistral answered 401: Mistral did not accept this key (401). Check that it is complete and not revoked.",
    ),
  ).toBeInTheDocument();
  expect(section.getAllByRole("button", { name: "Use this key" })).toHaveLength(
    29,
  );
  expect(section.getAllByRole("button", { name: "Remove…" })).toHaveLength(29);
});

it("switching shows Switching… until the native answer arrives", async () => {
  const { section, app, user } = await setup();
  const original = present(app.core).run.bind(present(app.core));
  let release: (() => void) | undefined;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  vi.spyOn(present(app.core), "run").mockImplementation(async (command) => {
    if (command.type === "keys" && command.request.op === "useMistralKey")
      await pending;
    return original(command);
  });
  await user.click(
    await section.findByRole("button", { name: "Use this key" }),
  );
  expect(section.getByRole("button", { name: "Switching…" })).toBeDisabled();
  await act(async () => {
    present(release)();
    await pending;
  });
  await waitFor(() => {
    expect(section.queryByText("Switching…")).toBeNull();
  });
});

it("Account never offers removal of the active credential", async () => {
  const { app } = await setup(false, 1);
  const view = render(
    <AppProvider app={app}>
      <KeySettings />
    </AppProvider>,
  );
  expect(
    within(view.container).queryByRole("button", { name: "Remove key" }),
  ).toBeNull();
});
