/**
 * The "MCP" label of a vault key and the ordering the Add-server key picker uses.
 * Depends on: protocol/core (ProjectSecretView), components/Controls. Called by: ProjectSecretsSettings,
 * connectors/KeySection.
 * Invariants: the label is a SUGGESTION. Nothing here (or anywhere in the UI) changes what a key may do: the tag, the
 * hint and the picker order only point at the smoother way to use it; grants, consent and revoke are the core's and
 * identical for labelled and plain keys.
 */
import type { ReactElement } from "react";

import type { ProjectSecretView } from "../../protocol/core";
import { Badge } from "../../components/Controls";

/** The label word on the wire. */
const MCP_LABEL = "mcp";

/** True for a key labelled for an MCP server. */
export function isMcp(secret: Pick<ProjectSecretView, "label">): boolean {
  return secret.label === MCP_LABEL;
}

/** MCP-labelled keys first, the others after, each group in the order given (a stable partition). */
export function mcpFirst<T extends Pick<ProjectSecretView, "label">>(list: readonly T[]): T[] {
  return [...list.filter(isMcp), ...list.filter((s) => !isMcp(s))];
}

/** The small "MCP" tag shown next to a labelled key's name. */
export function McpTag(): ReactElement {
  return <Badge data-mcp-tag="true">MCP</Badge>;
}
