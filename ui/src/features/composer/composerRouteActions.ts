/**
 * The handler of the `ui/composer` UI-control route (the route and its actions are documented
 * in `composerRoutes.ts`), loaded on first use so the main bundle does not carry it.
 * Depends on: AppContext types, the composer store and actions, the composer model. Called by:
 * composerRoutes (dynamic import).
 * Invariants: bodies are validated (`invalidParams`); `sessionId` defaults to the selected session, else the
 * new-chat view's composer (`"new-chat"`: type, attach, access and model are held there, send starts the
 * session); no action exists here that the composer's UI does not offer; every action runs through the Debug API's
 * action set (`app.composer.debugApi`, origin bound at creation) and everything it writes is marked as
 * Debug API content, so nothing it leaves behind runs later with the window's origin.
 */
import { NEW_CHAT } from "../../state/newChatKey";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import { configuredModeId, currentModeId } from "./model/access";
import { currentPick, newChatPick } from "./model/models";
import { queuedTurns, runningTurnId } from "./model/session";

function asObject(body: unknown): Record<string, unknown> {
  if (typeof body === "object" && body !== null && !Array.isArray(body))
    return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function text(value: unknown, name: string, allowEmpty = false): string {
  if (typeof value === "string" && (allowEmpty || value !== "")) return value;
  throw new UiControlError(
    "invalidParams",
    `${name} must be a ${allowEmpty ? "" : "non-empty "}string`,
  );
}

function bytesOf(base64: string): Uint8Array<ArrayBuffer> {
  let binary: string;
  try {
    binary = atob(base64);
  } catch {
    throw new UiControlError("invalidParams", "data must be base64");
  }
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** Runs one `ui/composer` request for `app`. */
export async function handleComposer(app: App, body: unknown): Promise<unknown> {
  const sessionOf = (input: Record<string, unknown>): string => {
    const given = input["sessionId"];
    if (typeof given === "string" && given !== "") return given;
    // with no session selected the composer on screen is the new-chat view's (`NEW_CHAT`).
    return app.shell.store.get().selected ?? NEW_CHAT;
  };
  const actionsOrFail = () => {
    // The Debug API's action set: every core call carries its origin and everything it writes into
    // the composer is marked as its content (including its origin), whoever sends it later.
    const actions = app.composer.debugApi;
    if (actions === null) throw new UiControlError("serverUnavailable", "the app has no core");
    return actions;
  };

  const read = (sessionId: string): unknown => {
    const s = app.composer.store.get();
    const entry = app.core?.store.get().sessions[sessionId];
    const state = entry?.state ?? null;
    const session = entry?.session;
    const isNew = sessionId === NEW_CHAT;
    const held = s.newChat;
    const pick = isNew
      ? newChatPick(s.config ?? undefined, held, s.defaultReasoning)
      : currentPick(entry?.runtime, session);
    const shell = app.shell.store.get();
    return {
      sessionId,
      // The new-chat view: its folder chip, whether Send asked for a folder, the held picks' author.
      ...(isNew
        ? {
            newChat: {
              folder: shell.newFolder,
              folderBy: shell.newFolderBy,
              askFolder: held.askFolder,
              heldBy: held.by,
            },
          }
        : {}),
      draft: s.drafts[sessionId] ?? "",
      context: (s.context[sessionId] ?? []).map((c) => ({
        id: c.id,
        label: c.label,
        source: c.source,
      })),
      images: (s.images[sessionId] ?? []).map((i) => ({
        id: i.id,
        name: i.name,
        mimeType: i.mimeType,
        bytes: i.bytes,
      })),
      sending: s.sending[sessionId] === true,
      compacting: s.compacting[sessionId] === true,
      note: s.notes[sessionId] ?? null,
      running: runningTurnId(state, session),
      queue: queuedTurns(state),
      mode: isNew
        ? (held.mode ?? configuredModeId(s.config ?? undefined))
        : currentModeId(entry?.runtime, session, entry?.accessMode),
      model: pick.alias,
      effort: pick.effort,
      menu: s.menu?.sessionId === sessionId ? s.menu.kind : null,
      rewind: s.rewind?.sessionId === sessionId ? s.rewind : null,
      // the pid of another Vibe process holding this session (the composer is read-only), else null.
      heldByPid: isNew ? null : (app.core?.store.get().held[sessionId] ?? null),
    };
  };

  const act = async (input: Record<string, unknown>): Promise<unknown> => {
    const action = input["action"] ?? "read";
    const sessionId = sessionOf(input);
    const actions = actionsOrFail();
    switch (action) {
      case "read":
        await actions.prepareSession(sessionId);
        return read(sessionId);
      case "type":
        app.composer.setDraftFromDebugApi(sessionId, text(input["text"], "text", true));
        return read(sessionId);
      case "send": {
        const mode = input["mode"] ?? "send";
        if (mode !== "send" && mode !== "steer" && mode !== "queue") {
          throw new UiControlError("invalidParams", "mode must be send, steer or queue");
        }
        const outcome = await actions.send(sessionId, mode);
        // From the new-chat view, `outcome.started` names the session the send created.
        return { outcome: { ...outcome, result: undefined }, ...asObject(read(sessionId)) };
      }
      case "insertContext":
        actions.insertContext(sessionId, {
          label: text(input["label"], "label"),
          source: text(input["source"], "source"),
          text: text(input["text"], "text"),
        });
        return read(sessionId);
      case "attachImage": {
        const name = text(input["name"], "name");
        const file = new File([bytesOf(text(input["data"], "data"))], name);
        const added = await actions.attachFiles(sessionId, [file]);
        return { added, ...asObject(read(sessionId)) };
      }
      case "stop":
        return { stopped: await actions.stop(sessionId) };
      case "retry": {
        const extra = typeof input["instructions"] === "string" ? input["instructions"] : "";
        return { outcome: { ...(await actions.retry(sessionId, extra)), result: undefined } };
      }
      case "slash": {
        const args = typeof input["args"] === "string" ? input["args"] : "";
        return { outcome: await actions.runCommand(sessionId, text(input["name"], "name"), args) };
      }
      case "access":
        // Answers the composer as it then reads (like `model`), so a held new-chat pick is visible at once.
        return {
          ok: await actions.setAccess(sessionId, text(input["mode"], "mode")),
          ...asObject(read(sessionId)),
        };
      case "model": {
        const alias = typeof input["alias"] === "string" ? input["alias"] : undefined;
        const effort = typeof input["effort"] === "string" ? input["effort"] : undefined;
        return {
          ok: await actions.pickModel(sessionId, alias, effort),
          ...asObject(read(sessionId)),
        };
      }
      case "queueEdit":
        return {
          ok: await actions.replaceQueued(
            sessionId,
            text(input["queueItemId"], "queueItemId"),
            text(input["text"], "text"),
          ),
        };
      case "queueRemove":
        return {
          ok: await actions.removeQueued(sessionId, text(input["queueItemId"], "queueItemId")),
        };
      case "rewind": {
        const entryId = text(input["entryId"], "entryId");
        await actions.openRewind(sessionId, entryId, "");
        const confirm = input["confirm"];
        if (confirm === "here" || confirm === "new") {
          return {
            ok: await actions.confirmRewind(confirm === "new"),
            selected: app.shell.store.get().selected,
          };
        }
        return read(sessionId);
      }
      case "rewindClose":
        actions.closeRewind();
        return read(sessionId);
      case "branch":
        return { branch: await actions.branch(sessionId, text(input["entryId"], "entryId")) };
      default:
        throw new UiControlError(
          "invalidParams",
          `unknown action ${typeof action === "string" ? action : "(not a string)"}`,
        );
    }
  };

  return act(asObject(body));
}
