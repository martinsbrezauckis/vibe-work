/** The context meter's optional breakdown. Renders reported counts only; all reads live in the store.
 * Depends on shared controls and the generated protocol model; mounted by StatusBar on a person’s click.
 */
import { type RefObject, useEffect, useMemo, useRef } from "react";
import { createPortal } from "react-dom";
import { Button, IconButton } from "../../components/Controls";
import { formatTokens } from "../../state/account";
import {
  CONTEXT_LABELS,
  contextShare,
  createContextComposition,
} from "../../state/contextComposition";
import { useStore } from "../../state/createStore";
import { useContextPopover } from "./useContextPopover";
import type { CoreController } from "../../state/coreStore";

/** The open panel owns its read controller and releases it when dismissed. */
export function ContextComposition({
  core,
  sessionId,
  trigger,
  close,
}: {
  core: CoreController;
  sessionId: string;
  trigger: RefObject<HTMLButtonElement | null>;
  close: () => void;
}) {
  const controller = useMemo(() => createContextComposition(core, sessionId), [core, sessionId]);
  const state = useStore(controller.store, (value) => value);
  const panel = useRef<HTMLDivElement>(null);
  useEffect(() => controller.start(), [controller]);
  useContextPopover(panel, trigger, close);
  const breakdown = state.breakdown;
  const categories = breakdown?.categories ?? [];
  return createPortal(
    <div
      ref={panel}
      data-testid="shell.context-breakdown"
      id="context-composition"
      className="context-composition ui-panel"
      role="dialog"
      aria-labelledby="context-composition-title"
      popover="manual"
      tabIndex={-1}
    >
      <header>
        <h2 id="context-composition-title" className="ui-heading">
          Context
        </h2>
        <IconButton
          data-testid="shell.context-close"
          icon="x"
          label="Close"
          title="Close context breakdown"
          onClick={close}
        />
      </header>
      {state.status === "loading" && <p role="status">Loading…</p>}
      {state.status === "unavailable" && (
        <div className="context-composition-state">
          <span role="status">Unavailable</span>
          <Button data-testid="shell.context-retry" onClick={controller.refresh}>
            Retry
          </Button>
        </div>
      )}
      {breakdown && (
        <>
          <div className="context-composition-total">
            <span>Used</span>
            <strong>
              {breakdown.measured
                ? `${formatTokens(breakdown.usedTokens)} tokens`
                : "Not measured yet"}
            </strong>
            <span>
              {breakdown.measured
                ? contextShare(breakdown.usedTokens, breakdown.contextWindow)
                : "—"}
            </span>
          </div>
          {breakdown.measured &&
            (categories.length > 0 ? (
              <table>
                <caption>Last model call</caption>
                <thead>
                  <tr>
                    <th scope="col">Part</th>
                    <th scope="col">Estimated tokens</th>
                    <th scope="col">% of window</th>
                  </tr>
                </thead>
                <tbody>
                  {categories.map((category) => (
                    <tr key={category.id}>
                      <th scope="row">{CONTEXT_LABELS[category.id]}</th>
                      <td>{formatTokens(category.estimatedTokens)}</td>
                      <td>{contextShare(category.estimatedTokens, breakdown.contextWindow)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <p role="status">Breakdown unavailable</p>
            ))}
          <dl>
            <div>
              <dt>Context window</dt>
              <dd>
                {breakdown.contextWindow > 0
                  ? `${formatTokens(breakdown.contextWindow)} tokens`
                  : "Not reported"}
              </dd>
            </div>
            <div>
              <dt>Compaction at</dt>
              <dd>
                {breakdown.compactionThreshold > 0
                  ? `${formatTokens(breakdown.compactionThreshold)} tokens`
                  : "Not reported"}
              </dd>
            </div>
          </dl>
        </>
      )}
    </div>,
    document.body,
  );
}
