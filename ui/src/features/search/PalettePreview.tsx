/**
 * The command palette's preview pane: the highlighted result larger, with its
 * type, its path, the matched text and what Enter does.
 * Depends on: components/Highlight, components/Icon, paletteTypes. Called by: CommandPalette.
 */
import type { ReactElement } from "react";

import { Highlight } from "../../components/Highlight";
import { Icon } from "../../components/Icon";
import { GROUPS, type Result } from "./paletteTypes";

/** The preview of `result` (a note when nothing is selected). */
export function PalettePreview({
  result,
  query,
}: {
  result: Result | undefined;
  query: string;
}): ReactElement {
  if (result === undefined)
    return (
      <aside className="pal-pv" aria-label="Preview">
        <p className="pv-go">Nothing selected.</p>
      </aside>
    );
  const meta = GROUPS.find((g) => g.kind === result.kind);
  return (
    <aside className="pal-pv" aria-label="Preview" data-kind={result.kind}>
      <div className="pv-kind">
        <Icon name={meta?.icon ?? "zap"} size={14} />
        {meta?.one ?? "Result"}
      </div>
      <div className="pv-path">{result.path.join(" › ")}</div>
      <div className="pv-card">
        <b>
          <Highlight text={result.title} query={query} />
        </b>
        {result.detail !== "" && result.detail !== result.title && (
          <span>
            <Highlight text={result.detail} query={query} />
          </span>
        )}
      </div>
      <p className="pv-go">
        <Icon name="corner-down-left" size={12} />
        {result.opens}
      </p>
    </aside>
  );
}
