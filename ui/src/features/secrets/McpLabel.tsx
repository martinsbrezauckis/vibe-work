/**
 * The "MCP" label of a vault key: the small tag on a key meant for an MCP server, the info sign (ⓘ) that
 * says what the label is for, and the ordering the Add-server key picker uses.
 * Depends on: protocol/core (ProjectSecretView), components/Icon. Called by: ProjectSecretsSettings,
 * connectors/KeySection.
 * Invariants: the label is a SUGGESTION. Nothing here (or anywhere in the UI) changes what a key may do: the tag, the
 * hint and the picker order only point at the smoother way to use it; grants, consent and revoke are the core's and
 * identical for labelled and plain keys.
 */
import { type ReactElement, useId, useState } from "react";

import type { ProjectSecretView } from "../../protocol/core";
import { Icon } from "../../components/Icon";
import { Badge } from "../../components/Controls";

/** The label word on the wire. */
export const MCP_LABEL = "mcp";

/** What the ⓘ says: one plain sentence. */
export const MCP_HINT =
  "Store the keys of MCP servers here and choose “Always” when you connect the server, so it does not ask every time.";

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

/**
 * The info sign: a real button (reachable with Tab, opens with Enter or Space, closes with Escape or a second press)
 * whose sentence shows below it. Works in both themes (colours come from the tokens).
 */
export function McpInfo(): ReactElement {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <span className="mcp-info">
      <button
        data-testid="secrets.mcp-label-set-open"
        type="button"
        className="tb-btn mcp-info-btn"
        aria-label="About MCP keys"
        aria-expanded={open}
        aria-controls={id}
        data-mcp-info="true"
        onClick={() => {
          setOpen(!open);
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") setOpen(false);
        }}
      >
        <Icon name="info" />
      </button>
      {open ? (
        <span
          id={id}
          role="note"
          className="mcp-info-text key-field-hint"
          data-mcp-info-text="true"
        >
          {MCP_HINT}
        </span>
      ) : null}
    </span>
  );
}
