/**
 * Compact location and expandable full text. Uses shared icons/styles; called by WorktreeChip.
 * The tooltip and accessible name always retain every name and the base, including when clipped.
 */
import { Badge } from "../../components/Controls";
import { type ReactElement, useId, useState } from "react";
import { Icon } from "../../components/Icon";
import type { WorktreeInfo } from "../../protocol/core";

/** Keeps the header to one line until the user asks for the complete location. */
export function LocationChip({
  text,
  path,
  state,
}: {
  text: string;
  path: string | undefined;
  state: WorktreeInfo["state"];
}): ReactElement {
  const [expanded, setExpanded] = useState(false);
  const id = useId();
  return (
    <div className="wt-location">
      <Badge
        data-testid="worktrees.location-toggle"
        type="button"
        presentation="worktree"
        state={state}
        as="button"
        title={[text, path].filter(Boolean).join("\n")}
        aria-label={text}
        aria-expanded={expanded}
        aria-controls={id}
        data-worktree="chip"
        onClick={() => {
          setExpanded((value) => !value);
        }}
      >
        <Icon name="git-branch" size={12} />
        <span className="wt-chip-text">{text}</span>
        <Icon name={expanded ? "chevron-up" : "chevron-down"} size={12} />
      </Badge>
      <div id={id} className="wt-location-full" hidden={!expanded}>
        <div>{text}</div>
        {path !== undefined && <div className="wt-path">{path}</div>}
      </div>
    </div>
  );
}
