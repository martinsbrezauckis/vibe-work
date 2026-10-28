/** Files search and tree behavior stay bounded and keep the current folder's answers. Uses the typed fake core. */
import { waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  nameMatches,
  createExplorer,
} from "../../../ui/src/features/explorer/store";
import { visibleRows } from "../../../ui/src/features/explorer/FileTree";
import { createApp } from "../../../ui/src/features/shell/createApp";
import { fakeClient } from "./support/fakeClient";
import type { CommandResult } from "../../../ui/src/protocol/core";

describe("Files explorer", () => {
  it("finds names with spaces and non-ASCII characters and limits a large index", () => {
    expect(nameMatches(["src/ābols file.rs", "README.md"], "āb frs")).toEqual([
      "src/ābols file.rs",
    ]);
    expect(
      nameMatches(
        Array.from({ length: 80_000 }, (_, n) => `src/file${String(n)}.rs`),
        "file",
      ),
    ).toHaveLength(200);
  });
  it("loads only the root, then only a folder opened by the person", async () => {
    const client = fakeClient();
    client.answerWith("gitBrowser", (command) => {
      if (command.type !== "gitBrowser" || command.request.op !== "explore")
        throw new Error("unexpected request");
      const r = command.request.request;
      return {
        type: "gitBrowser",
        result: {
          kind: "explorer",
          result:
            r.op === "index"
              ? { kind: "index", paths: ["src/main.rs"], truncated: false }
              : r.op === "folder"
                ? {
                    kind: "folder",
                    entries:
                      r.path === ""
                        ? [{ path: "src", directory: true }]
                        : [{ path: "src/main.rs", directory: false }],
                    ignored: [],
                    next: null,
                    status: null,
                  }
                : { kind: "done" },
        },
      };
    });
    const app = createApp({ client, persist: false });
    const core = app.core;
    if (!core) throw new Error("core");
    const explorer = createExplorer(core);
    explorer.choose("/work/a");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(
      client.commands.filter(
        (c) =>
          c.type === "gitBrowser" &&
          c.request.op === "explore" &&
          c.request.request.op === "folder",
      ),
    ).toHaveLength(1);
    expect(visibleRows(explorer.store.get()).map((r) => r.path)).toEqual([
      "src",
    ]);
    explorer.toggle("src");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(visibleRows(explorer.store.get()).map((r) => r.path)).toEqual([
      "src",
      "src/main.rs",
    ]);
    explorer.stop();
  });
  it("drops a slow answer after another folder is chosen", async () => {
    const client = fakeClient();
    let release: (v: CommandResult) => void = () => undefined;
    const old = new Promise<CommandResult>((resolve) => {
      release = resolve;
    });
    const app = createApp({ client, persist: false });
    const core = app.core;
    if (!core) throw new Error("core");
    const run = core.run.bind(core);
    core.run = async (command, ...args) =>
      command.type === "gitBrowser" &&
      command.request.op === "explore" &&
      command.request.folder === "/work/old" &&
      command.request.request.op === "index"
        ? old
        : run(command, ...args);
    client.answerWith("gitBrowser", () => ({
      type: "gitBrowser",
      result: {
        kind: "explorer",
        result: { kind: "index", paths: ["fresh.rs"], truncated: false },
      },
    }));
    const explorer = createExplorer(core);
    explorer.choose("/work/old");
    explorer.choose("/work/new");
    await new Promise((resolve) => setTimeout(resolve, 0));
    release({
      type: "gitBrowser",
      result: {
        kind: "explorer",
        result: { kind: "index", paths: ["old.rs"], truncated: false },
      },
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(explorer.store.get().paths).toEqual(["fresh.rs"]);
    explorer.stop();
  });
  it("shows source hits while commit search is still being read", async () => {
    const client = fakeClient();
    client.answerWith("gitBrowser", () => ({
      type: "gitBrowser",
      result: {
        kind: "explorer",
        result: {
          kind: "search",
          hits: [{ path: "calc.rs", line: 9, text: "multiply" }],
          truncated: false,
        },
      },
    }));
    const app = createApp({ client, persist: false }),
      core = app.core;
    if (!core) throw new Error("core");
    let release: (v: CommandResult) => void = () => undefined;
    const history = new Promise<CommandResult>((resolve) => {
        release = resolve;
      }),
      run = core.run.bind(core);
    core.run = async (command, ...args) =>
      command.type === "gitBrowser" &&
      command.request.op === "explore" &&
      command.request.request.op === "history"
        ? history
        : run(command, ...args);
    const explorer = createExplorer(core);
    explorer.store.set((s) => ({ ...s, folder: "/work/calculator" }));
    explorer.query("multiply");
    await waitFor(() => {
      expect(explorer.store.get().hits).toHaveLength(1);
    });
    expect(explorer.store.get().searching).toBe(true);
    release({
      type: "gitBrowser",
      result: {
        kind: "explorer",
        result: { kind: "history", versions: [], cursor: null },
      },
    });
    await waitFor(() => {
      expect(explorer.store.get().searching).toBe(false);
    });
    explorer.stop();
  });
  it("keeps a large plain folder visible without a changed-files filter", async () => {
    const client = fakeClient();
    client.answerWith("gitBrowser", (command) => {
      if (command.type !== "gitBrowser" || command.request.op !== "explore")
        throw new Error("request");
      return {
        type: "gitBrowser",
        result: {
          kind: "explorer",
          result:
            command.request.request.op === "index"
              ? {
                  kind: "index",
                  paths: Array.from(
                    { length: 6000 },
                    (_, n) => `file${String(n)}.txt`,
                  ),
                  truncated: false,
                }
              : {
                  kind: "status",
                  status: null,
                  note: null,
                  commits: null,
                  latest: null,
                },
        },
      };
    });
    const app = createApp({ client, persist: false }),
      core = app.core;
    if (!core) throw new Error("core");
    const explorer = createExplorer(core);
    explorer.choose("/work/plain");
    await waitFor(() => {
      expect(
        explorer.store.get().indexing || explorer.store.get().statusLoading,
      ).toBe(false);
    });
    expect(explorer.store.get().changedOnly).toBe(false);
    explorer.stop();
  });
});
