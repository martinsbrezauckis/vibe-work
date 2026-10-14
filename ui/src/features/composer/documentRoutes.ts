/**
 * The `ui/documents` UI-control route. The Rust side lists the route name
 * (`crates/vw-host/src/host.rs` SHELL_ROUTES). Every action calls the same function a click on a
 * document chip calls, through the Debug API's action set (`app.documents.debugApi`), so every core call
 * it triggers carries the Debug API origin (a label; no native confirmation for it):
 * - `read` (default): the session's waiting documents (id, name, kind, status, pages, error), its read
 *   documents (context chips: uri, name, pages, characters, kept) and the open preview;
 * - `extract {docId}`, `remove {docId}`, `asPicture {docId}`, `preview {docId, open?}` (docId of a read
 *   document's context chip).
 * There is no action here that adds a file: the composer's picker, paste and drop share the file router.
 * Depends on: AppContext, uiControl, the documents and composer stores. Called by: useShellWiring.
 * Invariants: bodies are validated (`invalidParams`); `sessionId` defaults to the selected session.
 */
import { toCallError } from "../../protocol/client/types";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";

function asObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function docIdOf(input: Record<string, unknown>): string {
  const id = input["docId"];
  if (typeof id === "string" && id !== "") return id;
  throw new UiControlError("invalidParams", "docId must be a non-empty string");
}

/** What `read` answers for one session. */
function readDocs(app: App, sessionId: string): unknown {
  const docs = app.documents?.store.get();
  const context = app.composer.store.get().context[sessionId] ?? [];
  return {
    sessionId,
    waiting: (docs?.chips[sessionId] ?? []).map((d) => ({
      id: d.id,
      name: d.name,
      kind: d.kind,
      method: d.method,
      status: d.status,
      pages: d.pages,
      error: d.error,
      canAttachAsPicture: d.canAttachAsPicture,
    })),
    read: context
      .filter((c) => c.doc !== undefined)
      .map((c) => ({
        contextId: c.id,
        uri: c.uri ?? null,
        name: c.label,
        pages: c.doc?.pages ?? null,
        chars: c.doc?.chars ?? 0,
        kept: c.doc?.kept ?? null,
        model: c.doc?.model ?? null,
      })),
    preview: docs?.preview ?? null,
  };
}

/** Runs one `ui/documents` request. */
export async function handleDocuments(app: App, body: unknown): Promise<unknown> {
  const input = asObject(body);
  const documents = app.documents;
  if (documents === null) throw new UiControlError("serverUnavailable", "the app has no core");
  const given = input["sessionId"];
  const sessionId =
    typeof given === "string" && given !== "" ? given : app.shell.store.get().selected;
  if (sessionId === null) throw new UiControlError("invalidParams", "no session is selected");
  const actions = documents.debugApi;
  const action = input["action"] ?? "read";
  try {
    switch (action) {
      case "read":
        return readDocs(app, sessionId);
      case "extract":
        await actions.extract(sessionId, docIdOf(input));
        return readDocs(app, sessionId);
      case "remove":
        await actions.remove(sessionId, docIdOf(input));
        return readDocs(app, sessionId);
      case "asPicture":
        await actions.asPicture(sessionId, docIdOf(input));
        return readDocs(app, sessionId);
      case "preview": {
        const open = input["open"] !== false;
        actions.preview(open ? { sessionId, contextId: docIdOf(input) } : null);
        return readDocs(app, sessionId);
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
export function registerDocumentRoutes(app: App): () => void {
  return app.uiControl.register("documents", (body) => handleDocuments(app, body));
}
