/**
 * Config problems Vibe reports, shown once each in the bell instead of being swallowed: a skill, hook or tool file
 * Vibe could not load (`runtime.issues`, file and message), a config warning (`runtime.config.validationWarnings`),
 * a project config folder Vibe ignores because it is marked untrusted (`workspace/trust/untrustedConfig`), and a model
 * the organization's admin config enforces (`config/fields/read`: `active_model` whose top layer is `admin`).
 * Depends on: coreStore (the sessions' runtime snapshots and the typed `call` command), toastStore, the vocabulary.
 * Called by: createApp (wiring), useShellWiring (`start`).
 * Invariants:
 * - One plain line per problem, naming the file (or folder, or model) and what is wrong; never explanatory prose.
 * - Each problem is one bell entry, shown once per run, that stays until the person dismisses it (Dismiss or Clear
 *   all); a dismissed problem (kind, file, message) is remembered and, when the app persists settings, never shown
 *   again, also in later runs. Closing its toast is not dismissing it. A changed message is a new problem.
 * - One alert per model: a warning about a model that a model-fallback alert of this run names is not shown, and
 *   leaves the bell (without counting as dismissed) when that alert arrives after it.
 * - Reads only sessions that already have a runtime (their channel is open); the trust read runs once per folder
 *   and the admin read once per app run. A failed read shows nothing (it is retried on a later run).
 */
import type { JsonValue } from "../protocol/core";
import { TEXT, wording } from "../vocabulary";
import type { CoreController } from "./coreStore";
import type { Toasts } from "./toastStore";

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

function problem(kind: string, file: string, message: string, text: string): ConfigProblem {
  return { id: JSON.stringify([kind, file, message]), text: oneLine(text) };
}

/** The problems in one runtime snapshot: Vibe's config issues, then its config warnings. */
export function runtimeProblems(runtime: JsonValue | undefined): ConfigProblem[] {
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

  const call = async (method: string, params: JsonValue): Promise<JsonValue | undefined> => {
    try {
      const reply = await deps.core.run({ type: "call", method, params }, { origin: "window" });
      return reply.type === "reply" ? reply.result : undefined;
    } catch {
      return undefined;
    }
  };

  // The store changes on every event; a runtime snapshot already looked at is not read again.
  const read = new WeakSet<object>();
  const scan = (): void => {
    for (const entry of Object.values(deps.core.store.get().sessions)) {
      const runtime = entry.runtime;
      if (typeof runtime !== "object" || runtime === null || read.has(runtime)) continue;
      read.add(runtime);
      show(runtimeProblems(runtime));
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

  return {
    /** Starts watching; returns the function that stops it. */
    start(): () => void {
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
      return () => {
        stopDismissed();
        stopAlerts();
        stopScan();
      };
    },
  };
}

/** The controller the app holds. */
export type ConfigNotices = ReturnType<typeof createConfigNotices>;
