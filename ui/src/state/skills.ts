/** Tools skills state and actions. Depends on the typed core, shell, stores and the install-offer flow
 * (`installOffers.ts`: Install… / Update to vN… show an offer first and its button installs exactly that offer,
 * ); views render only. Mutations keep their original folder, survive tab changes and roll optimistic switches
 * back on failure. Called by: createApp, the Skills tab, the `ui/skills` route.
 */
import type { SkillDetail, SkillsAction, SkillsRequest, SkillsView } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import {
  type ConfirmOutcome,
  createOfferFlow,
  isStaleOffer,
  type OfferMode,
} from "./installOffers";
import type { Shell } from "./shellStore";
import { summarize } from "./sessionSummary";

export const SKILLS_BUSY = "Wait until the chats in this folder finish, then change skills.";
/**
 * The core's `serverCode` for an offered update that ran while Vibe now reports another version (or none) installed
 * (`crates/vw-core-api/src/skills.rs`): the row shows the core's sentence and the list is read again.
 */
export const UPDATED_OTHER_VERSION = "updatedOtherVersion";
/** The changes a row's controls make (everything but the reads and the offers). */
type Mutation = Exclude<
  SkillsAction,
  { op: "read" } | { op: "detail" } | { op: "offerImport" } | { op: "offerUpdate" }
>;
export interface SkillsState {
  view: SkillsView | null;
  loading: boolean;
  error: string | null;
  search: string;
  expanded: readonly string[];
  details: Readonly<Record<string, SkillDetail | undefined>>;
  detailLoading: readonly string[];
  errors: Readonly<Record<string, string | undefined>>;
  pending: readonly string[];
  removing: string | null;
  note: string | null;
  focus: { name: string; installed: boolean; serial: number } | null;
}
export const SKILLS_INITIAL: SkillsState = {
  view: null,
  loading: false,
  error: null,
  search: "",
  expanded: [],
  details: {},
  detailLoading: [],
  errors: {},
  pending: [],
  removing: null,
  note: null,
  focus: null,
};

/** Creates the skills controller; each action facade preserves its transport origin. */
export function createSkills({ core, shell }: { core: CoreController; shell: Shell }) {
  const store = createStore<SkillsState>(SKILLS_INITIAL);
  const patch = (p: Partial<SkillsState>) => {
    store.set((s) => ({ ...s, ...p }));
  };
  const context = () => {
    const { selected, newFolder } = shell.store.get();
    return selected !== null
      ? { sessionId: selected }
      : newFolder === null
        ? {}
        : { cwd: newFolder };
  };
  const key = () => JSON.stringify(context());
  let generation = 0;
  const request = async (origin: "ui" | "debug-api", action: SkillsAction, scope = context()) => {
    const result = await core.run(
      { type: "skills", request: { ...scope, ...action } as SkillsRequest },
      // The window facade's calls are not part of a served Debug API action (both are the window's commands).
      origin === "ui" ? { origin: "window" } : undefined,
    );
    if (result.type !== "skills") throw new Error("Unexpected answer to a skills request");
    return result.result;
  };
  const load = async (origin: "ui" | "debug-api") => {
    const current = ++generation;
    patch({ loading: true, error: null });
    try {
      const result = await request(origin, { op: "read" });
      if (current === generation && result.kind === "view") patch({ view: result.view });
    } catch (e) {
      if (current === generation) patch({ error: toCallError(e).error.message });
    } finally {
      if (current === generation) patch({ loading: false });
    }
  };
  /**
   * Runs one change: null when it went through, `notSent` when nothing was sent (another change runs, the folder
   * is busy, the row is locked: the controls are disabled then), else the core's error (shown on the row, except
   * `staleOffer` / `offerExpired`, which the offer block answers with a fresh offer).
   */
  const mutate = async (
    origin: "ui" | "debug-api",
    action: Mutation,
    name: string,
  ): Promise<ConfirmOutcome> => {
    const s = store.get();
    if (s.pending.length > 0) return "notSent";
    const row = s.view?.skills.find((r) => r.name === name);
    if (s.view?.busy || row?.locked) return "notSent";
    const scope = context();
    const scopeKey = key();
    ++generation;
    patch({
      pending: [...s.pending, name],
      errors: { ...s.errors, [name]: undefined },
      note: null,
    });
    if (action.op === "setEnabled" && s.view !== null)
      patch({
        view: {
          ...s.view,
          skills: s.view.skills.map((r) =>
            r.name === name ? { ...r, enabled: action.enabled } : r,
          ),
        },
      });
    try {
      const result = await request(origin, action, scope);
      if (
        action.op === "import" &&
        result.kind === "view" &&
        !result.view.skills.some((skill) => skill.name === name && skill.installed)
      )
        throw new Error(
          "Vibe accepted the install, but this skill is not available in this session.",
        );
      if (result.kind === "view" && scopeKey === key()) {
        const next = result.view.skills.find((r) => r.name === name);
        patch({
          view: result.view,
          removing: null,
          details: {},
          note:
            action.op === "import"
              ? `Installed ${name}.`
              : action.op === "remove"
                ? `Removed ${name}.`
                : action.op === "setLatest"
                  ? `Updated ${name} to v${String(next?.installedVersion ?? row?.latestVersion)}.`
                  : null,
          ...(action.op === "import" || action.op === "remove"
            ? {
                search: "",
                focus: {
                  name,
                  installed: action.op === "import",
                  serial: (store.get().focus?.serial ?? 0) + 1,
                },
              }
            : {}),
        });
      }
      return null;
    } catch (e) {
      if (scopeKey === key()) {
        const current = store.get().view;
        patch({
          ...(action.op === "remove" ? { removing: null } : {}),
          ...(action.op === "setEnabled" && current !== null && row !== undefined
            ? {
                view: {
                  ...current,
                  skills: current.skills.map((r) =>
                    r.name === name ? { ...r, enabled: row.enabled } : r,
                  ),
                },
              }
            : {}),
          // Cancel in the native dialog is the user's own answer: nothing changed, no error. A stale offer
          // is told by the offer block, which shows the fresh offer instead.
          errors: {
            ...store.get().errors,
            [name]:
              toCallError(e).error.code === "confirmationDeclined" ||
              isStaleOffer(toCallError(e).error)
                ? undefined
                : toCallError(e).error.message,
          },
        });
      }
      // The change ran but left another version: the list shows what is installed now.
      if (toCallError(e).error.serverCode === UPDATED_OTHER_VERSION && scopeKey === key())
        await load(origin);
      return toCallError(e).error;
    } finally {
      patch({ pending: store.get().pending.filter((n) => n !== name) });
    }
  };
  // Install… / Update to vN…: the offer the row shows before its button installs.
  const offers = createOfferFlow<"ui" | "debug-api">({
    async read(origin, name, mode: OfferMode) {
      const row = store.get().view?.skills.find((r) => r.name === name);
      const result = await request(
        origin,
        mode === "install"
          ? { op: "offerImport", skillId: row?.skillId ?? name }
          : { op: "offerUpdate", name },
      );
      if (result.kind !== "offer") throw new Error("Unexpected answer to a skills offer");
      return result.offer;
    },
    confirm(origin, name, mode, offerId) {
      const row = store.get().view?.skills.find((r) => r.name === name);
      return mutate(
        origin,
        mode === "install"
          ? { op: "import", skillId: row?.skillId ?? name, offerId }
          : { op: "setLatest", name, offerId },
        name,
      );
    },
  });
  /** Opens or closes a row's detail; opening reads the skill's full detail. */
  const disclose = async (origin: "ui" | "debug-api", name: string): Promise<void> => {
    const s = store.get();
    if (s.expanded.includes(name)) {
      patch({ expanded: s.expanded.filter((n) => n !== name) });
      // Collapsing the row also closes its offer (focus stays on the disclosure).
      offers.actions(origin).cancel(name, false);
      return;
    }
    patch({ expanded: [...s.expanded, name], detailLoading: [...s.detailLoading, name] });
    const scope = key();
    try {
      const result = await request(origin, { op: "detail", name });
      if (scope === key() && result.kind === "detail")
        patch({ details: { ...store.get().details, [name]: result.detail } });
    } catch (e) {
      if (scope === key())
        patch({ errors: { ...store.get().errors, [name]: toCallError(e).error.message } });
    } finally {
      patch({ detailLoading: store.get().detailLoading.filter((n) => n !== name) });
    }
  };
  const actions = (origin: "ui" | "debug-api") => ({
    load: () => load(origin),
    setSearch: (search: string) => {
      patch({ search });
    },
    showRemove: (removing: string | null) => {
      patch({ removing });
    },
    disclose: (name: string): Promise<void> => disclose(origin, name),
    async change(action: Mutation, name: string): Promise<boolean> {
      return (await mutate(origin, action, name)) === null;
    },
    /** Install… on a catalog row: opens the row's detail and shows what the click would install. */
    async offerInstall(name: string): Promise<void> {
      if (!store.get().expanded.includes(name)) void disclose(origin, name);
      await offers.actions(origin).open(name, "install");
    },
    /** Update to vN… on an installed row: opens its detail and shows the update offer ("4 → 5"). */
    async offerUpdate(name: string): Promise<void> {
      const row = store.get().view?.skills.find((r) => r.name === name);
      if (!store.get().expanded.includes(name)) void disclose(origin, name);
      await offers.actions(origin).open(name, "update", row?.installedVersion ?? null);
    },
    /** The offer's Install vN / Update to vN: installs exactly the shown offer. */
    confirmOffer: (name: string): Promise<boolean> => offers.actions(origin).confirm(name),
    /** The offer's Cancel (and Escape). */
    cancelOffer: (name: string): void => {
      offers.actions(origin).cancel(name);
    },
  });
  return {
    store,
    /** The open install offers (the offer block and the `ui/skills` route read them). */
    offers: offers.store,
    actions: actions("ui"),
    debugApi: actions("debug-api"),
    /** Observe folder selection and local chat activity while the tab is mounted. */
    watch(): () => void {
      let previous = key();
      const activity = () =>
        Object.values(core.store.get().sessions)
          .map((entry) => summarize(entry, core.store.get().stuck))
          .filter((row) => row.badge === "run" || row.badge === "input")
          .map((row) => `${row.id}:${row.cwd ?? ""}:${row.badge}`)
          .sort()
          .join("\n");
      let previousActivity = activity();
      const update = () => {
        const current = key();
        if (current !== previous) {
          previous = current;
          patch({ view: null, expanded: [], details: {}, errors: {}, removing: null, note: null });
          // An offer is bound to its folder: one from the previous folder would be refused.
          offers.actions("ui").clear();
          void load("ui");
          return;
        }
        const nextActivity = activity();
        if (nextActivity === previousActivity) return;
        previousActivity = nextActivity;
        const view = store.get().view;
        const { selected, newFolder } = shell.store.get();
        const model = core.store.get();
        const selectedEntry = selected === null ? undefined : model.sessions[selected];
        const folder =
          selectedEntry === undefined ? newFolder : summarize(selectedEntry, model.stuck).cwd;
        if (view !== null && folder === null) {
          void load("ui");
          return;
        }
        if (view !== null && folder !== null) {
          const busy = Object.values(model.sessions).some((s) => {
            const row = summarize(s, model.stuck);
            return row.cwd === folder && (row.badge === "run" || row.badge === "input");
          });
          if (busy !== view.busy) patch({ view: { ...view, busy } });
        }
      };
      const stops = [shell.store.subscribe(update), core.store.subscribe(update)];
      void load("ui");
      return () => {
        stops.forEach((stop) => {
          stop();
        });
      };
    },
  };
}
export type Skills = ReturnType<typeof createSkills>;
