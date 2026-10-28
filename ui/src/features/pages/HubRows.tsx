/**
 * The rows of the project handbook's live hub: one section per group in index order, a row per
 * page (title, Markdown, earlier-format or prototype tag, the visual check badge, status, date, plan progress, comment
 * count, inputs, path, its findings), the
 * Archive folded in `<details>`, and a findings list.
 * Depends on: protocol types. Called by: Hub.tsx, the Projects overview's detail pane (findings list).
 * Invariants: renders only; all page text is plain text; opening a page calls `open` (none: rows are not buttons).
 */
import { Badge } from "../../components/Controls";
import type { ReactElement } from "react";

import type { Finding, HubGroup, HubPage, VisualReport } from "../../protocol/core";
import { VisualBadge } from "./VisualCheck";

/** A list of findings (rule, page, message), most severe first. */
export function Findings({
  findings,
  label,
}: {
  findings: readonly Finding[];
  label: string;
}): ReactElement | null {
  if (findings.length === 0) return null;
  return (
    <ul className="pages-findings" aria-label={label}>
      {findings.map((f, i) => (
        <li key={`${f.rule}-${f.page}-${String(i)}`} data-severity={f.severity}>
          <code>{f.rule}</code>
          <span>
            {f.page === "index.html" ? "" : `${f.page}: `}
            {f.message}
          </span>
        </li>
      ))}
    </ul>
  );
}

function PageRow({
  page,
  comments,
  findings,
  open,
  visual,
}: {
  page: HubPage;
  comments: number;
  findings: readonly Finding[];
  open: ((path: string) => void) | null;
  /**: the page's current visual check report (absent: not checked since it changed). */
  visual: VisualReport | undefined;
}): ReactElement {
  const meta = [page.status, page.modified].filter((x): x is string => x !== null).join(" · ");
  const shown = findings.filter((f) => f.severity !== "info" || page.lite);
  const body = (
    <>
      <b>
        {page.title}
        {page.lite && <Badge presentation="page-label">Markdown</Badge>}
        {page.kind === "prototype" && (
          <Badge presentation="page-label" title="Its own script runs, isolated: no network">
            Prototype
          </Badge>
        )}
        <VisualBadge report={visual} />
      </b>
      <span className="pages-hub-meta">
        {meta}
        {page.stepsTotal > 0 && ` · ${String(page.stepsDone)} of ${String(page.stepsTotal)} steps`}
        {comments > 0 && ` · ${String(comments)} comment${comments === 1 ? "" : "s"}`}
        {page.inputs > 0 && ` · asks for your input (${String(page.inputs)})`}
      </span>
      {page.description !== null && <span className="pages-hub-desc">{page.description}</span>}
      <span className="pages-hub-path">{page.path}</span>
      {shown.slice(0, 4).map((f, i) => (
        <span
          key={`${f.rule}-${String(i)}`}
          className="pages-flag"
          data-severity={f.severity}
          data-rule={f.rule}
        >
          {f.rule} {f.message}
        </span>
      ))}
      {shown.length > 4 && <span className="pages-flag">and {shown.length - 4} more</span>}
    </>
  );
  return (
    <li data-item-id={page.path}>
      {open === null ? (
        <div className="pages-hub-row">{body}</div>
      ) : (
        <button
          data-testid="pages.hub-rows-open"
          type="button"
          className="pages-hub-row"
          data-path={page.path}
          onClick={() => {
            open(page.path);
          }}
        >
          {body}
        </button>
      )}
    </li>
  );
}

/** The groups of the hub; the Archive is folded. */
export function Groups({
  groups,
  counts,
  findingsOf,
  open,
  visualOf = () => undefined,
}: {
  groups: readonly HubGroup[];
  counts: Readonly<Record<string, number | undefined>>;
  findingsOf: (path: string) => readonly Finding[];
  open: ((path: string) => void) | null;
  /**: a page's visual check report (the Projects overview passes none). */
  visualOf?: (path: string) => VisualReport | undefined;
}): ReactElement {
  return (
    <>
      {groups.map((group) => {
        const list = (
          <ul>
            {group.pages.map((page) => (
              <PageRow
                key={page.path}
                page={page}
                comments={counts[page.path] ?? 0}
                findings={findingsOf(page.path)}
                open={open}
                visual={visualOf(page.path)}
              />
            ))}
          </ul>
        );
        return (
          <section
            data-item-id={group.id}
            key={group.id}
            className="pages-hub-group"
            data-group={group.id}
          >
            {group.id === "archive" ? (
              <details>
                <summary data-testid="pages.hub-rows-summary">
                  <span>{group.label}</span>
                  <span>{group.pages.length}</span>
                </summary>
                {list}
              </details>
            ) : (
              <>
                <h4>
                  <span>{group.label}</span>
                  <span>{group.pages.length}</span>
                </h4>
                {list}
              </>
            )}
          </section>
        );
      })}
    </>
  );
}
