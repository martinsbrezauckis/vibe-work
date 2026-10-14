/**
 * The Connectors page's state and actions:
 * the page view from the core's `connectors` command, the filter and search, the selected tile, the add
 * dialog, and one sign-in at a time (`connectorSignIn.ts`). Sign-in pages open only through the link dialog
 * (`transcript.askLink`) and from there in the system browser; the app never loads them itself.
 * Depends on: coreStore (`run`, `listen`), shellStore (selected session, closes the page on a selection),
 * transcriptStore (link dialog), toastStore, connectorSignIn, the connector model. Called by: createApp,
 * the connector views, the `ui/connectors` route.
 * Invariants: every command a Debug API route triggers carries the Debug API origin
 * (`actions("debug-api")`), which the core records without showing a native dialog. Nothing here holds a token
 * (the core never sends one).
 */
import type {
  ConnectorsRequest,
  ConnectorsResult,
  ConnectorsView,
  NewServerSecret,
} from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { Filter } from "../features/connectors/model/view";
import { summarize } from "./sessionSummary";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import type { Shell } from "./shellStore";
import type { Toasts } from "./toastStore";
import type { Transcript } from "./transcriptStore";
import { type SignIn, type Timers, createSignIns } from "./connectorSignIn";

export type { SignIn } from "./connectorSignIn";

/** Everything the page shows. */
export interface ConnectorsState {
  /** The page is shown in the content area. */
  open: boolean;
  view: ConnectorsView | null;
  loading: boolean;
  /** Why the last read failed, in Vibe's or the core's words. */
  error: string | null;
  filter: Filter;
  search: string;
  /** `c:<alias>` or `s:<server name>`. */
  selected: string | null;
  /** The add dialog is open. */
  addOpen: boolean;
  /** What the add dialog starts with (a marketplace entry that needs a key), or `null`. */
  addPrefill: AddPrefill | null;
  /** The sign-in in progress (one at a time). */
  signIn: SignIn | null;
  /** Sign-ins Vibe asked for during a session's turn (not started here). */
  prompts: readonly SignIn[];
  /** A short outcome line ("Linear is turned off."). */
  note: string | null;
}

/** The add dialog's starting values. */
export interface AddPrefill {
  url: string;
  name: string;
  /** Start with "This server needs an API key" ticked. */
  key: boolean;
}

/** The key part of an `addKeyServer` request: an existing vault secret or a new one, and the header. */
export interface KeyChoice {
  secretId: string | null;
  newSecret: NewServerSecret | null;
  /** `null`: `Authorization: Bearer <key>`. */
  header: string | null;
}

const INITIAL: ConnectorsState = {
  open: false,
  view: null,
  loading: false,
  error: null,
  filter: "all",
  search: "",
  selected: null,
  addOpen: false,
  addPrefill: null,
  signIn: null,
  prompts: [],
  note: null,
};

/** The session a Mistral sign-in goes through: the shown one, else the most recently active one. */
export function contextSession(core: CoreController, shell: Shell): string | null {
  const selected = shell.store.get().selected;
  if (selected !== null) return selected;
  const state = core.store.get();
  const newest = Object.values(state.sessions)
    .map((entry) => summarize(entry, state.stuck))
    .sort((a, b) => b.bumpedAt - a.bumpedAt)[0];
  return newest?.id ?? null;
}

/** Creates the connectors controller. */
export function createConnectors(deps: {
  core: CoreController;
  shell: Shell;
  transcript: Transcript;
  toasts: Toasts;
  timers?: Timers;
  /** Hosted actions need a usable model key; local MCP reads remain available. */
  canUseKey?: () => boolean;
  /** Key-backed MCP grants need an unlocked vault, independent of model key readiness. */
  canUseVault?: () => boolean;
}) {
  const { core, shell, transcript, toasts } = deps;
  const store = createStore<ConnectorsState>(INITIAL);
  const patch = (p: Partial<ConnectorsState>) => {
    store.set((s) => ({ ...s, ...p }));
  };
  const canUseKey = deps.canUseKey ?? (() => true);
  const canUseVault = deps.canUseVault ?? (() => true);

  /** One `connectors` request with `origin`. */
  const request = async (
    origin: "ui" | "debug-api",
    req: ConnectorsRequest,
  ): Promise<ConnectorsResult> => {
    if (!canUseKey() && ["refresh", "signIn", "setConnectorOff"].includes(req.op))
      throw new Error("Mistral key required for hosted connectors");
    const result = await core.run(
      { type: "connectors", request: req },
      origin === "debug-api" ? { origin } : undefined,
    );
    if (result.type !== "connectors") throw new Error("unexpected answer to a connectors request");
    return result.result;
  };

  /** Shows a request's view, or its error in the page. */
  const show = (result: ConnectorsResult): void => {
    if (result.kind === "view" || result.kind === "serverAdded")
      patch({ view: result.view, error: null });
  };
  const fail = (thrown: unknown, req?: ConnectorsRequest): string => {
    const error = toCallError(thrown).error;
    const hosted = req !== undefined && ["refresh", "signIn", "setConnectorOff"].includes(req.op);
    if (error.code === "keyService" || (hosted && !canUseKey())) {
      patch({ note: null });
      return error.message;
    }
    // Declining the native dialog is the user's own answer: no error line.
    // The core's sentences start lower-case (they also go to logs); the page shows them as sentences.
    const sentence = error.message.charAt(0).toUpperCase() + error.message.slice(1);
    if (error.code !== "confirmationDeclined") patch({ note: sentence });
    return error.message;
  };

  /**
   * Reads the view with the context session's live state. Every change and every finished sign-in ends
   * with it: a change's own answer is read without a session (the change went to the lobby), so showing
   * it would drop the session's statuses.
   */
  const reload = async (origin: "ui" | "debug-api"): Promise<void> => {
    patch({ loading: true });
    const sessionId = contextSession(core, shell);
    try {
      show(await request(origin, { op: "read", ...(sessionId === null ? {} : { sessionId }) }));
    } catch (thrown) {
      const error = toCallError(thrown).error;
      patch({ error: error.code === "keyService" ? null : error.message });
    } finally {
      patch({ loading: false });
    }
  };

  const signIns = createSignIns({
    core,
    store,
    transcript,
    toasts,
    request,
    show,
    reload: () => reload("ui"),
    canUseKey,
    ...(deps.timers === undefined ? {} : { timers: deps.timers }),
  });

  const actions = (origin: "ui" | "debug-api") => {
    const run = async (req: ConnectorsRequest, note: string | null): Promise<boolean> => {
      try {
        show(await request(origin, req));
        patch({ note });
        await reload(origin);
        return true;
      } catch (thrown) {
        fail(thrown, req);
        return false;
      }
    };
    /** Reads the view (with the context session's live state). */
    const load = () => reload(origin);
    return {
      /** Opens the page and reads the catalog. */
      async open(): Promise<void> {
        patch({ open: true });
        await load();
      },
      close(): void {
        patch({ open: false, addOpen: false, addPrefill: null });
      },
      load,
      /** Asks Mistral for the catalog again. */
      async refresh(): Promise<void> {
        if (!canUseKey()) return;
        const sessionId = contextSession(core, shell);
        await run({ op: "refresh", ...(sessionId === null ? {} : { sessionId }) }, null);
      },
      setFilter(filter: Filter): void {
        patch({ filter });
      },
      setSearch(search: string): void {
        patch({ search });
      },
      select(id: string | null): void {
        patch({ selected: id });
      },
      /** Opens (with optional starting values) or closes the add dialog. */
      showAdd(open: boolean, prefill: AddPrefill | null = null): void {
        patch({ addOpen: open, addPrefill: open ? prefill : null, note: null });
      },
      /** Starts a Mistral connector's sign-in through the context session. */
      signIn: (alias: string) =>
        canUseKey()
          ? signIns.connector(origin, alias, contextSession(core, shell))
          : Promise.resolve(),
      /** Shows the sign-in page's link dialog again. */
      openPage: () => {
        signIns.openPage();
      },
      checkNow: () =>
        canUseKey() ? signIns.checkNow(origin, contextSession(core, shell)) : Promise.resolve(),
      stopWaiting: () => signIns.stop(origin),
      /** Turns a connector off (or on) for every session. */
      turnOff: (alias: string, off: boolean, name: string) =>
        canUseKey()
          ? run({ op: "setConnectorOff", alias, off }, `${name} is turned ${off ? "off" : "on"}.`)
          : Promise.resolve(false),
      /** Adds a server and starts its sign-in (one flow). Returns the saved name, or null. */
      async addServer(url: string, name: string | null): Promise<string | null> {
        try {
          const result = await request(origin, {
            op: "addServer",
            url,
            ...(name === null ? {} : { name }),
          });
          show(result);
          await reload(origin);
          if (result.kind !== "serverAdded") return null;
          patch({
            addOpen: false,
            addPrefill: null,
            selected: `s:${result.name}`,
            note: result.created ? null : `${result.name} was already added.`,
          });
          await signIns.server(origin, result.name);
          return result.name;
        } catch (thrown) {
          fail(thrown);
          return null;
        }
      },
      /**
       * adds a server that needs an API key. The core shows the native dialog (allow until Vibe Work
       * closes / always / don't); the key stays in the vault. Returns the saved name, or null.
       */
      async addKeyServer(url: string, name: string | null, key: KeyChoice): Promise<string | null> {
        if (!canUseVault()) return null;
        try {
          const result = await request(origin, {
            op: "addKeyServer",
            url,
            ...(name === null ? {} : { name }),
            ...(key.secretId === null ? {} : { secretId: key.secretId }),
            ...(key.newSecret === null ? {} : { newSecret: key.newSecret }),
            ...(key.header === null ? {} : { header: key.header }),
          });
          show(result);
          await reload(origin);
          if (result.kind !== "serverAdded") return null;
          patch({ addOpen: false, addPrefill: null, selected: `s:${result.name}`, note: null });
          return result.name;
        } catch (thrown) {
          fail(thrown);
          return null;
        }
      },
      /** allows a key server's key again (the core shows the native dialog). */
      allowServerKey: (serverName: string) =>
        canUseVault()
          ? run({ op: "allowServerKey", name: serverName }, `${serverName} may use its key.`)
          : Promise.resolve(false),
      signInServer: (serverName: string) => signIns.server(origin, serverName),
      signOutServer: (serverName: string) =>
        run({ op: "signOutServer", name: serverName }, `Signed out of ${serverName}.`),
      async removeServer(serverName: string): Promise<boolean> {
        const ok = await run({ op: "removeServer", name: serverName }, `${serverName} removed.`);
        if (ok && store.get().selected === `s:${serverName}`) patch({ selected: null });
        return ok;
      },
      setServerOff: (serverName: string, off: boolean) =>
        run(
          { op: "setServerOff", name: serverName, off },
          `${serverName} is turned ${off ? "off" : "on"}.`,
        ),
      /** Opens a sign-in Vibe asked for in a session (link dialog). */
      openPrompt: (alias: string) => {
        signIns.openPrompt(alias);
      },
    };
  };

  // Picking a session in the list shows that session instead of the page.
  let selected = shell.store.get().selected;
  shell.store.subscribe(() => {
    const now = shell.store.get().selected;
    if (now !== selected && store.get().open)
      patch({ open: false, addOpen: false, addPrefill: null });
    selected = now;
  });

  /** A recovered key can load a catalog that was held back while the vault was unavailable. */
  const onKeyReadinessChanged = (): Promise<void> =>
    store.get().open ? reload("ui") : Promise.resolve();

  return {
    store,
    actions: actions("ui"),
    debugApi: actions("debug-api"),
    onKeyReadinessChanged,
  } as const;
}

/** The connectors controller. */
export type Connectors = ReturnType<typeof createConnectors>;
