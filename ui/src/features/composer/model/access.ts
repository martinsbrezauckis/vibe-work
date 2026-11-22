/**
 * Access modes (Vibe's approval policies) as the composer's access menu shows them:
 * labels and the colour level of the composer's top rule (`.composer.lvl-*`), which profiles are
 * offered, and which one the session runs in.
 * Depends on: protocol/core types. Called by: the access menu, the composer view, composerSession.
 * Invariants:
 * - Smart approve is offered only when Vibe lists `smart-approve` among the session's policies (a Mistral
 *   dark launch); Vibe Work never forces it.
 * - Full access (`auto-approve`) needs consent: a trusted native menu pick decides; other paths need
 *   native confirmation. The gate always requires a token for the switch.
 * - A chat saved with the retired `vw-plan` agent opens in Ask.
 */
import type { JsonValue } from "../../../protocol/core";
import { term, TEXT, wording } from "../../../vocabulary";
import { type Obj, obj } from "../../transcript/model/entries";

/** The rule colour of a mode (classes `lvl-yolo|smart|warn|safe`; neutral has none). */
type AccessLevel = "yolo" | "smart" | "warn" | "neutral" | "safe";

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

/** Vibe's policies in menu order with the wording that matches the access rules. */
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
];

/** Names in a runtime list, or `null` without that list. */
function runtimeNames(runtime: JsonValue | undefined, key: string): string[] | null {
  const entries = obj(runtime)?.[key];
  if (!Array.isArray(entries)) return null;
  return entries.flatMap((a) => {
    const name = obj(a)?.["name"];
    return typeof name === "string" ? [name] : [];
  });
}

/**
 * Vibe's policy list bounds the offered modes.
 */
export function offeredModes(runtime: JsonValue | undefined): AccessMode[] {
  const policies = runtimeNames(runtime, "approvalPolicies") ?? runtimeNames(runtime, "agents");
  return ACCESS_MODES.filter((mode) => {
    if (mode.id === "smart-approve") return policies?.includes(mode.id) === true;
    return policies === null || policies.includes(mode.id);
  });
}

/**
 * The core's resolved mode is available before attachment and stays visible during verification.
 * Older clients fall back to the legacy agent-as-policy, then the session's saved policy.
 */
export function currentModeId(
  runtime: JsonValue | undefined,
  session: Obj | undefined,
  remembered?: string,
): string | null {
  if (remembered === "vw-plan") {
    const restored = obj(obj(runtime)?.["activeApprovalPolicy"] ?? null)?.["name"];
    return typeof restored === "string" && restored !== "" ? restored : "ask";
  }
  if (remembered !== undefined && remembered !== "" && remembered !== "default") return remembered;
  const activeAgent = obj(obj(runtime)?.["activeAgent"] ?? null)?.["name"];
  const activePolicy = obj(obj(runtime)?.["activeApprovalPolicy"] ?? null)?.["name"];
  if (typeof activePolicy === "string" && activePolicy !== "") return activePolicy;
  if (activeAgent === "vw-plan") return "ask";
  const active = activeAgent;
  if (typeof active === "string" && ACCESS_MODES.some((mode) => mode.id === active)) return active;
  const savedPolicy = session?.["approvalPolicy"];
  const savedName =
    typeof savedPolicy === "string" ? savedPolicy : obj(savedPolicy ?? null)?.["name"];
  if (typeof savedName === "string" && savedName !== "") return savedName;
  const agent = session?.["agent"];
  const name = typeof agent === "string" ? agent : obj(agent ?? null)?.["name"];
  if (name === "vw-plan") return "ask";
  return typeof name === "string" && ACCESS_MODES.some((mode) => mode.id === name) ? name : null;
}

/**
 * The mode a new session starts in (for the new-chat composer): the configuration's `defaultApprovalPolicy` from a
 * `config/read` result (lobby call, no session), when it is one of the modes this menu knows; else `null` (the chip
 * then says "Access", as before). A saved `vw-plan` default maps to Ask.
 */
export function configuredModeId(config: JsonValue | undefined): string | null {
  const o = obj(config ?? null);
  const view = obj(o?.["config"] ?? null) ?? o;
  const configured = view?.["defaultApprovalPolicy"] ?? view?.["default_approval_policy"];
  if (configured === undefined && view?.["defaultAgent"] === "vw-plan") return "ask";
  const agent = configured ?? view?.["defaultAgent"];
  const name = typeof agent === "string" ? agent : obj(agent ?? null)?.["name"];
  return typeof name === "string" &&
    name !== "" &&
    (configured !== undefined || ACCESS_MODES.some((mode) => mode.id === name))
    ? name
    : null;
}

/** The mode with `id`, or a generic entry for a profile we do not know (a custom TOML profile). */
export function modeOf(id: string | null): AccessMode | null {
  if (id === null) return null;
  if (id === "vw-plan") return ACCESS_MODES.find((mode) => mode.id === "ask") ?? null;
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
