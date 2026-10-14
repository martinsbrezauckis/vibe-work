/**
 * The composer's session controls: the runtime snapshot behind the pickers, the model and
 * thinking pickers (`config/model/write`, pinned per session), the access menu
 * (`session/agent/update`; native picks are consent, other Full access switches ask), the skills of the `/`
 * menu (`skills/list`), and the GUI `/` commands.
 * Depends on: coreStore, composerStore, composerSend, composerRewind, shellStore, sessionActions,
 * toastStore, the composer model. Called by: createComposer.
 * Invariants:
 * - Every call goes through the core and the gate; nothing is shown as switched before Vibe answered
 *   (the runtime in the reply, or the next `runtimeUpdated`, is what the chips show).
 * - Chips are initialized when a composer opens and before its first turn; menus never initialize picks.
 */
import type { JsonValue } from "../protocol/core";
import { formatTokens } from "./account";
import { toCallError } from "../protocol/client/types";
import { modeOf } from "../features/composer/model/access";
import { currentPick, type CurrentPick } from "../features/composer/model/models";
import { anchorProblem } from "../features/composer/model/session";
import { type Obj, arr, obj, str } from "../features/transcript/model/entries";
import type { ComposerStore } from "./composerStore";
import type { SendActions, SendOutcome } from "./composerSend";
import type { RewindActions } from "./composerRewind";
import type { CoreController } from "./coreStore";
import type { SessionActions } from "./sessionActions";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";
import {
  modelWriteAccepted,
  prepareComposerSession,
  type ComposerInitialization,
} from "./composerDefaults";

/** The runtime snapshot inside a reply (`{runtime: ...}`), if any. */
function runtimeIn(result: JsonValue | undefined): JsonValue | undefined {
  const runtime = obj(result ?? null)?.["runtime"];
  return obj(runtime ?? null) === null ? undefined : runtime;
}

/** The session controls. */
export function createSessionControls(deps: {
  core: CoreController;
  composer: ComposerStore;
  shell: Shell;
  toasts: Toasts;
  sessions: SessionActions | null;
  send: () => SendActions;
  rewind: RewindActions;
  /** Bound by the facade only for a trusted native access-menu event. */
  trustedAccessPick: boolean;
  initialization: ComposerInitialization;
  /** Opens the Tools page (connectors and MCP servers) like the rail's plug button; `/mcp`, `/connectors`. */
  openTools?: (() => void) | undefined;
}) {
  const { core, composer, shell, toasts, sessions, rewind } = deps;
  const call = async (method: string, params: Record<string, JsonValue>, token?: string) => {
    const reply = await core.run({
      type: "call",
      method,
      params,
      ...(token === undefined ? {} : { confirmationToken: token }),
    });
    return reply.type === "reply" ? reply.result : null;
  };
  const keepRuntime = (sessionId: string, result: JsonValue | null) => {
    const runtime = runtimeIn(result ?? undefined);
    if (runtime !== undefined) core.setRuntime(sessionId, runtime);
  };
  const failed = (thrown: unknown): string => toCallError(thrown).error.message;
  const compactionRuns = new Map<string, symbol>();
  const historyOf = (sessionId: string) =>
    arr(core.store.get().sessions[sessionId]?.state?.["history"]);

  const controls = {
    /** Loads effective chips and admits new-chat defaults before interaction or submission. */
    prepareSession(sessionId: string, fresh = false, held?: CurrentPick): Promise<boolean> {
      return prepareComposerSession(core, composer, deps.initialization, sessionId, fresh, held);
    },
    /** Reads the session's runtime (models, profiles, active ones). Opens its channel if needed. */
    async loadRuntime(sessionId: string): Promise<boolean> {
      try {
        keepRuntime(sessionId, await call("runtime/read", { sessionId }));
        return true;
      } catch {
        return false;
      }
    },
    /** Refreshes the configured models from the lobby so later saved choices apply to new chats. */
    async loadConfig(): Promise<boolean> {
      try {
        const cwd = shell.store.get().newFolder;
        const result = await call("config/read", cwd === null ? {} : { cwd });
        if (result === null) return false;
        composer.setConfig(result);
        return true;
      } catch {
        return false;
      }
    },
    /** Loads the skills the `/` menu offers (`skills/list`), once per session. */
    async loadSkills(sessionId: string): Promise<void> {
      if (composer.store.get().skills[sessionId] !== undefined) return;
      try {
        const result = await call("skills/list", { sessionId });
        const { skillRows } = await import("../features/composer/model/slash");
        composer.setSkills(sessionId, skillRows(result ?? undefined));
      } catch {
        composer.setSkills(sessionId, []);
      }
    },
    /** Pins a model and/or thinking level to the session (`config/model/write`). */
    async pickModel(
      sessionId: string,
      modelAlias?: string,
      reasoningEffort?: string,
    ): Promise<boolean> {
      composer.setMenu(null);
      try {
        const pending = deps.initialization.pending.get(sessionId);
        if (pending !== undefined) await pending;
        const reply = await core.run({
          type: "pickModel",
          sessionId,
          ...(modelAlias === undefined ? {} : { modelAlias }),
          ...(reasoningEffort === undefined ? {} : { reasoningEffort }),
        });
        if (reply.type !== "reply" || !modelWriteAccepted(reply.result)) return false;
        keepRuntime(sessionId, reply.result);
        if (runtimeIn(reply.result) === undefined)
          if (!(await controls.loadRuntime(sessionId))) return false;
        const actual = currentPick(core.store.get().sessions[sessionId]?.runtime, undefined);
        if (
          (modelAlias !== undefined && actual.alias !== modelAlias) ||
          (reasoningEffort !== undefined && actual.effort !== reasoningEffort)
        )
          return false;
        deps.initialization.ready.add(sessionId);
        return true;
      } catch (thrown) {
        toasts.push({ kind: "error", bellKind: "failure", text: failed(thrown) });
        return false;
      }
    },
    /**
     * Switches the access mode. A native menu pick is the user's decision; other Full access switches
     * ask for a native token. The core still gates every call; declining leaves the mode unchanged.
     */
    async setAccess(sessionId: string, agentName: string): Promise<boolean> {
      composer.setMenu(null);
      try {
        const pending = deps.initialization.pending.get(sessionId);
        if (pending !== undefined) await pending;
        let token: string | undefined;
        if (modeOf(agentName)?.confirm === true && !deps.trustedAccessPick) {
          const answer = await core.run({
            type: "requestConfirmation",
            method: "session/agent/update",
            params: { sessionId, agentName },
          });
          if (answer.type === "confirmation") token = answer.token;
        }
        const reply = await core.run({
          type: "setAccessMode",
          sessionId,
          agentName,
          ...(token === undefined ? {} : { confirmationToken: token }),
        });
        keepRuntime(sessionId, reply.type === "reply" ? reply.result : null);
        if (runtimeIn(reply.type === "reply" ? reply.result : undefined) === undefined) {
          await controls.loadRuntime(sessionId);
        }
        return true;
      } catch (thrown) {
        const error = toCallError(thrown).error;
        if (error.code !== "confirmationDeclined")
          toasts.push({ kind: "error", bellKind: "failure", text: error.message });
        return false;
      }
    },
    /** Runs a GUI `/` command (`slashNames.ts`); `args` is the text after the name. */
    async runCommand(sessionId: string, name: string, args: string): Promise<SendOutcome> {
      const done = (): SendOutcome => ({ ok: true, kind: "command" });
      const fail = (message: string): SendOutcome => {
        composer.note(sessionId, message);
        toasts.push({ kind: "error", bellKind: "failure", text: message });
        return { ok: false, kind: "command", message };
      };
      const invalid = (message: string): SendOutcome => {
        composer.note(sessionId, message);
        return { ok: false, kind: "command", message };
      };
      const lastUser = (): Obj | null => {
        const users = historyOf(sessionId)
          .map(obj)
          .filter((e): e is Obj => e?.["role"] === "user" && e["type"] === "message");
        return users.length > 0 ? (users[users.length - 1] ?? null) : null;
      };
      try {
        switch (name) {
          case "compact": {
            if (composer.store.get().compacting[sessionId] === true)
              return fail("Context compaction is already running.");
            const run = Symbol("compaction");
            compactionRuns.set(sessionId, run);
            composer.setCompacting(sessionId, true);
            try {
              await call("session/compact", {
                sessionId,
                ...(args === "" ? {} : { extraInstructions: args }),
              });
              return done();
            } finally {
              if (compactionRuns.get(sessionId) === run) {
                compactionRuns.delete(sessionId);
                composer.setCompacting(sessionId, false);
              }
            }
          }
          case "new":
          case "clear": {
            const result = await call("session/history/clear", { sessionId });
            const next = str(
              obj(obj(obj(result)?.["state"] ?? null)?.["session"] ?? null)?.["id"] ?? null,
            );
            // `/clear` hands the conversation to a new session id: follow it.
            if (next !== null && next !== sessionId) {
              await core.run({ type: "refreshSession", sessionId: next });
              shell.actions.selectSession(next);
            }
            return done();
          }
          case "rename":
            if (args === "") return invalid("Type the new name after /rename.");
            await sessions?.rename(sessionId, args);
            return done();
          case "rewind":
          case "branch": {
            const user = lastUser();
            const problem =
              user === null ? "There is no message to go back to." : anchorProblem(user);
            if (user === null || problem !== null) return invalid(problem ?? "Nothing to rewind.");
            const id = str(user["id"] ?? null) ?? "";
            if (name === "branch") {
              await rewind.branch(sessionId, id);
            } else {
              const text = arr(user["content"])
                .map((b) => str(obj(b)?.["text"] ?? null) ?? "")
                .join("");
              await rewind.openRewind(sessionId, id, text);
            }
            return done();
          }
          case "retry":
            return await deps.send().retry(sessionId, args);
          case "model":
          case "thinking":
            composer.setMenu({ sessionId, kind: "model" });
            return done();
          case "reload":
            await call("config/reload", { sessionId });
            composer.note(
              sessionId,
              "Config and skills reloaded. Changed AGENTS.md files reach this session when it reopens: Settings, Instructions, Apply.",
            );
            return done();
          case "status": {
            const result = await call("runtime/read", { sessionId });
            keepRuntime(sessionId, result);
            const runtime = obj(obj(result)?.["runtime"] ?? null);
            const model = str(
              obj(obj(runtime?.["config"] ?? null)?.["activeModel"] ?? null)?.["displayName"] ??
                null,
            );
            const agent = str(obj(runtime?.["activeAgent"] ?? null)?.["name"] ?? null);
            const window = runtime?.["contextWindow"];
            const tokens = obj(core.store.get().sessions[sessionId]?.session["tokenUsage"] ?? null);
            const total = tokens?.["totalTokens"];
            composer.note(
              sessionId,
              [
                `Model: ${model ?? "unknown"}`,
                `access: ${modeOf(agent)?.label ?? "unknown"}`,
                typeof window === "number" ? `compacts at ${formatTokens(window)} tokens` : null,
                typeof total === "number" ? `used so far: ${String(total)} tokens` : null,
              ]
                .filter((x) => x !== null)
                .join(" · "),
            );
            return done();
          }
          case "log": {
            const result = await call("session/log/read", { sessionId });
            const path = str(obj(obj(result)?.["log"] ?? null)?.["path"] ?? null);
            composer.note(
              sessionId,
              path === null ? "This session has no log file." : `Log: ${path}`,
            );
            return done();
          }
          case "copy": {
            const answers = historyOf(sessionId)
              .map(obj)
              .filter((e): e is Obj => e?.["role"] === "assistant");
            const last = answers[answers.length - 1];
            const text = arr(last?.["content"])
              .map((b) => str(obj(b)?.["text"] ?? null) ?? "")
              .join("");
            if (text === "") return invalid("There is no answer to copy yet.");
            await navigator.clipboard.writeText(text);
            return done();
          }
          case "help":
            shell.actions.setOverlay("palette");
            return done();
          case "plugins": {
            const groups = pluginGroups(await call("plugin/info", { sessionId }));
            composer.note(
              sessionId,
              groups.length === 0
                ? "This session runs no plugins."
                : `Plugins in this session: ${groups.map(([name, n]) => `${name} (${String(n)})`).join(", ")}.`,
            );
            return done();
          }
          case "reload-plugins": {
            // Vibe answers `{}`; the change is read from the session's components before and after (like the CLI).
            const before = pluginComponents(await call("plugin/info", { sessionId }));
            await call("plugin/reload", { sessionId });
            const after = pluginComponents(await call("plugin/info", { sessionId }));
            const added = after.filter((name) => !before.includes(name));
            const removed = before.filter((name) => !after.includes(name));
            composer.note(
              sessionId,
              added.length === 0 && removed.length === 0
                ? "Plugins reloaded. Nothing changed."
                : [
                    "Plugins reloaded.",
                    added.length > 0 ? `Added: ${added.join(", ")}.` : null,
                    removed.length > 0 ? `Removed: ${removed.join(", ")}.` : null,
                  ]
                    .filter((x) => x !== null)
                    .join(" "),
            );
            return done();
          }
          case "config":
            shell.actions.openSettings();
            return done();
          case "mcp":
          case "connectors":
            if (deps.openTools === undefined) return invalid("Tools are not available here.");
            deps.openTools();
            return done();
          case "resume":
          case "continue":
            shell.actions.closeSettings();
            shell.actions.showList();
            return done();
          case "todo":
            shell.actions.openTab("plan");
            return done();
          default:
            return invalid(`/${name} is not a command.`);
        }
      } catch (thrown) {
        return fail(failed(thrown));
      }
    },
  };
  return controls;
}

/** The session controls. */
export type SessionControls = ReturnType<typeof createSessionControls>;

/** The component names (`kind:name`) of a `plugin/info` reply, sorted. */
export function pluginComponents(reply: JsonValue | null): string[] {
  return arr(obj(obj(reply)?.["info"] ?? null)?.["components"])
    .map(obj)
    .map((c) => {
      const name = str(c?.["name"] ?? null);
      return name === null ? null : `${str(c?.["kind"] ?? null) ?? "component"} ${name}`;
    })
    .filter((x): x is string => x !== null)
    .sort();
}

/** The plugins of a `plugin/info` reply with their component count: the part before `:` of each component name. */
export function pluginGroups(reply: JsonValue | null): Array<[string, number]> {
  const counts = new Map<string, number>();
  for (const entry of pluginComponents(reply)) {
    const name = entry.slice(entry.indexOf(" ") + 1);
    const plugin = name.includes(":") ? name.slice(0, name.indexOf(":")) : name;
    counts.set(plugin, (counts.get(plugin) ?? 0) + 1);
  }
  return [...counts.entries()].sort(([a], [b]) => a.localeCompare(b));
}
