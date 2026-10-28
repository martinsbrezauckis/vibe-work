/**
 * The `ui/processes` UI-control route: an external agent reads and uses the "Left running" list as the
 * window shows it (processes deleted sessions left running) and clicks Stop like a user does.
 * - `read` (default): whether the list dialog is open, the items (id, session, title, pid, name, ports) and which
 *   are stopping;
 * - `open` / `close`: the dialog, as the status bar button and its Close do;
 * - `stop {id}`: Stop on one item (the request is the user's click; no dialog).
 * - For the open session: `session` reads its processes (id, kind vibe|started,
 *   command, status, pid, ports, earlier) with the shown outputs; `output {id}` shows or hides a process's live
 *   output like the Output button (and answers it); `stopSession {id}` is its Stop.
 * The after-delete question itself is not asked for a Debug API delete: `ui/sessions delete` answers the list.
 * Depends on: state/leftRunning, state/uiControl, AppContext types. Called by: useShellWiring. The Rust side lists
 * the route (`crates/vw-host/src/host.rs` SHELL_ROUTES).
 * Invariants: bodies are validated (`invalidParams`); every action calls the controller function a click calls.
 */
import { UiControlError } from "../../state/uiControl";
import type { App } from "./AppContext";

function asObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

/** The list as the window shows it. */
function leftRunningJson(app: App) {
  const state = app.leftRunning?.store.get();
  return {
    open: state?.open ?? false,
    items: state?.items ?? [],
    stopping: state?.stopping ?? [],
  };
}

/** The open session's processes as the dialog shows them. */
function sessionJson(app: App) {
  const state = app.sessionProcesses?.store.get();
  return {
    sessionId: state?.sessionId ?? null,
    items: state?.items ?? [],
    outputs: state?.outputs ?? {},
    stopping: state?.stopping ?? [],
  };
}

/** `session` (read the list), `output {id}` (show or hide its live output, like the Output button), `stopSession {id}`. */
async function sessionAction(app: App, input: Record<string, unknown>): Promise<unknown> {
  const procs = app.sessionProcesses;
  if (procs === null) throw new UiControlError("serverUnavailable", "the core is not running");
  await procs.refresh();
  if (input["action"] === "session")
    return { ...sessionJson(app), leftRunning: leftRunningJson(app) };
  const id = input["id"];
  if (typeof id !== "string" || !procs.store.get().items.some((i) => i.id === id))
    throw new UiControlError("notFound", "no process of the open session has that id");
  if (input["action"] === "output") {
    procs.toggleOutput(id);
    await procs.readOutputs();
    return sessionJson(app);
  }
  const stopped = await procs.stop(id);
  return { stopped, ...sessionJson(app) };
}

/** Registers `ui/processes`; returns the function that removes it. */
export function registerProcessRoutes(app: App): () => void {
  return app.uiControl.register("processes", async (body) => {
    const input = asObject(body);
    const left = app.leftRunning;
    if (left === null) throw new UiControlError("serverUnavailable", "the core is not running");
    switch (input["action"] ?? "read") {
      case "read":
        await left.refresh();
        break;
      case "open":
        left.open();
        await left.refresh();
        break;
      case "close":
        left.close();
        break;
      case "stop": {
        const id = input["id"];
        if (typeof id !== "string" || id === "")
          throw new UiControlError("invalidParams", "stop needs the item's id");
        await left.refresh();
        if (!left.store.get().items.some((i) => i.id === id))
          throw new UiControlError("notFound", "no listed process has that id");
        const stopped = await left.stop(id);
        return { stopped, ...leftRunningJson(app) };
      }
      // the open session's part of the panel.
      case "session":
      case "output":
      case "stopSession":
        return sessionAction(app, input);
      default:
        throw new UiControlError(
          "invalidParams",
          "action must be read, open, close, stop, session, output or stopSession",
        );
    }
    return leftRunningJson(app);
  });
}
