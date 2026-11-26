/**
 * Config problems Vibe reports, shown once each in the bell instead of being swallowed: a skill, hook or tool file
 * Vibe could not load (`runtime.issues`, file and message), a config warning (`runtime.config.validationWarnings`),
 * a project config folder Vibe ignores because it is marked untrusted (`workspace/trust/untrustedConfig`), and a model
 * the organization's admin config enforces (`config/fields/read`: `active_model` whose top layer is `admin`).
 * Depends on: coreStore (the sessions' runtime snapshots and the typed `call` command), toastStore, the vocabulary,
 * thinkingLevels (the retired Large 4 alias rule), keyStore/keyReadiness (the composer's key guard).
 * Called by: createApp (wiring), useShellWiring (`start`).
 * Invariants:
 * - One plain line per problem, naming the file (or folder, or model) and what is wrong; never explanatory prose.
 * - Each problem is one bell entry, shown once per run, that stays until the person dismisses it (Dismiss or Clear
 *   all); a dismissed problem (kind, file, message) is remembered and, when the app persists settings, never shown
 *   again, also in later runs. Closing its toast is not dismissing it. A changed message is a new problem.
 * - One alert per model: a warning about a model that a model-fallback alert of this run names is not shown, and
 *   leaves the bell (without counting as dismissed) when that alert arrives after it.
 * - Vibe's warning about Vibe Work's retired `large-4` alias is not shown while the list offers the one catalog Large 4
 *   a chat on that alias is put back on (`retiredAliasSettled`); Core alerts itself if that fails.
 * - Model warnings wait while the Mistral key is unread or unusable; a later usable key reads each attached chat's
 *   runtime once and waits for Core's checked restore outcome before judging warnings hidden during the lock.
 * - Reads only sessions that already have a runtime (their channel is open); the trust read runs once per folder
 *   and the admin read once per app run. A failed read shows nothing (it is retried on a later run).
 */
import type { JsonValue } from "../protocol/core";
import { TEXT, wording } from "../vocabulary";
import type { CoreController } from "./coreStore";
import type { Toasts } from "./toastStore";
import { retiredAliasSettled } from "./thinkingLevels";
import type { KeyController } from "./keyStore";
import { keyReadiness } from "./keyReadiness";

const KEY = "vw.config.problems.dismissed";
/** Vibe's warnings about an unavailable or excluded model (`vibe_schema.py`), which name its alias. */
const MODEL_WARNING = /^(?:Active model|Model) '([^']+)'/;

/** True when `message` names `name` as a whole word. */
function names(message: string, name: string): boolean {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`(?:^|[^\\p{L}\\p{N}])${escaped}(?=$|[^\\p{L}\\p{N}])`, "u").test(message);
}
/** Dismissed problems remembered at most (the oldest are forgotten first). */
const MAX_DISMISSED = 500;

/** One problem as shown. */
export interface ConfigProblem {
  /** Stable identity: kind, file and message. */
  id: string;
  text: string;
  /** For a warning about a model: the model's alias and display name (the fallback alert may name either). */
  models?: readonly string[];
}

type Obj = Readonly<Record<string, JsonValue | undefined>>;

function obj(value: JsonValue | undefined): Obj | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : null;
}

function list(value: JsonValue | undefined): readonly JsonValue[] {
  return Array.isArray(value) ? value : [];
}

/** Vibe's text on one line (a multi-line message keeps its words). */
function oneLine(text: string): string {
  return text.replace(/\s*\n\s*/g, " ").trim();
}

/** A warning already owned by the config bell, including a file-prefixed issue. */
export function isConfigProblem(runtime: JsonValue | undefined, message: string): boolean {
  const root = obj(runtime);
  const line = oneLine(message);
  return (
    list(obj(root?.["config"])?.["validationWarnings"]).some(
      (warning) => typeof warning === "string" && oneLine(warning) === line,
    ) ||
    list(root?.["issues"]).some((value) => {
      const issue = obj(value);
      const text = issue?.["message"];
      if (typeof text !== "string") return false;
      const file = typeof issue?.["file"] === "string" ? issue["file"] : "";
      return oneLine(text) === line || oneLine(`${file}: ${text}`) === line;
    })
  );
}

/**
 * True for Vibe's warning that the retired `large-4` alias is not available while `runtime`'s list offers the one
 * catalog Large 4 Core puts such a chat back on without a notice (`retiredAliasSettled`): no bell entry, no notice.
 */
export function settledModelWarning(runtime: JsonValue | undefined, message: string): boolean {
  const alias = MODEL_WARNING.exec(oneLine(message))?.[1];
  const models = list(obj(obj(runtime)?.["config"])?.["models"]).map(obj);
  return (
    alias !== undefined &&
    (retiredAliasSettled(alias, models) || checkedActiveModelWarning(runtime, alias, models))
  );
}

/** A completed Core restore and Vibe's effective model/catalog settle an earlier fallback warning. */
function checkedActiveModelWarning(
  runtime: JsonValue | undefined,
  alias: string,
  models: readonly (Obj | null)[],
): boolean {
  const root = obj(runtime);
  if (root?.["modelChoiceRestore"] !== "checked") return false;
  const active = obj(obj(root["config"])?.["activeModel"]);
  const name = active?.["name"];
  const thinking = active?.["thinking"];
  if (
    active?.["alias"] !== alias ||
    typeof name !== "string" ||
    name === "" ||
    typeof thinking !== "string" ||
    thinking === ""
  )
    return false;
  return models.some(
    (model) =>
      model?.["alias"] === alias &&
      model["name"] === name &&
      list(model["thinkingLevels"]).includes(thinking),
  );
}

function problem(kind: string, file: string, message: string, text: string): ConfigProblem {
  return { id: JSON.stringify([kind, file, message]), text: oneLine(text) };
}

/** The problems in one runtime snapshot: Vibe's config issues, then its config warnings. */
export function runtimeProblems(runtime: JsonValue | undefined, keyUsable = true): ConfigProblem[] {
  const root = obj(runtime);
  const issues = list(root?.["issues"]).flatMap((value) => {
    const issue = obj(value);
    const message = issue?.["message"];
    if (typeof message !== "string" || message.trim() === "") return [];
    const file = typeof issue?.["file"] === "string" ? issue["file"].trim() : "";
    return [
      problem(
        "issue",
        file,
        message,
        file === "" ? message : wording(TEXT.configProblem, { file, message }),
      ),
    ];
  });
  const config = obj(root?.["config"]);
  const warnings = list(config?.["validationWarnings"]).flatMap((warning) => {
    if (typeof warning !== "string" || warning.trim() === "") return [];
    const shown = problem(
      "warning",
      "",
      warning,
      wording(TEXT.configWarning, { message: warning }),
    );
    const alias = MODEL_WARNING.exec(warning)?.[1];
    if (alias === undefined) return [shown];
    if (!keyUsable) return [];
    if (settledModelWarning(runtime, warning)) return [];
    const display = list(config?.["models"])
      .map(obj)
      .find((model) => model?.["alias"] === alias)?.["displayName"];
    return [
      {
        ...shown,
        models: typeof display === "string" && display !== "" ? [alias, display] : [alias],
      },
    ];
  });
  return [...issues, ...warnings];
}

/** The folders in a `workspace/trust/untrustedConfig` reply. */
export function untrustedProblems(reply: JsonValue | undefined): ConfigProblem[] {
  return list(obj(reply)?.["dirs"]).flatMap((dir) =>
    typeof dir === "string" && dir !== ""
      ? [problem("untrusted", dir, "", wording(TEXT.configUntrusted, { folder: dir }))]
      : [],
  );
}

/** The admin-enforced model in a `config/fields/read` reply, if the organization's config sets it. */
export function adminModelProblem(reply: JsonValue | undefined): ConfigProblem | null {
  const field = list(obj(reply)?.["fields"])
    .map(obj)
    .find((f) => f?.["name"] === "active_model" || f?.["path"] === "active_model");
  const top = obj(list(field?.["layerValues"])[0]);
  const model = top?.["value"];
  if (top?.["layer"] !== "admin" || typeof model !== "string" || model === "") return null;
  return problem("admin", "active_model", model, wording(TEXT.configAdminModel, { model }));
}

function loadDismissed(persist: boolean): string[] {
  if (!persist) return [];
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(saved) ? saved.filter((id): id is string => typeof id === "string") : [];
  } catch {
    return [];
  }
}

/** The controller: watches the core store, pushes each new problem once per run, remembers dismissals. */
export function createConfigNotices(deps: {
  core: CoreController;
  toasts: Toasts;
  keys: KeyController | null;
  persist: boolean;
}) {
  const dismissed = new Set(loadDismissed(deps.persist));
  // Shown in this run: problem id by bell entry id.
  const shown = new Map<number, ConfigProblem>();
  // Messages of the model-fallback alerts in this run: one alert per model, so a warning about a model one of
  // them names is not shown, and leaves the bell when such an alert arrives after it.
  const fallbacks: string[] = [];
  const alerted = (p: ConfigProblem): boolean =>
    p.models?.some((model) => fallbacks.some((message) => names(message, model))) === true;
  const shownIds = new Set<string>();
  const folders = new Set<string>();
  let adminRead = false;
  const keyUsable = (): boolean => {
    const state = deps.keys?.store.get();
    return (
      state === undefined || (state.availability !== "loading" && keyReadiness(state) === null)
    );
  };

  const show = (problems: readonly ConfigProblem[]): void => {
    for (const p of problems) {
      if (dismissed.has(p.id) || shownIds.has(p.id) || alerted(p)) continue;
      shownIds.add(p.id);
      shown.set(deps.toasts.push({ kind: "warn", bellKind: "failure", text: p.text }), p);
    }
  };

  const remember = (ids: readonly number[]): void => {
    let changed = false;
    for (const id of ids) {
      const problem = shown.get(id);
      if (problem === undefined) continue;
      shown.delete(id);
      dismissed.add(problem.id);
      changed = true;
    }
    if (!changed || !deps.persist) return;
    try {
      localStorage.setItem(KEY, JSON.stringify([...dismissed].slice(-MAX_DISMISSED)));
    } catch {
      /* The app run still keeps the dismissed set. */
    }
  };

  const call = async (
    method: string,
    params: JsonValue,
    freshRuntimeRead = false,
  ): Promise<JsonValue | undefined> => {
    try {
      const reply = await deps.core.run(
        { type: "call", method, params },
        { origin: "window" },
        freshRuntimeRead,
      );
      return reply.type === "reply" ? reply.result : undefined;
    } catch {
      return undefined;
    }
  };

  // The store changes on every event; a runtime snapshot already looked at is not scanned again.
  const read = new WeakSet<object>();
  // One original runtime per chat and key transition. Only a later Core-checked runtime settles it.
  const awaiting = new Map<string, JsonValue>();
  const scan = (): void => {
    for (const entry of Object.values(deps.core.store.get().sessions)) {
      const runtime = entry.runtime;
      if (typeof runtime !== "object" || runtime === null) continue;
      if (
        awaiting.has(entry.id) &&
        awaiting.get(entry.id) !== runtime &&
        obj(runtime)?.["modelChoiceRestore"] === "checked"
      ) {
        awaiting.delete(entry.id);
        read.delete(runtime); // This object may have been scanned during the lock.
      }
      if (read.has(runtime)) continue;
      read.add(runtime);
      show(
        runtimeProblems(
          runtime,
          keyUsable() &&
            !awaiting.has(entry.id) &&
            obj(runtime)?.["modelChoiceRestore"] !== "pending",
        ),
      );
      const cwd = entry.session["cwd"];
      if (typeof cwd === "string" && cwd !== "" && !folders.has(cwd)) {
        folders.add(cwd);
        void call("workspace/trust/untrustedConfig", { cwd }).then((reply) => {
          show(untrustedProblems(reply));
        });
      }
      if (!adminRead) {
        adminRead = true;
        void call("config/fields/read", { sessionId: entry.id }).then((reply) => {
          const admin = adminModelProblem(reply);
          if (admin !== null) show([admin]);
        });
      }
    }
  };

  let keyEpoch = 0;
  const readAfterUnlock = (epoch: number): void => {
    for (const entry of Object.values(deps.core.store.get().sessions)) {
      const runtime = entry.runtime;
      if (typeof runtime !== "object" || runtime === null) continue;
      awaiting.set(entry.id, runtime);
      // A single read per attached chat and key transition, including an idle chat with no new runtime event.
      void call("runtime/read", { sessionId: entry.id }, true).then((reply) => {
        if (epoch !== keyEpoch || !keyUsable()) return;
        const current = deps.core.store.get().sessions[entry.id];
        if (current === undefined) return;
        if (
          current.runtime !== runtime &&
          obj(current.runtime)?.["modelChoiceRestore"] === "checked"
        )
          return; // A new Core-checked runtime event already won this race.
        const response = obj(reply);
        const outcome = response?.["modelChoiceRestore"];
        const fresh = obj(response?.["runtime"]);
        if (
          fresh === null ||
          (outcome !== "checked" && outcome !== "pending") ||
          (fresh["modelChoiceRestore"] !== undefined && fresh["modelChoiceRestore"] !== outcome)
        )
          return;
        const tagged =
          fresh["modelChoiceRestore"] === outcome
            ? fresh
            : { ...fresh, modelChoiceRestore: outcome };
        if (outcome === "checked") awaiting.delete(entry.id);
        read.delete(tagged); // A fresh reply counts even when its object identity matches a cached snapshot.
        deps.core.setRuntime(entry.id, tagged);
      });
    }
  };

  return {
    /** Starts watching; returns the function that stops it. */
    start(): () => void {
      let wasUsable = keyUsable();
      const stopDismissed = deps.toasts.onDismissed(remember);
      const stopAlerts = deps.core.listen((event) => {
        if (event.type !== "alert" || event.kind !== "modelChoiceNotRestored") return;
        fallbacks.push(event.message);
        for (const [entry, problem] of [...shown]) {
          if (!alerted(problem)) continue;
          // Not the person's dismissal: taken out of `shown` first, so it is not remembered as dismissed.
          shown.delete(entry);
          deps.toasts.forget(entry);
        }
      });
      scan();
      const stopScan = deps.core.store.subscribe(scan);
      const stopKeys = deps.keys?.store.subscribe(() => {
        const usable = keyUsable();
        if (usable === wasUsable) return;
        wasUsable = usable;
        const epoch = ++keyEpoch;
        if (usable) {
          readAfterUnlock(epoch);
          return;
        }
        awaiting.clear();
        for (const [entry, problem] of shown) {
          if (problem.models === undefined) continue;
          // A lock is not dismissal. A fresh runtime can report this problem again after unlock.
          shown.delete(entry);
          shownIds.delete(problem.id);
          deps.toasts.forget(entry);
        }
      });
      return () => {
        keyEpoch += 1;
        stopDismissed();
        stopAlerts();
        stopScan();
        stopKeys?.();
      };
    },
  };
}

/** The controller the app holds. */
export type ConfigNotices = ReturnType<typeof createConfigNotices>;
