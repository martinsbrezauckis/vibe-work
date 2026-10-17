/**
 * Access modes (Vibe's agent profiles) as the composer's access menu shows them:
 * labels and the colour level of the composer's top rule (`.composer.lvl-*`), which profiles are
 * offered, and which one the session runs in.
 * Depends on: protocol/core types. Called by: the access menu, the composer view, composerSession.
 * Invariants:
 * - Smart approve is offered only when Vibe lists `smart-approve` among the session's agents (a Mistral
 *   dark launch); Vibe Work never forces it.
 * - Full access (`auto-approve`) needs consent: a trusted native menu pick decides; other paths need
 *   native confirmation. The gate always requires a token for the switch.
 * - "Plan first" is Vibe's builtin `plan` profile and is never called read-only (shell
 *   commands still run). "Plan on a page" is the app-owned `vw-plan` profile: the plan is
 *   written as a Project Pages plan page; other file writes are refused; shell commands still run.
 */
import type { JsonValue } from "../../../protocol/core";
import { term, TEXT, wording } from "../../../vocabulary";
import { type Obj, obj } from "../../transcript/model/entries";

/** The rule colour of a mode (classes `lvl-yolo|smart|warn|safe`; neutral has none). */
export type AccessLevel = "yolo" | "smart" | "warn" | "neutral" | "safe";

/** One access mode. */
export interface AccessMode {
  id: string;
  label: string;
  description: string;
  tooltip: string;
  level: AccessLevel;
  /** Untrusted switches need a native confirmation first. */
  confirm: boolean;
}

/** Vibe's profiles in menu order with the wording that matches the access rules. */
export const ACCESS_MODES: readonly AccessMode[] = [
  {
    id: "auto-approve",
    label: term("accessModes", "auto-approve").name,
    description: term("accessModes", "auto-approve").description,
    tooltip: term("accessModes", "auto-approve").tooltip,
    level: "yolo",
    confirm: true,
  },
  {
    id: "smart-approve",
    label: term("accessModes", "smart-approve").name,
    description: term("accessModes", "smart-approve").description,
    tooltip: term("accessModes", "smart-approve").tooltip,
    level: "smart",
    confirm: false,
  },
  {
    id: "accept-edits",
    label: term("accessModes", "accept-edits").name,
    description: term("accessModes", "accept-edits").description,
    tooltip: term("accessModes", "accept-edits").tooltip,
    level: "warn",
    confirm: false,
  },
  {
    id: "ask",
    label: term("accessModes", "ask").name,
    description: term("accessModes", "ask").description,
    tooltip: term("accessModes", "ask").tooltip,
    level: "neutral",
    confirm: false,
  },
  {
    id: "plan",
    label: term("accessModes", "plan").name,
    description: term("accessModes", "plan").description,
    tooltip: term("accessModes", "plan").tooltip,
    level: "safe",
    confirm: false,
  },
  {
    id: "vw-plan",
    label: term("accessModes", "vw-plan").name,
    description: term("accessModes", "vw-plan").description,
    tooltip: term("accessModes", "vw-plan").tooltip,
    level: "safe",
    confirm: false,
  },
];

/** Names of the agents a `RuntimeSnapshot` lists (`runtime.agents[].name`), or `null` without one. */
export function runtimeAgents(runtime: JsonValue | undefined): string[] | null {
  const agents = obj(runtime)?.["agents"];
  if (!Array.isArray(agents)) return null;
  return agents.flatMap((a) => {
    const name = obj(a)?.["name"];
    return typeof name === "string" ? [name] : [];
  });
}

/**
 * The modes to offer: every mode except smart approve, which needs Vibe to list it, and "Plan on a page", which is
 * hidden when Vibe's list is known and lacks it (its profile file was not there when the session opened).
 */
export function offeredModes(runtime: JsonValue | undefined): AccessMode[] {
  const listed = runtimeAgents(runtime);
  return ACCESS_MODES.filter((mode) => {
    if (mode.id === "smart-approve") return listed?.includes(mode.id) === true;
    if (mode.id === "vw-plan") return listed === null || listed.includes(mode.id);
    return true;
  });
}

/**
 * The core's resolved mode is available before attachment and stays visible during verification.
 * Older clients fall back to the runtime, then Vibe's recorded agent (an object with a name).
 */
export function currentModeId(
  runtime: JsonValue | undefined,
  session: Obj | undefined,
  remembered?: string,
): string | null {
  if (remembered !== undefined && remembered !== "") return remembered;
  const active = obj(obj(runtime)?.["activeAgent"] ?? null)?.["name"];
  if (typeof active === "string" && active !== "") return active;
  const agent = session?.["agent"];
  const name = typeof agent === "string" ? agent : obj(agent ?? null)?.["name"];
  return typeof name === "string" && name !== "" ? name : null;
}

/**
 * The mode a new session starts in (for the new-chat composer): the configuration's `defaultAgent` from a
 * `config/read` result (lobby call, no session), when it is one of the modes this menu knows; else `null` (the chip
 * then says "Access", as before). Custom profile names come from the same configuration.
 */
export function configuredModeId(config: JsonValue | undefined): string | null {
  const o = obj(config ?? null);
  const view = obj(o?.["config"] ?? null) ?? o;
  const agent = view?.["defaultAgent"];
  const name = typeof agent === "string" ? agent : obj(agent ?? null)?.["name"];
  return typeof name === "string" && name !== "" ? name : null;
}

/** The mode with `id`, or a generic entry for a profile we do not know (a custom TOML profile). */
export function modeOf(id: string | null): AccessMode | null {
  if (id === null) return null;
  return (
    ACCESS_MODES.find((mode) => mode.id === id) ?? {
      id,
      label: term("accessModes", id).name,
      description: TEXT.customAccessDescription,
      tooltip: wording(TEXT.accessUse, { name: term("accessModes", id).name }),
      level: "neutral",
      confirm: false,
    }
  );
}
