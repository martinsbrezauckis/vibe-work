/**
 * The vault keeps its last state: no "remember" choice when creating or unlocking the vault, and one Settings switch
 * "Lock the vault when Vibe Work quits" with the vault controls. Real controller and typed client over a fake core;
 * synthetic statuses only.
 */
import { act, render, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";

import { CreateStep } from "../../../ui/src/features/setup/CreateStep";
import { KeySettings } from "../../../ui/src/features/setup/KeySettings";
import { UnlockStep } from "../../../ui/src/features/setup/UnlockStep";
import { AppProvider } from "../../../ui/src/features/shell/AppContext";
import type { KeyStatusView } from "../../../ui/src/protocol/core";
import { renderShell } from "./support/renderShell";

function present<T>(value: T | null | undefined): T {
  if (value === null || value === undefined)
    throw new Error("Missing test fixture");
  return value;
}

async function setup(
  screen: "create" | "unlock" | "settings",
  vaultState: "noVault" | "locked" | "unlocked",
) {
  let lockOnQuit = false;
  const requests: Record<string, unknown>[] = [];
  const status = (): KeyStatusView => ({
    vaultState,
    vaultKind: vaultState === "noVault" ? null : "vibeWork",
    availableVaults: vaultState === "noVault" ? [] : ["vibeWork"],
    vaultSelected: vaultState !== "noVault",
    vaultPath: vaultState === "noVault" ? null : "/synthetic/vault",
    rememberedDevice: vaultState === "unlocked",
    lockVaultOnQuit: lockOnQuit,
    keyState: vaultState === "unlocked" ? "ready" : "unknown",
    keyItemId: vaultState === "unlocked" ? "0" : null,
    ownKey: vaultState === "unlocked",
    activeKey: null,
    rememberNote: null,
  });
  const rig = await renderShell({
    setup(client) {
      client.answerWith("keys", (command) => {
        if (command.type !== "keys") throw new Error("wrong command");
        const r = command.request as unknown as Record<string, unknown>;
        requests.push(r);
        if (r.op === "readStatus")
          return { type: "keyStatus", status: status() };
        if (r.op === "listKeyCandidates")
          return { type: "keyCandidates", candidates: [] };
        if (r.op === "listMistralKeys")
          return { type: "mistralKeys", keys: [] };
        if (r.op === "detectVibeKey") return { type: "vibeKeys", found: [] };
        if (r.op === "setLockVaultOnQuit") lockOnQuit = r.on === true;
        return { type: "done" };
      });
    },
  });
  await act(() => present(rig.app.keys).start());
  const view = render(
    <AppProvider app={rig.app}>
      {screen === "create" && <CreateStep />}
      {screen === "unlock" && <UnlockStep />}
      {screen === "settings" && <KeySettings />}
    </AppProvider>,
  );
  return { part: within(view.container), user: userEvent.setup(), requests };
}

it("creating a vault offers no remember choice and sends none", async () => {
  const { part } = await setup("create", "noVault");
  expect(part.queryByRole("checkbox")).toBeNull();
  expect(part.queryByText(/Remember on this computer/)).toBeNull();
});

it("unlocking offers no remember choice and sends none", async () => {
  const { part, user, requests } = await setup("unlock", "locked");
  expect(part.queryByRole("checkbox")).toBeNull();
  expect(part.queryByText(/Remember on this computer/)).toBeNull();
  await user.type(part.getByLabelText("Vault password"), "synthetic password");
  await user.click(part.getByRole("button", { name: "Unlock" }));
  const unlock = requests.find((r) => r.op === "unlockVault");
  expect(unlock).toBeDefined();
  expect(unlock).not.toHaveProperty("rememberDevice");
});

it("Settings has the lock-at-quit switch, off by default, and no remember or forget action", async () => {
  const { part, user, requests } = await setup("settings", "unlocked");
  expect(part.queryByRole("button", { name: /this computer/ })).toBeNull();
  const row = present(
    part
      .getByText("Lock the vault when Vibe Work quits")
      .closest("[data-setting]"),
  );
  expect(row).toHaveAttribute("data-setting", "vault-lock-on-quit");
  expect(row).toHaveTextContent(
    "When off, an unlocked vault stays unlocked after a restart: its key is kept in this computer's credential store.",
  );
  const toggle = part.getByRole("switch", {
    name: "Lock the vault when Vibe Work quits",
  });
  expect(toggle).toHaveAttribute("aria-checked", "false");
  await user.click(toggle);
  expect(requests).toContainEqual({ op: "setLockVaultOnQuit", on: true });
  expect(
    await part.findByRole("switch", {
      name: "Lock the vault when Vibe Work quits",
      checked: true,
    }),
  ).toBeInTheDocument();
});

it("the switch shows while the vault is locked too", async () => {
  const { part } = await setup("settings", "locked");
  expect(
    part.getByRole("switch", { name: "Lock the vault when Vibe Work quits" }),
  ).toBeInTheDocument();
});
