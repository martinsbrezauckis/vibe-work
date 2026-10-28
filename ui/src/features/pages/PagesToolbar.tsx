/**
 * The Project tab's toolbar: handbook (the live hub), back, forward, reload, the page's path, Plan,
 * Comment, "Point Vibe at this", "Share as one file", and the "Sandboxed" badge (agent pages render in the isolated
 * engine with no app access and no network; a prototype's own script runs there).
 * Depends on: components/Icon, protocol types. Called by: PagesTab.
 * Invariants: renders only; every button calls a prop; the path is plain text (never markup).
 */
import { Button } from "../../components/Controls";
import { term } from "../../vocabulary";
import { formatKeys } from "../../state/shortcuts";
import type { ReactElement } from "react";

import { Icon, type IconName } from "../../components/Icon";

function Tool({
  testId,
  icon,
  label,
  onClick,
  disabled,
}: {
  icon: IconName;
  testId: string;
  label: string;
  onClick: () => void;
  disabled: boolean;
}): ReactElement {
  return (
    <button
      data-testid={testId}
      type="button"
      className="ibtn"
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
    >
      <Icon name={icon} size={14} />
    </button>
  );
}

/** Toolbar actions. */
export interface ToolbarActions {
  hub: () => void;
  back: () => void;
  forward: () => void;
  reload: () => void;
  plan: () => void;
  comment: () => void;
  point: () => void;
  cancelPick: () => void;
  /**: save the shown page as one self-contained file (native save dialog). */
  share: () => void;
}

/** The toolbar. `path` is the page shown (`null`: the hub); `picking` while a pick waits for the user. */
export function PagesToolbar({
  path,
  folder,
  busy,
  picking,
  prototype,
  actions,
}: {
  path: string | null;
  folder: string;
  busy: boolean;
  picking: boolean;
  /**: the shown page is a prototype (its own script runs, sandboxed, no network). */
  prototype: boolean;
  actions: ToolbarActions;
}): ReactElement {
  const onPage = path !== null;
  return (
    <div className="pages-toolbar" role="toolbar" aria-label={term("workNames", "projectTab").name}>
      <Button
        data-testid="pages.pages-toolbar-pages-btn"
        type="button"
        appearance="page"
        aria-pressed={!onPage}
        title="The project handbook: every page, grouped by kind"
        onClick={actions.hub}
      >
        <Icon name="book-open" size={14} />
        Handbook
      </Button>
      <Tool
        testId="pages.toolbar-back"
        icon="arrow-left"
        label="Back"
        disabled={!onPage || busy}
        onClick={actions.back}
      />
      <Tool
        testId="pages.toolbar-forward"
        icon="arrow-right"
        label="Forward"
        disabled={!onPage || busy}
        onClick={actions.forward}
      />
      <Tool
        testId="pages.toolbar-reload"
        icon="refresh-cw"
        label="Reload"
        disabled={!onPage || busy}
        onClick={actions.reload}
      />
      <span className="pages-path" title={path === null ? folder : `${folder}/${path}`}>
        {path === null ? `${folder}/` : `${folder}/${path}`}
      </span>
      <span
        className="pages-sandbox"
        title={
          prototype
            ? "A prototype page: its own script runs, isolated (no network, no storage, no other pages, no access to Vibe Work)"
            : "Pages Vibe writes render in an isolated view: no script, no access to Vibe Work, no network beyond the handbook folder"
        }
      >
        {prototype ? "Prototype · sandboxed" : "Sandboxed"}
      </span>
      <span className="pages-actions">
        <Button
          data-testid="pages.pages-toolbar-pages-btn-2"
          type="button"
          appearance="page"
          title={`Open the current plan (${formatKeys("Mod+Shift+P")})`}
          disabled={busy}
          onClick={actions.plan}
        >
          Plan page
        </Button>
        {picking ? (
          <Button
            data-testid="pages.pages-toolbar-pages-btn-3"
            type="button"
            appearance="page"
            onClick={actions.cancelPick}
          >
            Cancel pick
          </Button>
        ) : (
          <>
            <Button
              data-testid="pages.pages-toolbar-pages-btn-4"
              type="button"
              appearance="page"
              title="Select text or click a section of the page, then write a comment for Vibe"
              disabled={!onPage || busy}
              onClick={actions.comment}
            >
              Comment
            </Button>
            <Button
              data-testid="pages.pages-toolbar-pages-btn-5"
              type="button"
              appearance="page"
              title="Select text or click a section: it is added to your next message"
              disabled={!onPage || busy}
              onClick={actions.point}
            >
              Point Vibe at this
            </Button>
            <Tool
              testId="pages.toolbar-share"
              icon="download"
              label="Share as one file (styles, images and your answers in one HTML file)"
              disabled={!onPage || busy}
              onClick={actions.share}
            />
          </>
        )}
      </span>
    </div>
  );
}
