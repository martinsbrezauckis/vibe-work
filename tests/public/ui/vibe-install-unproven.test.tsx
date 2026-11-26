/**
 * A Vibe start refused because its interpreter cannot be guarded shows the refusal on the setup step.
 * Existing-install rows say the version is unknown when that guard refused before probing; other failed starts and
 * ordinary no-answer rows keep their wording.
 */
import { act, screen, within } from "@testing-library/react";
import { expect, test } from "vitest";

import type { InstallView } from "../../../ui/src/protocol/core";
import { announcerStore } from "../../../ui/src/state/announcer";
import { RUNNING_STATUS } from "./support/fakeClient";
import { renderShell } from "./support/renderShell";

const REFUSAL =
  "This Vibe installation cannot run with Vibe Work's safety guards: choose Install Vibe for Vibe Work, or a Vibe installed in its own Python environment.";

function failed(error: string): InstallView {
  return {
    pinnedVersion: "3.1.5",
    phase: {
      state: "failed",
      step: "Start Vibe",
      kind: "start",
      error,
      outputTail: "",
    },
    existing: [],
    checkedExisting: false,
    update: {
      available: null,
      upcoming: null,
      mismatch: null,
      checkedAt: null,
      checkError: null,
      phase: { state: "idle" },
    },
  };
}

async function show(error: string): Promise<void> {
  const rig = await renderShell({
    setup: (client) => {
      client.answer("readStatus", {
        type: "status",
        status: {
          ...RUNNING_STATUS,
          server: {
            ...RUNNING_STATUS.server,
            status: { state: "notInstalled", reason: error },
          },
        },
      });
      client.answerWith("vibe", () => ({
        type: "vibeInstall",
        view: failed(error),
      }));
    },
  });
  await act(async () => {
    await rig.app.vibe?.start();
  });
}

test("an unguardable Vibe start shows and announces its refusal once", async () => {
  await show(REFUSAL);
  expect(await screen.findByText(REFUSAL)).toBeVisible();
  expect(announcerStore.get().assertive).toBe(REFUSAL);
  expect(screen.queryByText(/Try again keeps the install/)).toBeNull();
});

test.each([
  "Vibe did not start: handshake failed",
  `${REFUSAL} Extra diagnostics.`,
  "",
])("another failed start keeps its existing summary (%s)", async (error) => {
  await show(error);
  expect(
    await screen.findByText(
      "Vibe was installed but did not start. Try again keeps the install and only starts Vibe again.",
    ),
  ).toBeVisible();
});

test("an unguardable existing install has an unknown version and cannot be selected", async () => {
  const view: InstallView = {
    ...failed(""),
    phase: { state: "notInstalled" },
    checkedExisting: true,
    existing: [
      {
        path: "/opt/unguarded/vibe-app-server",
        version: null,
        supported: false,
        warning: REFUSAL,
      },
      {
        path: "/opt/unanswered/vibe-app-server",
        version: null,
        supported: false,
        warning: "This install did not answer within 60 seconds.",
      },
    ],
  };
  const rig = await renderShell({
    setup: (client) => {
      client.answer("readStatus", {
        type: "status",
        status: {
          ...RUNNING_STATUS,
          server: {
            ...RUNNING_STATUS.server,
            status: {
              state: "notInstalled",
              reason: "Vibe is not installed yet",
            },
          },
        },
      });
      client.answerWith("vibe", () => ({ type: "vibeInstall", view }));
    },
  });
  await act(async () => {
    await rig.app.vibe?.start();
  });
  const guarded = screen
    .getByText("/opt/unguarded/vibe-app-server")
    .closest("li");
  const unanswered = screen
    .getByText("/opt/unanswered/vibe-app-server")
    .closest("li");
  if (guarded === null || unanswered === null)
    throw new Error("existing row missing");
  expect(within(guarded).getByText("(version unknown)")).toBeVisible();
  expect(within(guarded).getByText(REFUSAL)).toBeVisible();
  expect(
    within(guarded).queryByRole("button", { name: "Use this one" }),
  ).toBeNull();
  expect(within(unanswered).getByText("(did not answer)")).toBeVisible();
});
