/** Security refusal grouping, retention, type filters and navigation through the real shell wiring. */
import { act, screen, within, waitFor } from "@testing-library/react";
import { announcerStore } from "../../../ui/src/state/announcer";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { CoreEvent } from "../../../ui/src/protocol/core";
import { snapshot } from "./support/fakeClient";
import { renderShell } from "./support/renderShell";

const refusal = (
  sessionId = "s1",
  guard = "git",
  stepId: string | null = "step-1",
  at = 1000,
): CoreEvent =>
  ({
    type: "guardRefused",
    sessionId,
    guard,
    what: "Not run: this action was refused",
    stepId,
    at,
    title: sessionId === "s1" ? "First chat" : "Second chat",
    refusalId: `${sessionId}-${guard}-${String(at)}`,
  }) as unknown as CoreEvent;
const bell = () => screen.getByRole("button", { name: "Notifications" });

describe("security bell", () => {
  it("groups by chat, counts guards, orders by activity and toasts only once per chat per run", async () => {
    const { app, client } = await renderShell();
    act(() => {
      client.emit(refusal());
      client.emit(refusal("s1", "folder", null, 2000));
      client.emit(refusal("s1", "git", "step-3", 3000));
    });
    expect(app.toasts.history.get()).toHaveLength(1);
    expect(app.toasts.history.get()[0]?.text).toBe(
      "Vibe Work stopped 3 actions in “First chat”",
    );
    expect(app.toasts.store.get()).toHaveLength(1);
    // The reason leads, so a narrow toast keeps it (a long chat title is cut instead).
    expect(app.toasts.store.get()[0]?.text).toBe(
      "Not run: this action was refused. In “First chat”.",
    );
    act(() => {
      client.emit(refusal("s2", "process", null, 4000));
      client.emit(refusal("s1", "network", null, 5000));
    });
    expect(app.toasts.store.get()).toHaveLength(2);
    await userEvent.setup().click(bell());
    const panel = screen.getByRole("dialog", { name: "Notifications" });
    const rows = panel.querySelectorAll("article");
    expect(rows[0]).toHaveTextContent("First chat");
    expect(rows[0]).toHaveTextContent(
      "Git guard 2 · Folder guard 1 · Network guard 1",
    );
    expect(app.toasts.history.get().every((n) => n.read)).toBe(true);
    act(() => {
      app.toasts.clear();
      client.emit(refusal("s1", "git", null, 6000));
    });
    expect(app.toasts.history.get()).toHaveLength(1);
    expect(app.toasts.store.get()).toHaveLength(0);
  });

  it("caps disclosure at 50 newest lines and offers step and chat links", async () => {
    const { app, client } = await renderShell();
    act(() => {
      for (let n = 0; n < 62; n++)
        client.emit(
          refusal("s1", "git", n === 61 ? null : `step-${String(n)}`, n),
        );
    });
    const user = userEvent.setup();
    await user.click(bell());
    await user.click(
      screen.getByRole("button", { name: /^Vibe Work stopped 62 actions/ }),
    );
    expect(screen.getAllByRole("button", { name: "Open step" })).toHaveLength(
      49,
    );
    expect(screen.getAllByRole("button", { name: "Open chat" })).toHaveLength(
      2,
    );
    expect(screen.getByText("and 12 more in this chat")).toBeInTheDocument();
    expect(app.toasts.history.get()).toHaveLength(1);
  });

  it("opens the owning chat and focuses a step inside collapsed steps", async () => {
    const { app, client } = await renderShell();
    const event = snapshot("s1");
    if (event.type !== "snapshot") throw new Error("snapshot");
    event.snapshot.state = {
      ...(event.snapshot.state as object),
      history: [
        {
          id: "step-1",
          type: "effect",
          generationStatus: "completed",
          detail: {
            kind: "shell",
            toolName: "bash",
            input: { command: "git push" },
            display: { verb: "Run", message: "git push" },
          },
          state: {
            status: "skipped",
            reason: "Not run: this action was refused",
          },
        },
      ],
    };
    act(() => {
      client.emit(event);
      client.emit(refusal());
    });
    const user = userEvent.setup();
    await user.click(bell());
    await user.click(
      screen.getByRole("button", { name: /^Vibe Work stopped an action/ }),
    );
    await user.click(screen.getByRole("button", { name: "Open step" }));
    await waitFor(() => {
      expect(app.shell.store.get().selected).toBe("s1");
      expect(document.querySelector('[data-entry-id="step-1"]')).toHaveFocus();
    });
    expect(document.activeElement).toHaveClass("find-current");
    expect(document.activeElement).toHaveAttribute("tabindex", "-1");
  });

  it("Show focuses the chat disclosure and the toast is said once by the announcer", async () => {
    const { client } = await renderShell();
    act(() => {
      client.emit(refusal());
    });
    const user = userEvent.setup();
    const show = screen.getByRole("button", { name: "Show" });
    const toast = show.closest(".toast");
    expect(toast).toBeInTheDocument();
    // The toast is no live region; the app's announcer says its words once.
    expect(toast).not.toHaveAttribute("role");
    const said = announcerStore.get().polite.trim();
    expect(said).not.toBe("");
    expect(toast?.textContent).toContain(said.slice(0, 20));
    await user.click(show);
    expect(
      screen.getByRole("button", { name: /^Vibe Work stopped an action/ }),
    ).toHaveFocus();
    expect(
      screen.getByRole("dialog", { name: "Notifications" }),
    ).not.toHaveAttribute("aria-live");
    await user.keyboard(" ");
    expect(
      screen.getByRole("button", { name: "Open step" }),
    ).toBeInTheDocument();
  });

  it("keeps no-step refusals navigable and exposes groups through the Debug API bell read", async () => {
    const { app, client } = await renderShell();
    act(() => {
      client.emit(snapshot("s1"));
      client.emit(refusal("s1", "safetyRule", null));
      client.emit(refusal("s1", "safetyRule", null));
    });
    const answer = await app.uiControl.handle({
      id: "bell-read",
      name: "notifications",
      body: {},
    });
    expect(answer).toMatchObject({
      ok: true,
      value: { bell: [{ security: { total: 1, counts: { safetyRule: 1 } } }] },
    });
    const user = userEvent.setup();
    await user.click(bell());
    await user.click(
      screen.getByRole("button", { name: /^Vibe Work stopped an action/ }),
    );
    expect(screen.queryByRole("button", { name: "Open step" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Open chat" }));
    await waitFor(() => {
      expect(app.shell.store.get().selected).toBe("s1");
    });
  });

  it("puts Show first in the bell's tab order and lets arrows choose a type", async () => {
    await renderShell();
    const user = userEvent.setup();
    await user.click(bell());
    await user.tab();
    expect(screen.getByRole("combobox", { name: "Show" })).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(
      screen.getByRole("dialog", { name: "Notifications" }),
    ).toContainElement(screen.getByRole("listbox", { name: "Show" }));
    await user.keyboard("{ArrowDown}{Enter}");
    expect(screen.getByRole("combobox", { name: "Show" })).toHaveTextContent(
      "Security",
    );
    expect(screen.getByText("No security entries.")).toBeInTheDocument();
  });

  it.each([
    ["security", "Security", "No security entries."],
    ["approval", "Approvals", "No approvals."],
    ["finished", "Finished", "No finished chats."],
    ["failure", "Failures", "No failures."],
    ["worktrees", "Session copies", "No session copies."],
  ] as const)(
    "filters %s and remembers the choice after closing",
    async (kind, label, empty) => {
      const { app } = await renderShell();
      const user = userEvent.setup();
      await user.click(bell());
      await user.click(screen.getByRole("combobox", { name: "Show" }));
      await user.click(screen.getByRole("option", { name: label }));
      expect(screen.getByText(empty)).toBeInTheDocument();
      act(() => {
        app.toasts.record({
          kind: "info",
          bellKind: kind,
          text: "Chosen type",
        });
        app.toasts.record({
          kind: "info",
          bellKind: kind === "failure" ? "approval" : "failure",
          text: "Other type",
        });
      });
      const panel = screen.getByRole("dialog", { name: "Notifications" });
      expect(within(panel).getByText("Chosen type")).toBeInTheDocument();
      expect(within(panel).queryByText("Other type")).toBeNull();
      await user.keyboard("{Escape}");
      await user.click(bell());
      expect(screen.getByRole("combobox", { name: "Show" })).toHaveTextContent(
        label,
      );
    },
  );
});
