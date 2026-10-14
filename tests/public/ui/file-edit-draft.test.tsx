/**
 * File-tab regression tests: a quick edit keeps the version it started from and an unsaved
 * draft stays visible until the user decides what to do. Uses a synthetic core and disk.
 */
import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { registerFilesTab } from "../../../ui/src/features/files/register";
import type {
  Command,
  CommandResult,
  FileContent,
} from "../../../ui/src/protocol/core";
import { assistant, turn, user } from "./support/transcriptFixtures";
import type { FakeClient } from "./support/fakeClient";
import { transcriptRig } from "./support/transcriptRig";

/** The rig's session. */
const SESSION = "s1";

type Disk = Record<string, { text: string; fingerprint: string }>;
type WriteReq = { file: string; text: string; fingerprint: string };

function content(file: string, disk: Disk): FileContent {
  const entry = disk[file];
  if (entry === undefined) throw new Error(`no ${file}`);
  return {
    file,
    display: file,
    name: file,
    size: entry.text.length,
    fingerprint: entry.fingerprint,
    truncated: false,
    dataUrl: null,
    tooLarge: false,
    openClass: "passive",
    kind: "text",
    text: entry.text,
  };
}

async function rig() {
  const disk: Disk = {
    "a.txt": { text: "original A\n", fingerprint: "a1" },
    "b.txt": { text: "original B\n", fingerprint: "b1" },
  };
  const writes: WriteReq[] = [];
  const answer = (client: FakeClient) => {
    client.answerWith("files", (command: Command): CommandResult => {
      if (command.type !== "files") throw new Error("files only");
      const req = command.request;
      const result = (() => {
        switch (req.op) {
          case "resolve":
            return {
              kind: "resolved" as const,
              refs: req.paths.map((text) =>
                text in disk
                  ? {
                      text,
                      ok: true,
                      file: text,
                      display: text,
                      kind: null,
                      size: 10,
                    }
                  : {
                      text,
                      ok: false,
                      file: null,
                      display: null,
                      kind: null,
                      size: null,
                    },
              ),
            };
          case "read":
            return {
              kind: "content" as const,
              content: content(req.file, disk),
            };
          case "stat": {
            const entry = disk[req.file];
            return {
              kind: "stat" as const,
              exists: entry !== undefined,
              fingerprint: entry?.fingerprint ?? null,
            };
          }
          case "write": {
            writes.push({
              file: req.file,
              text: req.text,
              fingerprint: req.fingerprint,
            });
            const entry = disk[req.file];
            if (entry === undefined || entry.fingerprint !== req.fingerprint)
              return "stale" as const;
            disk[req.file] = {
              text: req.text,
              fingerprint: `${req.fingerprint}+`,
            };
            return {
              kind: "written" as const,
              fingerprint: `${req.fingerprint}+`,
              size: req.text.length,
            };
          }
          case "view":
          case "viewer":
            return { kind: "viewer" as const, viewer: null };
          default:
            return { kind: "done" as const };
        }
      })();
      if (result === "stale")
        // The fake client turns an error-shaped answer into the core's error (code `stale`).
        return {
          code: "stale",
          message: "the file changed on disk since it was opened",
          method: null,
          summary: null,
          serverCode: null,
        } as unknown as CommandResult;
      return { type: "files", result };
    });
  };
  const r = await transcriptRig(
    [
      user("u1", "t1", "edit please"),
      assistant("a1", "t1", "See `a.txt` and `b.txt`."),
    ],
    [turn("t1", "completed", 1, 2)],
    { shell: { setup: answer } },
  );
  act(() => {
    registerFilesTab(r.app);
  });
  return { ...r, disk, writes };
}

async function openRef(path: string) {
  const button = await waitFor(() => {
    const el = document.querySelector<HTMLElement>(
      `button[data-file-ref="${path}"]`,
    );
    if (el === null) throw new Error(`no ref ${path}`);
    return el;
  });
  fireEvent.click(button);
  await waitFor(() => {
    expect(document.querySelector(".file-name")?.textContent).toBe(path);
  });
}

async function startEditing(path: string, draft: string) {
  fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
  const box = await screen.findByLabelText(`Text of ${path}`);
  fireEvent.change(box, { target: { value: draft } });
  return box;
}

describe("quick edits keep their base and their draft", () => {
  it("an older draft is never saved over the agent's newer text", async () => {
    const r = await rig();
    await openRef("a.txt");
    await startEditing("a.txt", "user draft\n");
    // The agent rewrites the file; the tab's poll (every 1.5 s) loads the new version.
    r.disk["a.txt"] = { text: "agent text\n", fingerprint: "a2" };
    await waitFor(
      () => {
        expect(
          r.app.files?.store.get().tabs[SESSION]?.content?.fingerprint,
        ).toBe("a2");
      },
      { timeout: 5000 },
    );
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => {
      expect(r.writes).toHaveLength(1);
    });
    expect(r.writes[0]).toEqual({
      file: "a.txt",
      text: "user draft\n",
      fingerprint: "a1",
    });
    expect(r.disk["a.txt"].text).toBe("agent text\n");
    await screen.findByText(/changed on disk since you opened it/);
    expect(screen.getByLabelText("Text of a.txt")).toHaveValue("user draft\n");
  });

  it("opening another file keeps the unsaved draft and asks for a decision", async () => {
    const r = await rig();
    await openRef("a.txt");
    await startEditing("a.txt", "user draft\n");
    // Another file is opened from the chat while the draft is unsaved.
    const nextFile = document.querySelector<HTMLElement>(
      'button[data-file-ref="b.txt"]',
    );
    if (nextFile === null) throw new Error("no ref b.txt");
    fireEvent.click(nextFile);
    await waitFor(() => {
      expect(r.app.files?.store.get().tabs[SESSION]?.shown?.file).toBe("b.txt");
    });
    await screen.findByText(/You are still editing a\.txt/);
    expect(screen.getByLabelText("Text of a.txt")).toHaveValue("user draft\n");
    expect(
      screen.getByRole("button", { name: "Close the file" }),
    ).toBeDisabled();
    // Cancel is the user's decision: the draft goes, b.txt shows.
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => {
      expect(screen.queryByLabelText("Text of a.txt")).toBeNull();
    });
    expect(r.writes).toHaveLength(0);
  });
});
