/** Open documents retain independent drafts and a focused, bounded editor pair. Uses the typed fake core. */
import { describe, expect, it } from "vitest";
import { createApp } from "../../../ui/src/features/shell/createApp";
import { fakeClient } from "./support/fakeClient";
import type { Command, CommandResult } from "../../../ui/src/protocol/core";

function rig() {
  const client = fakeClient();
  client.answerWith("files", (command: Command): CommandResult => {
    if (command.type !== "files") throw Error("files only");
    const r = command.request;
    if (r.op === "read")
      return {
        type: "files",
        result: {
          kind: "content",
          content: {
            file: r.file,
            name: r.file,
            display: `src/${r.file}`,
            kind: "text",
            text: `original ${r.file}\n`,
            fingerprint: r.file,
            size: 12,
            truncated: false,
            tooLarge: false,
            dataUrl: null,
            openClass: "passive",
          },
        },
      };
    return { type: "files", result: { kind: "done" } };
  });
  const app = createApp({ client, persist: false });
  if (!app.files) throw Error("files needed");
  return app.files;
}
const shown = (file: string) => ({ file, display: `src/${file}`, line: null });

describe("open file documents", () => {
  it("keeps each unsaved buffer when another file opens", async () => {
    const files = rig();
    await files.actions.showForEdit("s1", shown("a.ts"));
    files.edit.update("s1", { text: "my A" });
    await files.actions.showForEdit("s1", shown("b.ts"));
    expect(files.store.get().edits["s1"]?.base.file).toBe("b.ts");
    expect(files.document("s1", "a.ts")?.store.get().edits["s1"]?.text).toBe(
      "my A",
    );
    files.workspace.activate("s1", "a.ts");
    expect(files.store.get().edits["s1"]?.text).toBe("my A");
  });
  it("replaces only the focused side and restores one editor", async () => {
    const files = rig();
    await files.actions.showForEdit("s1", shown("a.ts"));
    await files.workspace.openToSide("s1", shown("b.ts"));
    files.workspace.focus("s1", "left");
    await files.actions.show("s1", shown("c.ts"));
    expect(files.workspace.of("s1").split).toMatchObject({
      left: "c.ts",
      right: "b.ts",
      focus: "left",
    });
    files.workspace.restore("s1");
    expect(files.workspace.of("s1").split).toBeNull();
    expect(files.workspace.of("s1").active).toBe("c.ts");
  });
  it("refuses to close unsaved tabs and keeps every file beyond the recent picker limit", async () => {
    const files = rig();
    for (let n = 0; n < 18; n++)
      await files.actions.showForEdit("s1", shown(`${String(n)}.ts`));
    files.edit.update("s1", { text: "unsaved" });
    expect(files.workspace.close("s1", "17.ts")).toBe(false);
    files.workspace.closeSaved("s1");
    expect(files.workspace.of("s1").order).toEqual(["17.ts"]);
  });
});

it("keeps the other editor when closing one side and uses one buffer for duplicate views", async () => {
  const files = rig();
  await files.actions.showForEdit("s1", shown("a.ts"));
  await files.workspace.openToSide("s1", shown("a.ts"));
  files.workspace.focus("s1", "right");
  expect(files.workspace.of("s1").split?.focus).toBe("right");
  files.edit.update("s1", { text: "shared text" });
  expect(files.document("s1", "a.ts")?.store.get().edits["s1"]?.text).toBe(
    "shared text",
  );
  await files.workspace.openToSide("s1", shown("b.ts"));
  expect(files.workspace.close("s1", "b.ts")).toBe(true);
  expect(files.workspace.of("s1").split).toBeNull();
  expect(files.workspace.of("s1").active).toBe("a.ts");
});

it("promotes an overflow choice into the least recently used saved visible slot", async () => {
  const files = rig();
  files.workspace.capacity("s1", 3);
  for (const file of ["a.ts", "b.ts", "c.ts"])
    await files.actions.showForEdit("s1", shown(file));
  files.workspace.activate("s1", "b.ts");
  files.edit.update("s1", { text: "unsaved B" });
  files.workspace.activate("s1", "c.ts");
  await files.actions.showForEdit("s1", shown("d.ts"));
  expect(files.workspace.of("s1").visible).toEqual(["d.ts", "b.ts", "c.ts"]);
  files.workspace.activate("s1", "a.ts");
  expect(files.workspace.of("s1").visible).toEqual(["d.ts", "b.ts", "a.ts"]);
});
