/**
 * One-click install offers: the window asks the core for an
 * offer (what the click would install: name, version, publisher, source, licence, what it runs), shows it next to
 * the one button that confirms it, and the click sends that offer's id back. The core refuses a used, foreign or
 * changed offer with `staleOffer` and an expired one with `offerExpired`, and installs nothing; the window then
 * reads a fresh offer and shows it plainly, like any offer, with one sentence above the actions (updated, or ran
 * out: told by the code alone, never by the refusal's words). It never confirms by itself.
 *
 * This file holds the offer state and its logic for both Tools tabs that install (Skills, Marketplace): the open
 * offers per target, read failures, where focus goes next and every text the offer block shows. The skills and marketplace controllers each create one flow with their own read and confirm calls.
 * Depends on: createStore, the typed core types. Called by: `state/skills.ts`, `state/market.ts`, the offer block
 * (`features/connectors/InstallOffer.tsx`) and the `ui/skills` / `ui/marketplace` routes.
 * Invariants: a confirm sends exactly the shown offer's id, once per click; a stale answer only reads and shows a
 * new offer; a read that finishes after Cancel (or after a newer read) is dropped.
 */
import type { CoreError, ToolsInstallOffer } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import { createStore } from "./createStore";

/** Install something new, or update what is installed. */
export type OfferMode = "install" | "update";
/** `reading`: the core is preparing the offer; `shown`: the facts and the confirm button; `installing`: clicked. */
type OfferPhase = "reading" | "shown" | "installing";
/** Why an offer shows again after a click: the core refused it as stale (`changed`) or as expired (`ranOut`). */
export type OfferNotice = "changed" | "ranOut";
/**
 * The core's `serverCode`s for a refused offer (`crates/vw-core-api/src/install_offers.rs`, `STALE_OFFER` and
 * `OFFER_EXPIRED`).
 */
export const STALE_OFFER = "staleOffer";
export const OFFER_EXPIRED = "offerExpired";
/** The fact lines of the offer block, in display order. */
export type FactKey = "name" | "version" | "publisher" | "from" | "licence" | "runs";

/** One open offer (a skill row's or a marketplace entry's). */
export interface OpenOffer {
  /** The skill name or the marketplace entry id. */
  target: string;
  mode: OfferMode;
  phase: OfferPhase;
  /** The offer shown; null only during the first read. */
  offer: ToolsInstallOffer | null;
  /** The installed version an update starts from ("4 → 5"); null for an install. */
  fromVersion: number | null;
  /** Set when the offer shows again after a `staleOffer` / `offerExpired` refusal. */
  notice: OfferNotice | null;
  /** Increments on every read, so an older read's answer is dropped. */
  token: number;
}

/** Where the next render moves focus (`serial` makes a repeated request distinct). */
export interface OfferFocus {
  target: string;
  /** `confirm`: Install vN / Update to vN; `notice`: the stale notice; `opener`: Install… / Update to vN…. */
  part: "confirm" | "notice" | "opener";
  serial: number;
}

/** Everything the offer blocks show. */
export interface OffersState {
  open: Readonly<Record<string, OpenOffer | undefined>>;
  /** Read failures per target ("Could not read what this installs: …"), shown where the offer was. */
  problems: Readonly<Record<string, string | undefined>>;
  focus: OfferFocus | null;
}

export const OFFERS_INITIAL: OffersState = { open: {}, problems: {}, focus: null };

/** One fact line as the offer block renders it. */
export interface OfferFact {
  key: FactKey;
  label: string;
  value: string;
}

const NOT_STATED = "Not stated";
const LABELS: Readonly<Record<FactKey, string>> = {
  name: "Name",
  version: "Version",
  publisher: "Publisher",
  from: "From",
  licence: "Licence",
  runs: "What it runs",
};
// The window names the catalog in words; the core's `source` names the registry with an id
// ("Mistral skill registry (data-id)"), which the offer block does not show. Unknown kinds show `source`.
const FROM: Readonly<Record<string, string>> = {
  skill: "Mistral's skill catalog",
  skillUpdate: "Mistral's skill catalog",
  marketplace: "Vibe Work's reviewed Tools catalog",
};

/** The heading above the facts. */
export function offerHeading(mode: OfferMode): string {
  return mode === "update" ? "What you update" : "What you install";
}

/** The loading line while the core prepares the offer. */
export const OFFER_READING = "Reading what you install…";

/** The confirm button's words: Install v4 / Update to v5 / Install (no version) / Installing…. */
export function confirmLabel(open: OpenOffer): string {
  if (open.phase === "installing") return open.mode === "update" ? "Updating…" : "Installing…";
  const version = open.offer?.version ?? null;
  if (open.mode === "update") return version === null ? "Update" : `Update to v${String(version)}`;
  return version === null ? "Install" : `Install v${String(version)}`;
}

/**
 * The words above the actions of an offer that shows again. The window has already read the fresh offer, so the
 * words say so and name the one step left (no refresh control exists): check the facts above, then press the
 * confirm button again. The verb follows the button (install / update).
 */
export function noticeText(notice: OfferNotice, mode: OfferMode): string {
  const what = notice === "changed" ? "This offer changed" : "This offer expired";
  const again = mode === "update" ? "update again" : "install again";
  return `${what} and was refreshed. Check the details above, then ${again}.`;
}

/** The read-failure sentence, with the core's reason (its last full stop dropped). */
function problemText(reason: string): string {
  return `Could not read what this installs: ${reason.trim().replace(/\.$/, "")}. Nothing was installed. Try again.`;
}

function value(offer: ToolsInstallOffer, key: FactKey, fromVersion: number | null): string {
  switch (key) {
    case "name":
      return offer.name;
    case "version":
      if (offer.version === null) return NOT_STATED;
      return fromVersion === null || fromVersion === offer.version
        ? String(offer.version)
        : `${String(fromVersion)} → ${String(offer.version)}`;
    case "publisher":
      return offer.publisher ?? NOT_STATED;
    case "from":
      return FROM[offer.kind] ?? offer.source;
    case "licence":
      return offer.licence ?? NOT_STATED;
    case "runs":
      return offer.runs;
  }
}

const KEYS: readonly FactKey[] = ["name", "version", "publisher", "from", "licence", "runs"];

/** The fact lines of an open offer (empty while the first read runs). */
export function offerFacts(open: OpenOffer): OfferFact[] {
  const offer = open.offer;
  if (offer === null) return [];
  return KEYS.map((key) => ({
    key,
    label: LABELS[key],
    value: value(offer, key, open.fromVersion),
  }));
}

/**
 * Why a refused offer shows again, from the core's code alone: `offerExpired` ran out; anything else stale (used,
 * unknown, foreign or changed) is told as updated.
 */
export function staleNotice(error: CoreError): OfferNotice {
  return error.serverCode === OFFER_EXPIRED ? "ranOut" : "changed";
}

/** Whether a core refusal is a stale-offer answer (the window then shows a fresh offer). */
export function isStaleOffer(error: CoreError): boolean {
  return error.serverCode === STALE_OFFER || error.serverCode === OFFER_EXPIRED;
}

/** What a confirm call returned: null when it went through, `notSent` when the tab sent nothing, else the error. */
export type ConfirmOutcome = CoreError | null | "notSent";

/** The calls a tab gives the flow. Each receives the origin of the action that started it. */
export interface OfferCalls<O extends string> {
  /** Reads a new offer for `target`. Throws the core's error. */
  read(origin: O, target: string, mode: OfferMode): Promise<ToolsInstallOffer>;
  /** Installs or updates with the shown offer's id; the tab shows its own success and other failures. */
  confirm(origin: O, target: string, mode: OfferMode, offerId: string): Promise<ConfirmOutcome>;
}

/** A JSON view of the open offers and read failures for a Debug API `state` read. */
export function offersReport(state: OffersState): unknown {
  return {
    open: Object.values(state.open)
      .filter((o): o is OpenOffer => o !== undefined)
      .map((o) => ({
        target: o.target,
        mode: o.mode,
        phase: o.phase,
        offerId: o.offer?.offerId ?? null,
        expiresAt: o.offer?.expiresAt ?? null,
        heading: offerHeading(o.mode),
        facts: offerFacts(o),
        confirm: o.phase === "reading" ? null : confirmLabel(o),
        notice: o.notice === null ? null : noticeText(o.notice, o.mode),
        noticeKind: o.notice,
      })),
    problems: state.problems,
    focus: state.focus,
  };
}

/**
 * Creates one tab's offer flow.
 * @param calls the tab's read and confirm calls.
 * @returns the store and an action set per origin (`actions("ui")`, `actions("debug-api")`).
 */
export function createOfferFlow<O extends string>(calls: OfferCalls<O>) {
  const store = createStore<OffersState>(OFFERS_INITIAL);
  let tokens = 0;
  let serial = 0;
  const focus = (target: string, part: OfferFocus["part"]): OfferFocus => ({
    target,
    part,
    serial: ++serial,
  });
  const setOpen = (
    target: string,
    open: OpenOffer | undefined,
    extra: Partial<OffersState> = {},
  ) => {
    store.set((s) => ({ ...s, open: { ...s.open, [target]: open }, ...extra }));
  };
  const current = (target: string, token: number) => store.get().open[target]?.token === token;
  const problem = (target: string, thrown: unknown): void => {
    setOpen(target, undefined, {
      problems: {
        ...store.get().problems,
        [target]: problemText(toCallError(thrown).error.message),
      },
      focus: focus(target, "opener"),
    });
  };

  const actions = (origin: O) => ({
    /**
     * Opens the offer for `target` (Install… / Update to vN…): shows the loading line, reads the offer, then shows
     * it and moves focus to the confirm button. A read failure closes it with the read-failed sentence.
     */
    async open(target: string, mode: OfferMode, fromVersion: number | null = null): Promise<void> {
      const token = ++tokens;
      setOpen(
        target,
        {
          target,
          mode,
          phase: "reading",
          offer: null,
          fromVersion,
          notice: null,
          token,
        },
        { problems: { ...store.get().problems, [target]: undefined } },
      );
      try {
        const offer = await calls.read(origin, target, mode);
        if (!current(target, token)) return;
        const open = store.get().open[target];
        if (open !== undefined)
          setOpen(target, { ...open, phase: "shown", offer }, { focus: focus(target, "confirm") });
      } catch (thrown) {
        if (current(target, token)) problem(target, thrown);
      }
    },
    /**
     * Cancel / Escape: closes the offer and moves focus back to Install…; `refocus: false` when the user closed it
     * another way (collapsing the row keeps focus on its disclosure).
     */
    cancel(target: string, refocus = true): void {
      if (store.get().open[target] === undefined) return;
      setOpen(target, undefined, refocus ? { focus: focus(target, "opener") } : {});
    },
    /** Closes every offer (the folder or the selection changed); nothing moves focus. */
    clear(): void {
      store.set((s) => ({ ...s, open: {}, problems: {} }));
    },
    /**
     * The confirm button: sends the shown offer's id once. On `staleOffer` / `offerExpired` it reads a new offer and
     * shows it with the notice (focus to the notice); it never sends again by itself. Returns whether the change went through.
     */
    async confirm(target: string): Promise<boolean> {
      const open = store.get().open[target];
      if (open?.phase !== "shown" || open.offer === null) return false;
      const token = open.token;
      setOpen(target, { ...open, phase: "installing" });
      const error = await calls.confirm(origin, target, open.mode, open.offer.offerId);
      if (!current(target, token)) return error === null;
      if (error === "notSent") {
        // Nothing reached the core, so the offer is still valid: show it again as it was.
        setOpen(target, { ...open, phase: "shown" });
        return false;
      }
      if (error === null) {
        setOpen(target, undefined);
        return true;
      }
      if (!isStaleOffer(error)) {
        // The offer was taken (single use) whatever the result; the tab shows the failure in its own line.
        setOpen(target, undefined);
        return false;
      }
      const next = ++tokens;
      setOpen(target, { ...open, phase: "reading", token: next });
      try {
        const offer = await calls.read(origin, target, open.mode);
        if (!current(target, next)) return false;
        setOpen(
          target,
          { ...open, phase: "shown", offer, notice: staleNotice(error), token: next },
          { focus: focus(target, "notice") },
        );
      } catch (thrown) {
        if (current(target, next)) problem(target, thrown);
      }
      return false;
    },
  });
  return { store, actions };
}
