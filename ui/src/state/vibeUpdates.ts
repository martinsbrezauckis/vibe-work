/**
 * Vibe updates: turns the `update` part of the install view into what the update area shows. The core
 * checks the signed supported-Vibe list together with the app's update check and decides what may be offered; this
 * module only words it.
 * - "Vibe Y is available": a tested version newer than the running one (Vibe Work's own copy only), with Update.
 * - "Vibe Y is out; support arrives in an upcoming update": newer on PyPI, not tested yet; never installable.
 * - A version notice for a user's own install (nothing is installed for it).
 * - The update's own progress, failure and "starts next time".
 * Depends on: protocol types. Called by: VibeUpdateNotice, UpdateSettings, their tests.
 * Invariants: pure; nothing here can install (Update only asks the core for an offer, which the user confirms in
 * a dialog).
 */
import type { InstallView } from "../protocol/core";

/** A button of the Vibe notice. */
export type VibeNoticeAction = "update" | "later";

/** What the Vibe notice shows. */
export interface VibeNotice {
  tone: "info" | "ask" | "error";
  headline: string;
  detail: string | null;
  /** 0..1 while the update installs. */
  progress: number | null;
  actions: readonly VibeNoticeAction[];
  /** Identifies this state, for "Later". */
  key: string;
}

/** The running Vibe's version, when Vibe is set up. */
export function runningVibe(view: InstallView): string | null {
  return view.phase.state === "installed" ? view.phase.version : null;
}

/** The notice for `view`, or `null` when there is nothing to say. */
export function vibeNoticeFor(view: InstallView): VibeNotice | null {
  const update = view.update;
  const phase = update.phase;
  const current = runningVibe(view);
  switch (phase.state) {
    case "installing":
      return {
        tone: "info",
        headline: `Updating Vibe${update.available === null ? "" : ` to ${update.available}`}…`,
        detail: `Step ${String(phase.index)} of ${String(phase.total)}: ${phase.step}. Your chats keep running.`,
        progress: phase.total > 0 ? Math.min(1, (phase.index - 1) / phase.total) : null,
        actions: [],
        key: "vibe-installing",
      };
    case "failed":
      return {
        tone: "error",
        headline: "The Vibe update did not finish.",
        detail: `${phase.error} Your current Vibe is unchanged.`,
        progress: null,
        actions: update.available === null ? ["later"] : ["update", "later"],
        key: `vibe-failed:${phase.step}:${phase.error}`,
      };
    case "installed":
      return {
        tone: "info",
        headline: `Vibe ${phase.version} is installed.`,
        detail: `It starts the next time you open Vibe Work${current === null ? "" : `; until then your chats keep running on Vibe ${current}`}.`,
        progress: null,
        actions: ["later"],
        key: `vibe-installed:${phase.version}`,
      };
    default:
      break;
  }
  if (update.available !== null) {
    return {
      tone: "info",
      headline: `Vibe ${update.available} is available.`,
      detail:
        `Vibe Work has tested it${current === null ? "" : ` (you have ${current})`}. ` +
        "It installs next to your current Vibe and starts the next time you open Vibe Work." +
        (update.upcoming === null
          ? ""
          : ` Vibe ${update.upcoming} is also out; support for it arrives in an upcoming update.`),
      progress: null,
      actions: ["update", "later"],
      key: `vibe-available:${update.available}`,
    };
  }
  if (update.upcoming !== null) {
    return {
      tone: "info",
      headline: `Vibe ${update.upcoming} is out; support arrives in an upcoming update.`,
      detail: "Vibe Work offers a new Vibe once it has been tested with it.",
      progress: null,
      actions: ["later"],
      key: `vibe-upcoming:${update.upcoming}`,
    };
  }
  if (update.mismatch !== null) {
    return {
      tone: "info",
      headline: "About your Vibe install",
      detail: update.mismatch,
      progress: null,
      actions: ["later"],
      key: `vibe-mismatch:${update.mismatch}`,
    };
  }
  return null;
}

/** One line for Settings: the running Vibe and what the last check found. */
export function vibeStatusLine(view: InstallView): string {
  const current = runningVibe(view);
  const update = view.update;
  const own =
    view.phase.state === "installed" && view.phase.source === "existing"
      ? " (your own install)"
      : "";
  const base = current === null ? "Vibe is not set up yet." : `Vibe ${current}${own}.`;
  switch (update.phase.state) {
    case "installing":
      return `${base} Updating: ${update.phase.step}…`;
    case "installed":
      return `${base} Vibe ${update.phase.version} is installed and starts next time.`;
    case "failed":
      return `${base} The update did not finish: ${update.phase.error}`;
    default:
      break;
  }
  if (update.available !== null) return `${base} Vibe ${update.available} is available.`;
  if (update.upcoming !== null)
    return `${base} Vibe ${update.upcoming} is out; support arrives in an upcoming update.`;
  if (update.mismatch !== null) return `${base} ${update.mismatch}`;
  if (update.checkedAt === null) return `${base} Not checked yet in this run.`;
  if (update.checkError !== null) return `${base} Could not check: ${update.checkError}`;
  return `${base} No newer tested Vibe.`;
}
