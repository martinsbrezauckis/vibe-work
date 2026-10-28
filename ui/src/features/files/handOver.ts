/**
 * "Send to Vibe Work": what other programs handed over (files and folders from a right-click menu, text from the
 * macOS text service) becomes chips in the message box of the open chat, or of the new-chat view when no chat is
 * open, and the box gets the focus. Files use the composer's document attachment reader, folders are references,
 * and selected text is quoted data.
 * Depends on: the client's `handOver`, the composer facade, the file chip builders, home, composerFocus.
 * Called by: `useHandOverWiring` (the shell), tests.
 * Invariants: only chips are added: never text in the box, never a send; a chip already in the box is not added
 * twice; what was refused and what was added is written in the box's note line (a status region, so it is read
 * out).
 */
import { useEffect } from "react";

import type { HandOverView } from "../../protocol/client/types";
import { requestComposerFocus } from "../../state/composerFocus";
import { NEW_CHAT } from "../../state/newChatKey";
import { useApp } from "../shell/AppContext";
import type { App } from "../shell/AppContext";
import { goHome } from "../shell/home";
import { folderChip } from "./model/context";

/** "Selected text from Mail · 812 characters". */
function textChipLabel(text: string, from: string | undefined): string {
  const count = `${text.length.toLocaleString("en-US")} character${text.length === 1 ? "" : "s"}`;
  return from === undefined ? `Selected text · ${count}` : `Selected text from ${from} · ${count}`;
}

function counted(n: number, one: string, many: string): string {
  return `${String(n)} ${n === 1 ? one : many}`;
}

/**
 * Puts one hand-over into the open chat's box (else the new-chat view's) and returns the note it wrote. Shows the
 * work view (leaving Settings or another page) and focuses the box.
 */
async function placeHandOver(
  app: App,
  view: HandOverView,
  sources: Map<string, { target: string; uri: string }>,
): Promise<string | null> {
  const facade = app.composer.actions;
  if (facade === null) return null;
  goHome(app);
  const shell = app.shell.store.get();
  const target = shell.selected ?? NEW_CHAT;
  const cwd =
    target === NEW_CHAT ? shell.newFolder : app.core?.store.get().sessions[target]?.session["cwd"];
  const project = typeof cwd === "string" ? cwd : null;
  const present = () => app.composer.store.get().context[target] ?? [];
  let files = 0;
  let folders = 0;
  const documents = app.documents;
  const refused = [...view.refused];
  for (const [key, prior] of sources) {
    if (!(app.composer.store.get().context[prior.target] ?? []).some((c) => c.uri === prior.uri))
      sources.delete(key);
  }
  for (const { path, document } of view.files) {
    const key = JSON.stringify([target, path]);
    const prior = sources.get(key);
    if (prior?.target === target) {
      if (prior.uri !== `vw://doc/${document.id}`)
        await documents?.actions.remove(target, document.id);
      files += 1;
      continue;
    }
    if (documents === null) continue;
    const placed = await documents.actions.attach(target, [document]);
    refused.push(...placed.refused);
    if (placed.added.length > 0) {
      sources.set(key, { target, uri: `vw://doc/${document.id}` });
      files += 1;
    }
  }
  for (const path of view.folders) {
    const chip = folderChip({ path, project });
    if (!present().some((item) => item.source === "folder" && item.text === chip.text))
      facade.insertContext(target, chip);
    folders += 1;
  }
  let text = false;
  if (view.text !== null) {
    const label = textChipLabel(view.text.text, view.text.from);
    if (!present().some((item) => item.source === "selection" && item.text === view.text?.text))
      facade.insertContext(target, { source: "selection", label, text: view.text.text });
    text = true;
  }
  const added = [
    files > 0 ? counted(files, "file", "files") : null,
    folders > 0 ? counted(folders, "folder", "folders") : null,
    text ? "the selected text" : null,
  ].filter((part) => part !== null);
  const parts = [
    added.length > 0 ? `Added to your message: ${added.join(", ")}.` : null,
    refused.length > 0
      ? `Not added: ${refused.map((r) => `${r.name}: ${r.reason}`).join(" · ")}`
      : null,
  ].filter((part) => part !== null);
  const note = parts.length > 0 ? parts.join(" ") : null;
  if (note !== null) app.composer.note(target, note);
  requestComposerFocus(target);
  return note;
}

/** Takes every waiting hand-over and places each. */
async function takeHandOvers(
  app: App,
  sources: Map<string, { target: string; uri: string }>,
): Promise<void> {
  const client = app.core?.client.handOver;
  if (client === undefined) return;
  for (const view of await client.take()) await placeHandOver(app, view, sources);
}

/** Listens for hand-overs for the lifetime of the shell (also the ones that waited for the window to start). */
export function useHandOverWiring(): void {
  const app = useApp();
  useEffect(() => {
    const client = app.core?.client.handOver;
    if (client === undefined) return;
    const sources = new Map<string, { target: string; uri: string }>();
    let taking = Promise.resolve();
    let live = true;
    let stop: (() => void) | null = null;
    void client
      .subscribe(() => {
        taking = taking.then(() => takeHandOvers(app, sources)).catch(() => undefined);
      })
      .then((off) => {
        if (live) stop = off;
        else off();
      })
      .catch(() => undefined);
    return () => {
      live = false;
      stop?.();
    };
  }, [app]);
}
