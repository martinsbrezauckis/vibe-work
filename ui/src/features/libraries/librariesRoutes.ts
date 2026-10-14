/**
 * The `ui/libraries` UI-control route. The Rust side lists the route name
 * (`crates/vw-host/src/host.rs` SHELL_ROUTES). Every action calls the same function a click calls, through the Debug
 * API's action set (`app.libraries.debugApi`), so every core call it triggers carries the Debug API origin:
 * - `read` (default): page open, libraries (id, name, documents, scope), the use facts, the selected library and
 *   its documents (id, name, status, pages), the note, the busy step, the last answer with its sources, the open text;
 * - `open`, `close`, `select {libraryId}`, `create {name, scope?: off|global|project:<id>}`,
 *   `scope {libraryId, scope}`, `delete {libraryId}`, `upload {libraryId, files: [{name, data (base64)}]}` (the bytes
 *   go to Mistral, like a drop), `deleteDocument {libraryId, documentId}`, `text {libraryId, documentId}`,
 *   `closeText`, `ask {question}`.
 * Depends on: AppContext, uiControl, LibraryDetail (`scopeOf`). Called by: useShellWiring.
 * Invariants: bodies are validated (`invalidParams`); there is no action with a path (the native picker is the
 * window's own).
 */
import type { LibraryFile } from "../../protocol/core";
import { toCallError } from "../../protocol/client/types";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import { scopeOf } from "./LibraryDetail";

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

function files(input: Record<string, unknown>): LibraryFile[] {
  const list = input["files"];
  if (!Array.isArray(list) || list.length === 0)
    throw new UiControlError("invalidParams", "files must be a non-empty list of {name, data}");
  return list.map((f: unknown) => {
    const o = asObject(f);
    return { name: text(o, "name"), data: typeof o["data"] === "string" ? o["data"] : "" };
  });
}

function read(app: App): unknown {
  const s = app.libraries?.store.get();
  if (s === undefined) return null;
  return {
    open: s.open,
    loading: s.loading,
    error: s.error,
    note: s.note,
    busy: s.busy,
    costs: s.costs,
    libraries: (s.libraries ?? []).map((l) => ({
      id: l.id,
      name: l.name,
      documents: l.documents,
      scope: l.scope,
    })),
    selected: s.selected,
    documents: (s.selected === null ? [] : (s.documents[s.selected] ?? [])).map((d) => ({
      id: d.id,
      name: d.name,
      status: d.status,
      pages: d.pages,
    })),
    question: s.question,
    answer: s.answer,
    text:
      s.viewer === null
        ? null
        : {
            name: s.viewer.name,
            source: s.viewer.source,
            chars: s.viewer.text.length,
            start: s.viewer.text.slice(0, 2000),
          },
  };
}

async function handle(app: App, body: unknown): Promise<unknown> {
  const input = asObject(body);
  const a = app.libraries?.debugApi;
  if (a === undefined) throw new UiControlError("serverUnavailable", "Libraries need the core");
  try {
    switch (input["action"] ?? "read") {
      case "read":
        break;
      case "open":
        await a.open();
        break;
      case "close":
        a.close();
        break;
      case "select":
        await a.select(text(input, "libraryId"));
        break;
      case "create":
        await a.create(
          text(input, "name"),
          scopeOf(typeof input["scope"] === "string" ? input["scope"] : "global"),
          null,
        );
        break;
      case "scope":
        await a.setScope(text(input, "libraryId"), scopeOf(text(input, "scope")));
        break;
      case "delete":
        await a.deleteLibrary(text(input, "libraryId"));
        break;
      case "upload":
        await a.uploadFiles(text(input, "libraryId"), files(input));
        break;
      case "deleteDocument":
        await a.deleteDocument(text(input, "libraryId"), text(input, "documentId"));
        break;
      case "text":
        await a.openText(text(input, "libraryId"), text(input, "documentId"));
        break;
      case "closeText":
        a.closeViewer();
        break;
      case "ask":
        await a.ask(text(input, "question"));
        break;
      default:
        throw new UiControlError(
          "invalidParams",
          `unknown action ${JSON.stringify(input["action"])}`,
        );
    }
  } catch (thrown) {
    if (thrown instanceof UiControlError) throw thrown;
    const error = toCallError(thrown).error;
    throw new UiControlError(error.code, error.message);
  }
  return read(app);
}

/** Registers the route; returns the function that removes it. */
export function registerLibrariesRoutes(app: App): () => void {
  return app.uiControl.register("libraries", (body) => handle(app, body));
}
