/**
 * The Allowed without asking page's state and actions, one per window (kept for the app run, so a
 * group's open/closed state survives leaving the page). Holds the two lists the core gives on request (earlier rules
 * via `vibeRules list`, the vault's grants via `secrets list`) and the person's choices; the session answers are read
 * live from the core model (`askGrants`). Every click and every `ui/settings` route action runs the same function
 * here, so the route can never do what the page cannot.
 * Depends on: AppContext types, state/createStore, state/secrets, state/keyReadiness, state/sessionSummary,
 * entries, view, dialog, wording. Called by: the Allowed page, the Safety summary, allowed/open, allowed/routes.
 * Invariants: a revoke goes out as one command in the click (a later callback never carries the click's origin); a
 * bulk revoke is ONE call (`askGrants removeMany`, `vibeRules removeMany`, `secrets revokeMany`) with exactly the
 * keys the confirmation listed, and its status names how many the core revoked (not how many were listed); a refused command (e.g. the vault's `vaultUserOnly` refusal of a Debug API request)
 * is shown in its group and returned to the caller, never retried another way; nothing is removed from the page
 * before the core says it is gone.
 */
import type { ErrorCode, SessionGrant } from "../../../protocol/core";
import { toCallError } from "../../../protocol/client/types";
import { createStore, type Store } from "../../../state/createStore";
import { vaultReadiness } from "../../../state/keyReadiness";
import { problem, secrets } from "../../../state/secrets";
import { type SessionEntry, summarize } from "../../../state/sessionSummary";
import type { App } from "../../shell/AppContext";
import { type RevokeAllDialog, revokeAllDialog } from "./dialog";
import {
  type AllowedEntry,
  type AllowedSources,
  allowedEntries,
  type GroupId,
  type RulesSource,
  type SessionInfo,
  type VaultSource,
} from "./entries";
import {
  type AllowedUi,
  INITIAL_UI,
  isFiltering,
  type LeafView,
  MORE_ROWS,
  FIRST_ROWS,
  type PageView,
  pageView,
} from "./view";
import { REVOKED_ONE, revokedMany } from "./wording";

/** Where focus goes next: the first of these elements that is on the page (`data-allowed-focus`). */
interface FocusRequest {
  targets: readonly string[];
  /** Bumped on every request, so the same target can be asked for twice. */
  serial: number;
}

/** The page's state. */
export interface AllowedState {
  rules: RulesSource;
  vault: VaultSource;
  ui: AllowedUi;
  /** Entry keys whose revoke is running ("Revoking…"). */
  busy: Readonly<Record<string, true>>;
  /** The last revoke's status per group (polite status line). */
  status: Readonly<Partial<Record<GroupId, string>>>;
  /** The last refused revoke per group (`key-problem`). */
  problem: Readonly<Partial<Record<GroupId, string>>>;
  dialog: RevokeAllDialog | null;
  dialogBusy: boolean;
  focus: FocusRequest | null;
}

/** The result of an action a caller (a click or a route) may need to report. */
export type Outcome =
  { ok: true } | { ok: false; code: ErrorCode; serverCode: string | null; message: string };

const READING_RULES: RulesSource = {
  state: "reading",
  rules: [],
  defaultsKnown: true,
  error: null,
};
const READING_VAULT: VaultSource = {
  state: "reading",
  fills: [],
  servers: [],
  error: null,
};

/** The first state. */
const INITIAL_ALLOWED: AllowedState = {
  rules: READING_RULES,
  vault: READING_VAULT,
  ui: INITIAL_UI,
  busy: {},
  status: {},
  problem: {},
  dialog: null,
  dialogBusy: false,
  focus: null,
};

/** The sessions as the page needs them (title, folder, parent), from the core model's entries. */
export function sessionInfos(
  sessions: Readonly<Record<string, SessionEntry>>,
): Record<string, SessionInfo> {
  const out: Record<string, SessionInfo> = {};
  for (const [id, entry] of Object.entries(sessions)) {
    const s = summarize(entry);
    out[id] = { title: s.title, cwd: s.cwd, parentId: s.parentId };
  }
  return out;
}

/** The sources from the parts the page subscribes to. */
export function sourcesOf(
  grants: readonly SessionGrant[],
  sessions: Readonly<Record<string, SessionInfo>>,
  state: AllowedState,
): AllowedSources {
  return { grants, sessions, rules: state.rules, vault: state.vault };
}

/** The groups the page keeps on screen although they are now empty (their status or problem is shown). */
export function keptGroups(state: AllowedState): GroupId[] {
  return (["sessions", "earlier", "vault"] as const).filter(
    (g) => state.status[g] !== undefined || state.problem[g] !== undefined,
  );
}

/** `record` without `key`. */
function without<K extends string, V>(
  record: Readonly<Partial<Record<K, V>>>,
  key: K,
): Partial<Record<K, V>> {
  return Object.fromEntries(Object.entries(record).filter(([k]) => k !== key)) as Partial<
    Record<K, V>
  >;
}

function fail(thrown: unknown): Outcome {
  const error = toCallError(thrown).error;
  return { ok: false, code: error.code, serverCode: error.serverCode, message: problem(thrown) };
}

/** Creates the store of one window. */
function createAllowed(app: App) {
  const state: Store<AllowedState> = createStore(INITIAL_ALLOWED);
  let rulesRead = 0;
  let vaultRead = 0;
  let watchers = 0;
  let unlisten: (() => void) | null = null;
  let focusSerial = 0;

  const patch = (next: Partial<AllowedState>) => {
    state.set((s) => ({ ...s, ...next }));
  };
  const setUi = (next: Partial<AllowedUi>) => {
    state.set((s) => ({ ...s, ui: { ...s.ui, ...next } }));
  };
  const focus = (...targets: string[]) => {
    patch({ focus: { targets, serial: ++focusSerial } });
  };
  const sources = (): AllowedSources => {
    const core = app.core?.store.get();
    return sourcesOf(core?.askGrants.grants ?? [], sessionInfos(core?.sessions ?? {}), state.get());
  };
  const entries = (): AllowedEntry[] => allowedEntries(sources(), Date.now());
  const view = (): PageView => {
    const s = state.get();
    return pageView(sources(), entries(), s.ui, keptGroups(s));
  };
  const leafOf = (key: string): LeafView | null => {
    for (const g of view().groups)
      for (const l of [...(g.leaf === null ? [] : [g.leaf]), ...g.subgroups])
        if (l.key === key) return l;
    return null;
  };
  const groupOfLeaf = (leaf: string): GroupId =>
    leaf === "earlier" ? "earlier" : leaf.startsWith("vault/") ? "vault" : "sessions";

  /** Reads the earlier rules (a newer read wins). */
  const readRules = async (): Promise<void> => {
    const core = app.core;
    if (core === null) return;
    const read = ++rulesRead;
    try {
      const r = await core.run({ type: "vibeRules", request: { op: "list" } });
      if (read !== rulesRead || r.type !== "vibeRules") return;
      patch({
        rules: { state: "ready", rules: r.rules, defaultsKnown: r.defaultsKnown, error: null },
      });
    } catch (thrown) {
      if (read === rulesRead)
        patch({ rules: { ...READING_RULES, state: "error", error: problem(thrown) } });
    }
  };

  /** Reads the vault's grants; a locked vault is not asked (its items are not shown or counted). */
  const readVault = async (): Promise<void> => {
    const core = app.core;
    if (core === null) return;
    const read = ++vaultRead;
    const blocked = app.keys === null ? null : vaultReadiness(app.keys.store.get());
    if (blocked !== null) {
      // No vault yet: nothing in it. Locked (or a vault to pick): not readable now. Unavailable: an error.
      patch({
        vault:
          blocked.kind === "setup"
            ? { ...READING_VAULT, state: "ready" }
            : blocked.kind === "locked"
              ? { ...READING_VAULT, state: "locked" }
              : { ...READING_VAULT, state: "error", error: blocked.message },
      });
      return;
    }
    try {
      const r = await secrets(core, { op: "list" });
      if (read !== vaultRead || r.kind !== "list") return;
      patch({
        vault: r.unlocked
          ? {
              state: "ready",
              fills: r.fillGrants,
              servers: r.serverGrants,
              error: null,
            }
          : { ...READING_VAULT, state: "locked" },
      });
    } catch (thrown) {
      if (read !== vaultRead) return;
      const code = toCallError(thrown).error.serverCode;
      patch({
        vault:
          code === "passwordRequired"
            ? { ...READING_VAULT, state: "locked" }
            : code === "notConfigured"
              ? // No vault connected in this run: nothing in it to allow, not an error to fix.
                { ...READING_VAULT, state: "ready" }
              : { ...READING_VAULT, state: "error", error: problem(thrown) },
      });
    }
  };

  /** Reads both lists. */
  const load = async (): Promise<void> => {
    await Promise.all([readRules(), readVault()]);
  };

  /** While a page that shows the lists is mounted: read them, and read the vault again when it changes. */
  const watch = (): (() => void) => {
    watchers++;
    if (watchers === 1 && app.core !== null) {
      void load();
      unlisten = app.core.listen((event) => {
        if (["secretsChanged", "keyStatusChanged", "lagged"].includes(event.type)) void readVault();
      });
    }
    return () => {
      watchers--;
      if (watchers === 0) {
        unlisten?.();
        unlisten = null;
      }
    };
  };

  /** Changes the Find text: the filter's own open/closed state and the paging start over. */
  const refilter = (next: Partial<AllowedUi>) => {
    setUi({ ...next, filterOpen: {}, shown: {} });
  };

  /** Opens or closes a group or sub-group. */
  const toggle = (key: string) => {
    const v = view();
    const all = v.groups.flatMap((g) => [
      { key: g.id, open: g.open },
      ...g.subgroups.map((s) => ({ key: s.key, open: s.open })),
    ]);
    const now = all.find((x) => x.key === key);
    if (now === undefined) return;
    const s = state.get().ui;
    if (v.filtering) setUi({ filterOpen: { ...s.filterOpen, [key]: !now.open } });
    else setUi({ open: { ...s.open, [key]: !now.open } });
  };

  /** "Show 20 more": the next rows of a leaf; focus to the first newly shown row's Revoke. */
  const showMore = (key: string) => {
    const l = leafOf(key);
    if (l === null || l.notShown === 0) return;
    const shown = l.rows.length;
    const ui = state.get().ui;
    setUi({ shown: { ...ui.shown, [key]: (ui.shown[key] ?? FIRST_ROWS) + MORE_ROWS } });
    const first = l.entries[shown];
    if (first !== undefined) focus(`revoke:${first.key}`);
  };

  /** The command that revokes one entry; resolves when the page's data shows it gone. */
  const revokeCommand = async (entry: AllowedEntry): Promise<void> => {
    const core = app.core;
    if (core === null) throw new Error("This run has no core.");
    if (entry.kind === "session" && "id" in entry.ref) {
      await core.run({ type: "askGrants", request: { op: "remove", id: entry.ref.id } });
    } else if (entry.kind === "rule" && "rule" in entry.ref) {
      const r = await core.run({ type: "vibeRules", request: { op: "remove", ...entry.ref.rule } });
      if (r.type === "vibeRules")
        patch({
          rules: { state: "ready", rules: r.rules, defaultsKnown: r.defaultsKnown, error: null },
        });
    } else if ("id" in entry.ref) {
      await secrets(core, { op: "revoke", grantId: entry.ref.id });
      await readVault();
    }
  };

  /** One row's Revoke (busy "Revoking…"; the kind's status line; focus to the next row, else the previous). */
  const revoke = async (key: string): Promise<Outcome> => {
    const entry = entries().find((e) => e.key === key);
    if (entry === undefined)
      return {
        ok: false,
        code: "notFound",
        serverCode: null,
        message: "That entry is not listed.",
      };
    if (state.get().busy[key] === true)
      return {
        ok: false,
        code: "denied",
        serverCode: null,
        message: "That entry is being revoked.",
      };
    const leaf = leafOf(entry.leaf);
    const rows = leaf?.rows ?? [];
    const at = rows.findIndex((r) => r.key === key);
    const neighbour = rows[at + 1] ?? rows[at - 1];
    const group = entry.group;
    state.set((s) => ({
      ...s,
      busy: { ...s.busy, [key]: true },
      status: without(s.status, group),
      problem: without(s.problem, group),
    }));
    // Sent at once, in the click: a later callback must never inherit the click's origin.
    const sent = revokeCommand(entry);
    try {
      await sent;
      const words =
        entry.kind === "session"
          ? REVOKED_ONE.session
          : entry.kind === "rule"
            ? REVOKED_ONE.rule
            : REVOKED_ONE.vault;
      state.set((s) => ({ ...s, status: { ...s.status, [group]: words } }));
      focus(
        ...(neighbour === undefined ? [] : [`revoke:${neighbour.key}`]),
        `header:${entry.leaf}`,
        `header:${group}`,
      );
      return { ok: true };
    } catch (thrown) {
      state.set((s) => ({ ...s, problem: { ...s.problem, [group]: problem(thrown) } }));
      return fail(thrown);
    } finally {
      state.set((s) => ({ ...s, busy: without(s.busy, key) as Record<string, true> }));
    }
  };

  /** "Revoke all {n}…": opens the confirmation for the leaf's matching entries. */
  const askRevokeAll = (key: string): Outcome => {
    const l = leafOf(key);
    if (l === null || !l.open || l.entries.length === 0)
      return {
        ok: false,
        code: "notFound",
        serverCode: null,
        message: "That list is not shown open.",
      };
    patch({ dialog: revokeAllDialog(l, isFiltering(state.get().ui)) });
    return { ok: true };
  };

  /** Cancel (or Escape, or the scrim): nothing is revoked; focus back to the bulk link. */
  const cancelRevokeAll = () => {
    const d = state.get().dialog;
    if (d === null || state.get().dialogBusy) return;
    patch({ dialog: null });
    focus(`bulk:${d.leaf}`, `header:${d.leaf}`);
  };

  /** How many of `list` the page no longer shows (the vault says no count: read again, then compare). */
  const goneOf = (list: readonly AllowedEntry[]): number => {
    const still = new Set(entries().map((e) => e.key));
    return list.filter((e) => !still.has(e.key)).length;
  };

  /**
   * The bulk command for exactly the listed entries: one call. Returns how many the core revoked (entries already
   * gone are skipped). A vault revoke that fails half way is read again before the error goes on, so the page shows
   * the vault as it is; `partial.revoked` then says how many went.
   */
  const bulkCommand = async (
    list: readonly AllowedEntry[],
    partial: { revoked: number },
  ): Promise<number> => {
    const core = app.core;
    if (core === null) throw new Error("This run has no core.");
    const ids = list.flatMap((e) => ("id" in e.ref ? [e.ref.id] : []));
    const kind = list[0]?.kind;
    if (kind === "session") {
      const r = await core.run({ type: "askGrants", request: { op: "removeMany", ids } });
      return r.type === "askGrants" ? r.removed : list.length;
    }
    if (kind === "rule") {
      const rules = list.flatMap((e) => ("rule" in e.ref ? [e.ref.rule] : []));
      const r = await core.run({ type: "vibeRules", request: { op: "removeMany", rules } });
      if (r.type !== "vibeRules") return list.length;
      patch({
        rules: { state: "ready", rules: r.rules, defaultsKnown: r.defaultsKnown, error: null },
      });
      return r.removed;
    }
    try {
      await secrets(core, { op: "revokeMany", grantIds: ids });
    } catch (thrown) {
      await readVault();
      partial.revoked = goneOf(list);
      throw thrown;
    }
    await readVault();
    return goneOf(list);
  };

  /** The confirm button ("Revoke 46 rules"): one bulk call; status "Revoked 46. …". */
  const confirmRevokeAll = async (): Promise<Outcome> => {
    const s = state.get();
    const d = s.dialog;
    if (d === null || s.dialogBusy)
      return { ok: false, code: "notFound", serverCode: null, message: "No confirmation is open." };
    const wanted = new Set(d.keys);
    const list = entries().filter((e) => wanted.has(e.key));
    const group = groupOfLeaf(d.leaf);
    if (list.length === 0) {
      patch({ dialog: null });
      return { ok: true };
    }
    patch({ dialogBusy: true });
    state.set((x) => ({
      ...x,
      status: without(x.status, group),
      problem: without(x.problem, group),
    }));
    const kind =
      list[0]?.kind === "session" ? "session" : list[0]?.kind === "rule" ? "rule" : "vault";
    const partial = { revoked: 0 };
    const sent = bulkCommand(list, partial);
    try {
      const removed = await sent;
      state.set((x) => ({
        ...x,
        dialog: null,
        status: { ...x.status, [group]: revokedMany(kind, removed) },
      }));
      focus(`bulk:${d.leaf}`, `header:${d.leaf}`, `header:${group}`);
      return { ok: true };
    } catch (thrown) {
      // A vault revoke can stop half way: say how many went, next to the error.
      const revoked = partial.revoked;
      state.set((x) => ({
        ...x,
        dialog: null,
        ...(revoked > 0 ? { status: { ...x.status, [group]: revokedMany(kind, revoked) } } : {}),
        problem: { ...x.problem, [group]: problem(thrown) },
      }));
      focus(`bulk:${d.leaf}`, `header:${d.leaf}`, `header:${group}`);
      return fail(thrown);
    } finally {
      patch({ dialogBusy: false });
    }
  };

  /** Shows a Settings category (the Settings page is open when this page is). */
  const showCategory = (category: string, setting?: string) => {
    if (app.shell.store.get().settings.open)
      app.shell.actions.setSettingsCategory(category, setting);
    else app.shell.actions.openSettings(category, setting);
  };

  return {
    state,
    view,
    entries,
    sources,
    load,
    readVault,
    watch,
    setQuery: (query: string) => {
      refilter({ query });
    },
    clearSearch: () => {
      refilter({ query: "" });
      focus("find");
    },
    toggle,
    showMore,
    revoke,
    askRevokeAll,
    cancelRevokeAll,
    confirmRevokeAll,
    showCategory,
    /** "Open session": leaves Settings for that session. */
    openSession: async (sessionId: string) => {
      app.shell.actions.closeSettings();
      app.projects?.close();
      await app.sessions?.open(sessionId);
    },
    /** "Open Project secrets" (the vault is locked). */
    openProjectSecrets: () => {
      showCategory("secrets");
    },
    /** Opens a group and one sub-group (the session grants list's link), clearing a Find that could hide it. */
    reveal: (group: GroupId, leaf: string | null) => {
      const ui = state.get().ui;
      setUi({
        query: "",
        filterOpen: {},
        shown: {},
        open: { ...ui.open, [group]: true, ...(leaf === null ? {} : { [leaf]: true }) },
      });
      focus(...(leaf === null ? [] : [`header:${leaf}`]), `header:${group}`);
    },
    /** The focus request was carried out. */
    focused: () => {
      patch({ focus: null });
    },
  };
}

/** One window's Allowed page store. */
export type Allowed = ReturnType<typeof createAllowed>;

const STORES = new WeakMap<App, Allowed>();

/** The store of `app` (made on first use, kept for the app run). */
export function allowedFor(app: App): Allowed {
  const existing = STORES.get(app);
  if (existing !== undefined) return existing;
  const made = createAllowed(app);
  STORES.set(app, made);
  return made;
}
