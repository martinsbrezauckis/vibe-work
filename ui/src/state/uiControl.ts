/**
 * UI-control routes: the Debug API's `ui/<name>` requests are answered here. Features
 * register a handler per name; the loop takes requests from the client transport (Tauri command
 * or the dev proxy), runs the handler against the UI's own state and replies.
 * Depends on: protocol/client types. Called by: the shell (registers `panel`, `state`, `theme`,
 * `list`, `session`; runs the loop), feature routes.
 * Invariants: a handler failure becomes an error reply (the Debug API caller gets a code and a
 * message, never a hung request); an unknown name is a `notFound` reply; the loop never throws
 * and backs off after a transport error; in the real app it only polls while the Debug API is on;
 * the reply is built only after the commands the request sent under its capability were answered by
 * the host, or after `DRAIN_MS`.
 */
import type {
  UiControlReply,
  UiControlRequest,
  UiControlTransport,
} from "../protocol/client/types";
import type { ErrorCode } from "../protocol/core";

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
 * Longest wait, after a handler returned, for the commands it started under its capability. Below the
 * Debug API's 10 s UI timeout; a command still running then keeps running, the reply just stops waiting for it.
 */
export const DRAIN_MS = 3_000;

/** Creates a registry of handlers. */
export function createUiControl() {
  const handlers = new Map<string, UiHandler>();
  // The capabilities of the requests being served right now (the newest last); while any is, the core
  // controller marks every command as the Debug API's and passes the capability (logs, refusal events
  // and the transcript say who acted; the host accepts the Debug API origin, which
  // acts as the user without native dialogs, only with the capability of a request still pending, so a script in
  // the window cannot claim it). A user click in that moment is marked too.
  const serving: string[] = [];
  // Commands sent with a request's capability that the host has not answered yet. The reply waits for
  // them: the host accepts the capability only while the request is pending, and Tauri does not keep invokes in
  // order, so a command sent before the reply could reach the host after it and be refused.
  const inflight = new Map<string, Set<Promise<unknown>>>();
  const drain = async (capability: string): Promise<void> => {
    const deadline = Date.now() + DRAIN_MS;
    for (;;) {
      const pending = inflight.get(capability);
      const left = deadline - Date.now();
      if (pending === undefined || pending.size === 0 || left <= 0) return;
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
    /**
     * Records a command the core controller sent with `capability`; the request's reply waits for it (bounded by
     * [`DRAIN_MS`]). Returns `promise` unchanged.
     */
    track<T>(capability: string, promise: Promise<T>): Promise<T> {
      let set = inflight.get(capability);
      if (set === undefined) {
        set = new Set();
        inflight.set(capability, set);
      }
      const entry: Promise<unknown> = promise.then(
        () => undefined,
        () => undefined,
      );
      set.add(entry);
      void entry.then(() => {
        set.delete(entry);
        if (set.size === 0 && inflight.get(capability) === set) inflight.delete(capability);
      });
      return promise;
    },
    /** True while a UI-control request is being served. */
    serving(): boolean {
      return serving.length > 0;
    },
    /**
     * The capability of the request being served (the newest when several are), `null` when none is. A request
     * from a transport without capabilities (a browser UI) gives `""`, which the app's host would refuse.
     */
    servingCapability(): string | null {
      return serving.length > 0 ? (serving[serving.length - 1] ?? "") : null;
    },
    /** Registers `name`; returns the function that removes it. Throws on a duplicate. */
    register(name: string, handler: UiHandler): () => void {
      if (handlers.has(name)) throw new Error(`ui route ${name} is already registered`);
      handlers.set(name, handler);
      return () => {
        handlers.delete(name);
      };
    },
    /** Registered route names. */
    names(): readonly string[] {
      return [...handlers.keys()];
    },
    /** Runs one request and builds its reply. */
    async handle(request: UiControlRequest): Promise<UiControlReply> {
      const handler = handlers.get(request.name);
      if (handler === undefined) {
        return {
          id: request.id,
          ok: false,
          error: { code: "notFound", message: `the UI has no route ${request.name}` },
        };
      }
      const capability = request.capability ?? "";
      serving.push(capability);
      let reply: UiControlReply;
      try {
        reply = { id: request.id, ok: true, value: await handler(request.body) };
      } catch (thrown) {
        if (thrown instanceof UiControlError) {
          reply = {
            id: request.id,
            ok: false,
            error: { code: thrown.code, message: thrown.message },
          };
        } else {
          const message = thrown instanceof Error ? thrown.message : "the UI route failed";
          reply = { id: request.id, ok: false, error: { code: "internal", message } };
        }
      }
      try {
        // Still serving while it waits: a command a pending one starts is marked and waited for too.
        await drain(capability);
      } finally {
        serving.splice(serving.lastIndexOf(capability), 1);
      }
      return reply;
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
      if (request !== null) await transport.reply(await control.handle(request));
    } catch {
      await sleep(1500);
    }
  }
}
