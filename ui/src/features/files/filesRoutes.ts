/**
 * The `ui/files` UI-control route (the Rust side lists it in `crates/vw-host/src/host.rs`
 * FILES_ROUTES). Every user action of the feature can be done here, through the same functions a click uses, with
 * the Debug API origin (the token holder acts as the user; the core asks no dialog for it):
 * - `state` (default): the File tab of the session (shown file, kind, size, status, mode, truncation, the text it
 *   shows, the viewer, the recent list) and the last drop report;
 * - `refs {entryId?}`: the file references the transcript shows (text, path, line, state `ok|no|pending`, the entry
 *   they are in), from the rendered rows; without `entryId` also every path the session's transcript asked about;
 * - `open {text, line?}` (a path as the chat writes it) or `open {file}` (a handle from `refs`/`recent`);
 * - `ref-menu {text, item?, file?, line?}`: the linked name's Open, Edit, Show in folder and Copy path actions;
 *   without an item, lists actions or ambiguous choices; a choice always needs its returned file handle.
 *   `switch {file}`, `close`, `reload`, `mode {mode: rendered|source}`, `external {what: open|reveal}`;
 * - `edit {text}`: saves `text` as the shown text file's whole content, like the editor's Save
 *   (conflict-checked: `stale` when the file changed on disk since the tab read it; nothing goes into the composer);
 * - `drop {files: [{name, data (base64), folder?}]}`: the same routing as a drop on the composer; answers the drop
 *   report and the composer's chips (pictures, document chips, context chips).
 * Depends on: AppContext, state/files, drop, uiControl. Called by: register.tsx.
 * Invariants: bodies are validated (`invalidParams`); `sessionId` defaults to the selected session.
 */
import { toCallError } from "../../protocol/client/types";
import type { Files } from "../../state/files";
import { tabOf } from "../../state/files";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import { type DroppedItem, runDrop } from "./drop";
import { saveEdit } from "./saveEdit";
import { saveEditor } from "./saveEditor";
import { editableKind, runFileRefAction } from "./fileRefActions";

/** Most characters of the shown text in a `state` answer. */
const STATE_TEXT_LIMIT = 65_536;

function asObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function text(input: Record<string, unknown>, key: string): string {
  const value = input[key];
  if (typeof value === "string" && value !== "") return value;
  throw new UiControlError("invalidParams", `${key} must be a non-empty string`);
}

/** The tab state of `sessionId`, as the route answers it. */
function state(app: App, files: Files, sessionId: string): unknown {
  const tab = tabOf(files.store.get(), sessionId);
  const c = tab.content;
  const shownText = c?.text ?? null;
  const draft = files.store.get().edits[sessionId];
  return {
    sessionId,
    panel: app.shell.store.get().panels[sessionId] ?? null,
    shown: tab.shown,
    status: tab.status,
    error: tab.error,
    editNotice: tab.editNotice,
    mode: tab.mode,
    content:
      c === null
        ? null
        : {
            name: c.name,
            display: c.display,
            kind: c.kind,
            size: c.size,
            fingerprint: c.fingerprint,
            truncated: c.truncated,
            tooLarge: c.tooLarge,
            openClass: c.openClass,
            hasPicture: c.dataUrl !== null,
            text: shownText === null ? null : shownText.slice(0, STATE_TEXT_LIMIT),
            textCut: shownText !== null && shownText.length > STATE_TEXT_LIMIT,
          },
    viewer: tab.viewer,
    recent: tab.recent,
    lastDrop: files.store.get().drops[sessionId] ?? null,
    editor:
      draft === undefined
        ? null
        : {
            text: draft.text.slice(0, STATE_TEXT_LIMIT),
            textCut: draft.text.length > STATE_TEXT_LIMIT,
            dirty: draft.text !== draft.base.text,
            conflict: draft.conflict,
            problem: draft.problem,
            saving: draft.saving,
            compare: draft.compare,
            wrap: draft.wrap,
            baselineFingerprint: draft.base.fingerprint,
            diskFingerprint: draft.disk.fingerprint,
          },
  };
}

/** The references the rendered transcript shows (optionally of one entry). */
function refs(files: Files, sessionId: string, entryId: string | null): unknown {
  const root =
    entryId === null
      ? document
      : document.querySelector(`[data-entry-id="${CSS.escape(entryId)}"]`);
  const known = files.store.get().refs[sessionId] ?? {};
  const shown = [...(root?.querySelectorAll<HTMLElement>("[data-file-ref]") ?? [])].map((el) => {
    const path = el.dataset["fileRef"] ?? "";
    const ref = known[path];
    return {
      text: el.textContent,
      path,
      state: el.dataset["fileState"] ?? "pending",
      display: ref?.status === "ok" ? ref.display : null,
      file: ref?.status === "ok" ? ref.file : null,
      ...(ref?.status === "choices" ? { choices: ref.choices, total: ref.total } : {}),
      entryId: el.closest<HTMLElement>("[data-entry-id]")?.dataset["entryId"] ?? null,
    };
  });
  const asked =
    entryId !== null
      ? undefined
      : Object.entries(known).map(([path, ref]) => ({
          path,
          state: ref.status,
          display: ref.status === "ok" ? ref.display : null,
          file: ref.status === "ok" ? ref.file : null,
          ...(ref.status === "choices" ? { choices: ref.choices, total: ref.total } : {}),
        }));
  return { sessionId, shown, ...(asked === undefined ? {} : { asked }) };
}

/** The `drop` body's files as drop items. */
function dropItems(input: Record<string, unknown>): DroppedItem[] {
  const list = input["files"];
  if (!Array.isArray(list) || list.length === 0)
    throw new UiControlError(
      "invalidParams",
      "files must be a non-empty array of {name, data, folder?}",
    );
  return list.map((raw: unknown) => {
    const file = asObject(raw);
    const name = text(file, "name");
    const folder = file["folder"] === true;
    const data = typeof file["data"] === "string" ? file["data"] : "";
    const binary = atob(data);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
    return { name, folder, size: bytes.length, bytes: () => Promise.resolve(bytes) };
  });
}

/** One `ui/files` request. */
async function route(app: App, files: Files, body: unknown): Promise<unknown> {
  const input = asObject(body);
  const given = input["sessionId"];
  const sessionId =
    typeof given === "string" && given !== "" ? given : app.shell.store.get().selected;
  if (sessionId === null) throw new UiControlError("invalidParams", "no session is selected");
  const actions = files.debugApi;
  const action = input["action"] ?? "state";
  const recent = (file: string) => {
    const hit = tabOf(files.store.get(), sessionId).recent.find((r) => r.file === file);
    if (hit !== undefined) return hit;
    const ref = Object.values(files.store.get().refs[sessionId] ?? {}).find(
      (r) => r.status === "ok" && r.file === file,
    );
    if (ref?.status === "ok") return { file: ref.file, display: ref.display, line: null };
    throw new UiControlError(
      "notFound",
      "no such file handle in this session's transcript or recent list",
    );
  };
  try {
    switch (action) {
      case "state":
        return state(app, files, sessionId);
      case "ref-menu": {
        const ref = await actions.resolveRef(sessionId, text(input, "text"));
        const item = input["item"];
        if (
          item !== undefined &&
          item !== "open" &&
          item !== "edit" &&
          item !== "reveal" &&
          item !== "copy-path"
        )
          throw new UiControlError("invalidParams", "item must be open, edit, reveal or copy-path");
        if (ref.status !== "ok" && ref.status !== "choices")
          throw new UiControlError(
            "notFound",
            "that path does not name a file this session may show",
          );
        if (ref.status === "choices" && input["file"] === undefined)
          return { choices: ref.choices, total: ref.total };
        const target =
          ref.status === "ok" ? ref : ref.choices.find((c) => c.file === input["file"]);
        if (target === undefined)
          throw new UiControlError("notFound", "choose a file from this reference's choices");
        if (item === undefined)
          return {
            items: ["open", "edit", "reveal", "copy-path"],
            canEdit: editableKind(target.kind),
          };
        if (item === "edit" && !editableKind(target.kind))
          throw new UiControlError("invalidParams", "this file kind has no editor");
        const line = typeof input["line"] === "number" ? input["line"] : null;
        await runFileRefAction(
          app,
          sessionId,
          { file: target.file, display: target.display, line },
          item,
          "debug-api",
        );
        return state(app, files, sessionId);
      }
      case "openEdit":
        app.shell.actions.openTab("file");
        if (!files.edit.begin(sessionId))
          throw new UiControlError(
            "invalidParams",
            "the File tab shows no complete editable text file",
          );
        return state(app, files, sessionId);
      case "setText": {
        const draft = files.store.get().edits[sessionId];
        if (draft === undefined || draft.saving)
          throw new UiControlError("invalidParams", "no editor is open, or a save is running");
        if (typeof input["text"] !== "string")
          throw new UiControlError("invalidParams", "text must be a string");
        files.edit.update(sessionId, { text: input["text"] });
        return state(app, files, sessionId);
      }
      case "saveEdit": {
        const outcome = await saveEditor(app, sessionId, "debug-api");
        if (outcome.kind === "error") throw new UiControlError("invalidParams", outcome.message);
        return { outcome: outcome.kind, ...(state(app, files, sessionId) as object) };
      }
      case "cancelEdit":
        if (files.store.get().edits[sessionId]?.saving === true)
          throw new UiControlError("invalidParams", "a save is running");
        files.edit.cancel(sessionId);
        return state(app, files, sessionId);
      case "reloadEdit":
        if (files.store.get().edits[sessionId] === undefined)
          throw new UiControlError("invalidParams", "no editor is open");
        if (files.store.get().edits[sessionId]?.saving === true)
          throw new UiControlError("invalidParams", "a save is running");
        if (!(await actions.reloadEdit(sessionId)))
          throw new UiControlError(
            "invalidParams",
            "the new version is not complete editable text",
          );
        return state(app, files, sessionId);
      case "keepEditing":
      case "compare": {
        const draft = files.store.get().edits[sessionId];
        if (draft === undefined) throw new UiControlError("invalidParams", "no editor is open");
        if (action === "compare" && (draft.disk.text === null || draft.disk.truncated))
          throw new UiControlError("invalidParams", "the disk version is not complete text");
        files.edit.update(
          sessionId,
          action === "compare" ? { compare: true } : { conflict: false, compare: false },
        );
        return state(app, files, sessionId);
      }
      case "refs":
        return refs(
          files,
          sessionId,
          typeof input["entryId"] === "string" ? input["entryId"] : null,
        );
      case "open": {
        app.shell.actions.openTab("file");
        if (typeof input["file"] === "string") await actions.show(sessionId, recent(input["file"]));
        else {
          const line = typeof input["line"] === "number" ? input["line"] : null;
          if (!(await actions.openText(sessionId, text(input, "text"), line)))
            throw new UiControlError(
              "notFound",
              "that path does not name a file this session may show",
            );
        }
        return state(app, files, sessionId);
      }
      case "switch":
        await actions.show(sessionId, recent(text(input, "file")));
        return state(app, files, sessionId);
      case "close":
        actions.close(sessionId);
        return state(app, files, sessionId);
      case "reload":
        await actions.reload(sessionId);
        return state(app, files, sessionId);
      case "mode": {
        const mode = input["mode"];
        if (mode !== "rendered" && mode !== "source")
          throw new UiControlError("invalidParams", "mode must be rendered or source");
        await actions.setMode(sessionId, mode);
        return state(app, files, sessionId);
      }
      case "external": {
        const what = input["what"];
        if (what !== "open" && what !== "reveal")
          throw new UiControlError("invalidParams", "what must be open or reveal");
        const error = await actions.external(sessionId, what);
        if (error !== null) throw new UiControlError("denied", error);
        return state(app, files, sessionId);
      }
      case "edit": {
        // a quick edit, saved like the editor's Save (conflict-checked; no composer chip).
        const original = tabOf(files.store.get(), sessionId).content?.text;
        if (original === null || original === undefined)
          throw new UiControlError("invalidParams", "the File tab shows no text file");
        const newText = input["text"];
        if (typeof newText !== "string")
          throw new UiControlError("invalidParams", "text must be a string");
        const outcome = await saveEdit(app, sessionId, original, newText, "debug-api");
        if (outcome.kind === "stale")
          throw new UiControlError("stale", "the file changed on disk since the tab read it");
        if (outcome.kind === "error") throw new UiControlError("invalidParams", outcome.message);
        return { outcome: outcome.kind, ...(state(app, files, sessionId) as object) };
      }
      case "drop": {
        const report = await runDrop(app, sessionId, dropItems(input), "debug-api");
        const composer = app.composer.store.get();
        return {
          report,
          note: composer.notes[sessionId] ?? null,
          pictures: (composer.images[sessionId] ?? []).map((i) => ({
            name: i.name,
            bytes: i.bytes,
          })),
          context: (composer.context[sessionId] ?? []).map((c) => ({
            label: c.label,
            source: c.source,
            uri: c.uri ?? null,
          })),
          documents: (app.documents?.store.get().chips[sessionId] ?? []).map((d) => ({
            name: d.name,
            kind: d.kind,
            status: d.status,
            pages: d.pages,
          })),
        };
      }
      default:
        throw new UiControlError("invalidParams", `unknown action ${JSON.stringify(action)}`);
    }
  } catch (thrown) {
    if (thrown instanceof UiControlError) throw thrown;
    const error = toCallError(thrown).error;
    throw new UiControlError(error.code, error.message);
  }
}

/** Registers the route; returns the function that removes it. */
export function registerFilesRoute(app: App, files: Files): () => void {
  return app.uiControl.register("files", (body) => route(app, files, body));
}
