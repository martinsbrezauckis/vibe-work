/**
 * UI-control routes: the Debug API's `ui/<name>` requests are answered here. Features
 * register a handler per name; the loop takes requests from the client transport (Tauri command
 * or the dev proxy), runs the handler against the UI's own state and replies.
 * Depends on: protocol/client types. Called by: the shell (registers `panel`, `state`, `theme`,
 * `list`, `session`; runs the loop), feature routes.
 * Invariants: a handler failure becomes an error reply (the Debug API caller gets a code and a
 * message, never a hung request); an unknown name is a `notFound` reply; the loop never throws
 * and backs off after a transport error; in the real app it only polls while the Debug API is on;
 * slow handlers and tracked commands acknowledge within `REPLY_MS` with a pending id; their final reply follows
 * on the same transport. Pure state reads remain available; another action answers busy until completion, for at
 * most `ACTION_SLOT_MS`: a wait nobody ends (a native dialog with no person at it) lets go then, as the host forgets
 * the request.
 * Authority: none lives here. A request carries only its id, route and body; the commands a handler sends are the
 * window's, like a click's (the host decides in Rust which few of them a request's grants let run as the Debug API,
 * `crates/vw-host/src/ui_grants.rs`). The action key below only groups an action's commands so its reply waits for
 * them; it is never sent anywhere.
 */
import type {
  UiControlReply,
  UiControlRequest,
  UiControlTransport,
} from "../protocol/client/types";
import { createCommandTrace } from "./commandTrace";
import { createWalkthrough } from "./walkthrough";
import type { CoreError, ErrorCode } from "../protocol/core";
import { toCallError } from "../protocol/client/types";

/** A handler's failure with a machine-readable code (the Debug API maps it to a status). */
export class UiControlError extends Error {
  readonly code: ErrorCode;

  constructor(code: ErrorCode, message: string) {
    super(message);
    this.code = code;
  }
}

/** A route handler: body in, JSON-serializable result out. */
export type UiHandler = (body: unknown) => unknown;

/**
 * Drain interval for tracked commands. Direct callers have a bounded drain; the transport acknowledges slow
 * work and keeps draining in the background until all of the action's commands have finished.
 */
export const DRAIN_MS = 3_000;
/** A slow action acknowledges within this budget and completes through the same transport later. */
export const REPLY_MS = 50;
/**
 * How long an acknowledged action keeps the one busy slot. The host forgets the request after `JOB_LIMIT`
 * (`crates/vw-host/src/bridge.rs`, ten minutes, started a moment after this one) and discards a later answer; the
 * window lets go no later, or every other action would answer busy until the app restarts. A test keeps the two in
 * step.
 */
export const ACTION_SLOT_MS = 10 * 60_000;

/** Creates a registry of handlers. */
export function createUiControl() {
  const trace = createCommandTrace();
  let clicked: string | null = null;
  let requestTrigger: string | null = null;
  const handlers = new Map<string, { handler: UiHandler; readOnly: (body: unknown) => boolean }>();
  let active: string | null = null;
  // The actions being served right now (page-local keys, the newest last). Commands the app sends meanwhile are
  // recorded under the newest one so its reply follows their answers (a Debug API client that reads right after
  // the reply sees what the action did). The key carries no authority and never leaves the page.
  const serving: string[] = [];
  let nextAction = 0;
  // Commands of an action that the host has not answered yet; the reply waits for them (bounded by `DRAIN_MS`).
  const inflight = new Map<string, Set<Promise<unknown>>>();
  // Whole actions include deferred commands, such as the prepare/show native menu handoff.
  const actions = new Map<string, Set<Promise<unknown>>>();
  const failures = new Map<string, CoreError>();
  // `over` turns true once the action let go (its slot ended): the wait stops, so no timer runs on for it.
  const drain = async (action: string, bounded: boolean, over: () => boolean): Promise<void> => {
    const deadline = Date.now() + DRAIN_MS;
    for (;;) {
      const pending = inflight.get(action);
      const actionPending = (actions.get(action)?.size ?? 0) > 0;
      const left = bounded && !actionPending ? deadline - Date.now() : DRAIN_MS;
      if (over() || pending === undefined || pending.size === 0 || left <= 0) return;
      let timer: ReturnType<typeof setTimeout> | undefined;
      await Promise.race([
        Promise.allSettled([...pending]),
        new Promise<void>((resolve) => {
          timer = setTimeout(resolve, left);
        }),
      ]);
      clearTimeout(timer);
    }
  };
  return {
    walkthrough: createWalkthrough(),
    trace,
    /** Marks a click for commands dispatched during the same turn; later unrelated work has no trigger. */
    clickTrigger(testId: string | null): void {
      clicked = testId;
      if (serving.length > 0) requestTrigger = testId;
      queueMicrotask(() => {
        clicked = null;
      });
    },
    /** The active click, including awaits while a Debug API click request is still pending. */
    trigger(): string | null {
      return clicked ?? requestTrigger;
    },
    /**
     * Records a command the core controller sent while `action` was served; the action's reply waits for it (bounded
     * by [`DRAIN_MS`]). Whole actions (`untilComplete`) keep the reply open through deferred commands.
     * Returns `promise` unchanged.
     */
    track<T>(action: string, promise: Promise<T>, untilComplete = false): Promise<T> {
      let set = inflight.get(action);
      if (set === undefined) {
        set = new Set();
        inflight.set(action, set);
      }
      const entry: Promise<unknown> = promise.then(
        () => undefined,
        (thrown: unknown) => {
          if (serving.includes(action) && !failures.has(action))
            failures.set(action, toCallError(thrown).error);
        },
      );
      set.add(entry);
      let actionSet = actions.get(action);
      if (untilComplete) {
        if (actionSet === undefined) {
          actionSet = new Set();
          actions.set(action, actionSet);
        }
        actionSet.add(entry);
      }
      void entry.then(() => {
        set.delete(entry);
        actionSet?.delete(entry);
        if (actionSet?.size === 0 && actions.get(action) === actionSet) actions.delete(action);
        if (set.size === 0 && inflight.get(action) === set) inflight.delete(action);
      });
      return promise;
    },
    /** True while a UI-control request is being served. */
    serving(): boolean {
      return serving.length > 0;
    },
    /** The page-local key of the action being served (the newest when several are), `null` when none is. */
    servingAction(): string | null {
      return serving.length > 0 ? (serving[serving.length - 1] ?? null) : null;
    },
    /** Registers `name`; returns the function that removes it. Throws on a duplicate. */
    register(
      name: string,
      handler: UiHandler,
      readOnly: (body: unknown) => boolean = () => false,
    ): () => void {
      if (handlers.has(name)) throw new Error(`ui route ${name} is already registered`);
      handlers.set(name, { handler, readOnly });
      return () => {
        handlers.delete(name);
      };
    },
    /** Registered route names. */
    names(): readonly string[] {
      return [...handlers.keys()];
    },
    /** Runs one request and builds its reply. */
    async handle(
      request: UiControlRequest,
      completed?: (reply: UiControlReply) => Promise<void>,
    ): Promise<UiControlReply> {
      const route = handlers.get(request.name);
      if (route === undefined) {
        return {
          id: request.id,
          ok: false,
          error: { code: "notFound", message: `the UI has no route ${request.name}` },
        };
      }
      const readOnly = route.readOnly(request.body);
      if (active !== null && !readOnly) {
        return {
          id: request.id,
          ok: true,
          value: { pendingId: active, state: "busy", poll: "/ui/requests" },
        };
      }
      nextAction += 1;
      const action = `action-${String(nextAction)}`;
      // A pure read does not collect commands for its reply.
      if (!readOnly) serving.push(action);
      // Gives up the busy slot. Runs once: when the action finishes or when its slot time ends.
      let held = true;
      const release = (): void => {
        if (!held) return;
        held = false;
        if (!readOnly) {
          serving.splice(serving.lastIndexOf(action), 1);
          if (serving.length === 0) requestTrigger = null;
        }
        failures.delete(action);
        if (active === request.id) active = null;
      };
      const work = (async (): Promise<UiControlReply> => {
        let reply: UiControlReply;
        try {
          reply = { id: request.id, ok: true, value: await route.handler(request.body) };
        } catch (thrown) {
          const error = toCallError(thrown).error;
          reply = {
            id: request.id,
            ok: false,
            error: {
              code: error.code,
              message: error.message,
              ...(error.serverCode === null ? {} : { serverCode: error.serverCode }),
            },
          };
        }
        try {
          // Still serving while it waits: a command a pending one starts is marked and waited for too.
          if (!readOnly) await drain(action, completed === undefined, () => !held);
          const failure = failures.get(action);
          if (request.name === "click" && reply.ok && failure !== undefined)
            reply = {
              id: request.id,
              ok: false,
              error: { code: failure.code, message: failure.message },
            };
        } finally {
          release();
        }
        return reply;
      })();
      if (completed === undefined) return work;
      let timer: ReturnType<typeof setTimeout> | undefined;
      const reply = await Promise.race([
        work,
        new Promise<null>((resolve) => {
          timer = setTimeout(() => {
            resolve(null);
          }, REPLY_MS);
        }),
      ]);
      clearTimeout(timer);
      if (reply !== null) return reply;
      if (!readOnly) active = request.id;
      // The wait may never end (a native dialog nobody answers). At the slot's end the window lets go and drops the
      // late answer: the host no longer has the request, so it would refuse it anyway.
      let expired = false;
      const slot = setTimeout(() => {
        expired = true;
        release();
      }, ACTION_SLOT_MS);
      void work
        .then((done) => {
          clearTimeout(slot);
          return expired ? undefined : completed(done);
        })
        .catch(() => undefined);
      return {
        id: request.id,
        ok: true,
        pending: true,
        value: { pendingId: request.id, state: "pending", poll: "/ui/requests" },
      };
    },
  };
}

/** The registry the app uses. */
export type UiControl = ReturnType<typeof createUiControl>;

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/**
 * Serves requests from `transport` until `signal` aborts. `isActive` is checked before every
 * poll (the real app polls only while the Debug API is on).
 */
export async function runUiControl(
  transport: UiControlTransport,
  control: UiControl,
  isActive: () => boolean,
  signal: AbortSignal,
  waitMs = 25_000,
): Promise<void> {
  while (!signal.aborted) {
    if (!isActive()) {
      await sleep(1000);
      continue;
    }
    try {
      const request = await transport.next(waitMs);
      if (request !== null) {
        let acknowledge!: () => void;
        const acknowledged = new Promise<void>((resolve) => {
          acknowledge = resolve;
        });
        const reply = await control.handle(request, async (done) => {
          await acknowledged;
          for (;;) {
            try {
              await transport.reply(done);
              return;
            } catch {
              if (signal.aborted) return;
              await sleep(1500);
            }
          }
        });
        try {
          await transport.reply(reply);
        } finally {
          acknowledge();
        }
      }
    } catch {
      await sleep(1500);
    }
  }
}
