/** Skills manager state and action contracts over a fake core; no registry traffic. */
import {
  act,
  fireEvent,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { SkillRow, SkillsView } from "../../../ui/src/protocol/core";
import { menuItems } from "../../../ui/src/features/composer/model/slash";
import { snapshot } from "./support/fakeClient";
import { renderShell, type Rig } from "./support/renderShell";

const builtin: SkillRow = {
  name: "worktree",
  description: "Work in a separate copy of the repository.",
  source: "builtin",
  installed: true,
  enabled: true,
  locked: false,
  lockReason: null,
  scope: "builtin",
  skillId: null,
  installedVersion: null,
  latestVersion: null,
  updateAvailable: false,
};
const locked: SkillRow = {
  ...builtin,
  name: "review-helper:review",
  source: "plugin",
  enabled: false,
  locked: true,
  lockReason: "Set by plugin review-helper",
};
const registry: SkillRow = {
  ...builtin,
  name: "release-notes",
  source: "user",
  skillId: "release-id",
  scope: "global",
  installedVersion: 2,
  latestVersion: 3,
  updateAvailable: true,
};
const catalog: SkillRow = {
  ...builtin,
  name: "data-analysis",
  description: "Analyse a CSV or spreadsheet.",
  source: "catalog",
  installed: false,
  enabled: null,
  skillId: "data-id",
  scope: null,
  latestVersion: 4,
};
function view(patch: Partial<SkillsView> = {}): SkillsView {
  return {
    skills: [builtin, locked, registry, catalog],
    issues: [],
    authenticated: true,
    catalogLoaded: true,
    busy: false,
    ...patch,
  };
}
/** `fail`: every change is refused by Vibe (`true`) or cancelled in the native dialog (`"declined"`). */
async function setup(
  initial = view(),
  fail: boolean | "declined" = false,
): Promise<Rig> {
  let current = initial;
  const r = await renderShell({
    setup(client) {
      client.answerWith("skills", (c) => {
        if (c.type !== "skills") throw new Error("Wrong command");
        const a = c.request;
        if (a.op === "detail")
          return {
            type: "skills",
            result: {
              kind: "detail",
              detail: {
                name: a.name,
                description: "Full description",
                body: "Full skill instructions",
                versions: ["v2", "v3"],
                locations: ["Registry · Global"],
              },
            },
          };
        // One-click installs: Install… / Update to v{n}… first read the offer the click confirms.
        if (a.op === "offerImport" || a.op === "offerUpdate")
          return {
            type: "skills",
            result: {
              kind: "offer",
              offer: {
                offerId: `offer-${a.op}`,
                kind: "skill",
                name:
                  a.op === "offerImport" ? "data-analysis" : "release-notes",
                publisher: null,
                source: "Mistral skill registry",
                licence: null,
                runs: "Instructions the agent may follow in every project. Installing runs no program.",
                version: a.op === "offerImport" ? 4 : 3,
                expiresAt: Date.now() + 600_000,
              },
            },
          };
        if (fail === "declined" && a.op !== "read")
          return {
            code: "confirmationDeclined",
            message: "skills/import: declined by the user",
            method: "skills/import",
            summary: null,
            serverCode: null,
          };
        if (fail && a.op !== "read")
          return {
            code: "serverError",
            message: "Vibe refused this change",
            method: null,
            summary: null,
            serverCode: null,
          };
        if (a.op === "setEnabled")
          current = {
            ...current,
            skills: current.skills.map((s) =>
              s.name === a.name ? { ...s, enabled: a.enabled } : s,
            ),
          };
        if (a.op === "import")
          current = {
            ...current,
            skills: current.skills.map((s) =>
              s.skillId === a.skillId
                ? {
                    ...s,
                    installed: true,
                    source: "user",
                    enabled: true,
                    scope: "global",
                    installedVersion: s.latestVersion,
                  }
                : s,
            ),
          };
        if (a.op === "remove")
          current = {
            ...current,
            skills: current.skills.map((s) =>
              s.name === a.name
                ? {
                    ...s,
                    installed: false,
                    source: "catalog",
                    enabled: null,
                    scope: null,
                    installedVersion: null,
                    updateAvailable: false,
                  }
                : s,
            ),
          };
        if (a.op === "setLatest")
          current = {
            ...current,
            skills: current.skills.map((s) =>
              s.name === a.name
                ? {
                    ...s,
                    installedVersion: s.latestVersion,
                    updateAvailable: false,
                  }
                : s,
            ),
          };
        return { type: "skills", result: { kind: "view", view: current } };
      });
    },
  });
  await act(async () => {
    await r.app.market?.actions.showTab("skills");
  });
  await waitFor(() => {
    expect(r.app.skills?.store.get().view).not.toBeNull();
  });
  return r;
}
function article(name: string) {
  return screen.getByRole("button", { name }).closest("article") as HTMLElement;
}

describe("Skills", () => {
  it("provides the third Tools tab, exact text, groups, sources, palette and disclosure", async () => {
    const r = await setup();
    expect(screen.getByRole("tab", { name: "Skills" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(
      screen.getByText(
        "Skills Vibe can use in chats. Changes apply to the selected chat and new chats in this folder.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "On this computer · 3" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Mistral's catalog · 1" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Built-in")).toBeInTheDocument();
    const command = r.app.commands
      .list()
      .find((c) => c.label === "Open Skills");
    expect(command).toBeDefined();
    act(() => {
      command?.run();
    });
    fireEvent.click(screen.getByRole("button", { name: "release-notes" }));
    expect(
      await screen.findByText("Full skill instructions"),
    ).toBeInTheDocument();
    expect(screen.getByText("Versions: v2, v3")).toBeInTheDocument();
    expect(screen.getByText("Registry · Global")).toBeInTheDocument();
  });

  it("shows loading, read error and retry", async () => {
    const r = await setup();
    act(() =>
      r.app.skills?.store.set((s) => ({ ...s, view: null, loading: true })),
    );
    expect(screen.getAllByLabelText("Loading skills")).toHaveLength(2);
    expect(document.querySelectorAll(".skills-loading .sk")).toHaveLength(6);
    act(() =>
      r.app.skills?.store.set((s) => ({
        ...s,
        loading: false,
        error: "transport closed",
      })),
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Vibe could not list its skills: transport closed.",
    );
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(
      await screen.findByRole("switch", { name: "Use worktree" }),
    ).toBeEnabled();
  });

  it.each([false, true])(
    "keeps installed skills with an empty or unavailable catalog (loaded %s)",
    async (loaded) => {
      await setup(view({ skills: [builtin], catalogLoaded: loaded }));
      expect(
        screen.getByText(
          loaded
            ? "Mistral's catalog has no skills to install right now."
            : "Mistral's catalog is not available right now.",
        ),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("switch", { name: "Use worktree" }),
      ).toBeEnabled();
    },
  );

  it("shows no-key Account link and hides Install while installed skills work", async () => {
    const r = await setup(view({ authenticated: false }));
    expect(
      screen.getByText(/Add your Mistral key to install skills/),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Install…" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Use worktree" })).toBeEnabled();
    fireEvent.click(
      screen.getByRole("button", { name: "Open Account settings" }),
    );
    expect(r.app.shell.store.get().settings.category).toBe("account");
  });

  it("disables every mutation while busy but leaves disclosures enabled", async () => {
    await setup(view({ busy: true }));
    expect(
      screen.getByText(
        "Wait until the chats in this folder finish, then change skills.",
      ),
    ).toBeInTheDocument();
    for (const control of [
      ...within(screen.getByTestId("skills-page")).getAllByRole("switch"),
      screen.getByRole("button", { name: "Install…" }),
      screen.getByRole("button", { name: "Update to v3…" }),
      screen.getByRole("button", { name: "Remove…" }),
    ])
      expect(control).toBeDisabled();
    expect(screen.getByRole("button", { name: "worktree" })).toBeEnabled();
  });

  it("shows plugin and configuration locks and puts issues only at the top", async () => {
    await setup(
      view({
        skills: [
          builtin,
          locked,
          {
            ...builtin,
            name: "house-rules",
            locked: true,
            lockReason: "Set in Vibe's settings file",
            source: "project",
          },
        ],
        issues: [{ file: "config.toml", message: "Invalid skill entry" }],
      }),
    );
    const control = within(article(locked.name)).getByRole("switch");
    expect(control).toBeDisabled();
    expect(control).toHaveAttribute("title", "Set by plugin review-helper");
    expect(screen.getByText("Set in Vibe's settings file")).toBeInTheDocument();
    expect(
      screen.getByText("config.toml: Invalid skill entry").closest("article"),
    ).toBeNull();
  });

  it("reverts an optimistic switch and announces the returned row error", async () => {
    const r = await setup(view(), true);
    let changed: Promise<boolean> | undefined;
    act(() => {
      changed = r.app.skills?.actions.change(
        { op: "setEnabled", name: "worktree", enabled: false },
        "worktree",
      );
    });
    expect(
      screen.getByRole("switch", { name: "Use worktree" }),
    ).toHaveAttribute("aria-checked", "false");
    await act(async () => {
      await changed;
    });
    expect(
      screen.getByRole("switch", { name: "Use worktree" }),
    ).toHaveAttribute("aria-checked", "true");
    expect(within(article("worktree")).getByRole("alert")).toHaveTextContent(
      "Vibe refused this change",
    );
  });

  it("shows the row error when removal fails", async () => {
    await setup(view(), true);
    fireEvent.click(
      within(article("release-notes")).getByRole("button", { name: "Remove…" }),
    );
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Remove",
      }),
    );
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
    expect(
      within(article("release-notes")).getByRole("alert"),
    ).toHaveTextContent("Vibe refused this change");
  });

  it("Cancel on the install offer installs nothing and leaves no row error", async () => {
    const r = await setup();
    fireEvent.click(screen.getByRole("button", { name: "Install…" }));
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    expect(
      r.client.commands.filter(
        (c) => c.type === "skills" && c.request.op === "import",
      ),
    ).toHaveLength(0);
    expect(
      within(article("data-analysis")).queryByRole("alert"),
    ).not.toBeInTheDocument();
    expect(
      within(article("data-analysis")).getByRole("button", {
        name: "Install…",
      }),
    ).toBeEnabled();
  });

  it("toggles with Space and preserves success on reopening", async () => {
    const r = await setup();
    const user = userEvent.setup();
    screen.getByRole("switch", { name: "Use worktree" }).focus();
    await user.keyboard(" ");
    await waitFor(() =>
      expect(
        screen.getByRole("switch", { name: "Use worktree" }),
      ).toHaveAttribute("aria-checked", "false"),
    );
    await act(async () => {
      await r.app.market?.actions.showTab("connectors");
      await r.app.market?.actions.showTab("skills");
    });
    expect(
      await screen.findByRole("switch", { name: "Use worktree" }),
    ).toHaveAttribute("aria-checked", "false");
  });

  it("installs globally, updates with catalog evidence, removes with Cancel focus and restores row focus", async () => {
    const r = await setup();
    const user = userEvent.setup();
    fireEvent.click(screen.getByRole("button", { name: "Install…" }));
    fireEvent.click(await screen.findByRole("button", { name: "Install v4" }));
    await screen.findByText("Installed data-analysis.");
    expect(screen.getByRole("button", { name: "data-analysis" })).toHaveFocus();
    fireEvent.click(screen.getByRole("button", { name: "Update to v3…" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "Update to v3" }),
    );
    expect(
      await screen.findByText("Updated release-notes to v3."),
    ).toBeInTheDocument();
    fireEvent.click(
      within(article("release-notes")).getByRole("button", { name: "Remove…" }),
    );
    const dialog = screen.getByRole("dialog", {
      name: "Remove release-notes?",
    });
    expect(
      within(dialog).getByText("Vibe stops offering it."),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByRole("button", { name: "Cancel" }),
    ).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(
      r.client.commands.filter(
        (c) => c.type === "skills" && c.request.op === "remove",
      ),
    ).toHaveLength(0);
    fireEvent.click(
      within(article("release-notes")).getByRole("button", { name: "Remove…" }),
    );
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    fireEvent.click(
      within(article("release-notes")).getByRole("button", { name: "Remove…" }),
    );
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Remove",
      }),
    );
    expect(
      await screen.findByText("Removed release-notes."),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "release-notes" })).toHaveFocus();
  });

  it("filters many long rows by name and description and does not infer updates", async () => {
    const rows = Array.from({ length: 10 }, (_, n) => ({
      ...builtin,
      name: `skill-${String(n)}`,
      description: `${"A long description. ".repeat(20)} ${n === 4 ? "spreadsheet" : "document"}`,
    }));
    await setup(
      view({ skills: [...rows, { ...registry, updateAvailable: false }] }),
    );
    expect(
      screen.queryByRole("button", { name: "Update to v3…" }),
    ).not.toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "Search skills" }), {
      target: { value: "spreadsheet" },
    });
    expect(screen.getByRole("button", { name: "skill-4" })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "skill-3" }),
    ).not.toBeInTheDocument();
  });

  it("offers Manage skills… as the final slash row and opens the tab from the composer", async () => {
    const r = await setup();
    const items =
      menuItems("/", [{ name: "review", description: "Review" }]) ?? [];
    expect(items.at(-1)?.kind).toBe("manage");
    act(() => {
      r.client.emit(snapshot("skills-chat"));
      r.app.shell.actions.selectSession("skills-chat");
      r.app.connectors?.actions.close();
    });
    const box = await r.composer();
    fireEvent.change(box, { target: { value: "/" } });
    const manage = await screen.findByRole("option", {
      name: "Manage skills…",
    });
    fireEvent.click(manage);
    await waitFor(() => {
      expect(screen.getByRole("tab", { name: "Skills" })).toHaveAttribute(
        "aria-selected",
        "true",
      );
    });
  });
});
