/**
 * One leaf list of the Allowed without asking page: its rows (`.check` rows that wrap: text left, Revoke right),
 * "Show 20 more ({n} not shown)", in the order and amount the view gives. Renders only; every action is the store's.
 * Depends on: components/Controls, allowed/store, view, entries. Called by: AllowedGroup.
 */
import type { ReactElement } from "react";

import { Button, SettingsRow } from "../../../components/Controls";
import type { CoversPart } from "../../../protocol/core";
import type { Allowed } from "./store";
import { type LeafView, moreLabel } from "./view";

/** Covers words: code parts as `<code>`. */
function Parts({ parts }: { parts: readonly CoversPart[] }): ReactElement {
  return (
    <>
      {parts.map((p, i) =>
        p.code ? <code key={i}>{p.text}</code> : <span key={i}>{p.text}</span>,
      )}
    </>
  );
}

/** The rows of `leaf` and its "Show 20 more". */
export function AllowedRows({
  leaf,
  allowed,
  busy,
}: {
  leaf: LeafView;
  allowed: Allowed;
  busy: Readonly<Record<string, true>>;
}): ReactElement {
  return (
    <>
      {leaf.rows.map((e) => {
        const running = busy[e.key] === true;
        return (
          <SettingsRow key={e.key} data-allowed-entry={e.key} data-kind={e.kind}>
            <span className="allow-what">
              <Parts parts={e.what} />
              <span className="d">{e.detail}</span>
            </span>
            <Button
              data-testid="settings.allowed-revoke"
              data-item-id={e.key}
              data-allowed-focus={`revoke:${e.key}`}
              shape="outlined"
              aria-label={`Revoke: ${e.name}`}
              disabled={running}
              onClick={() => void allowed.revoke(e.key)}
            >
              {running ? "Revoking…" : "Revoke"}
            </Button>
          </SettingsRow>
        );
      })}
      {leaf.notShown > 0 && (
        <div className="allow-more">
          <Button
            data-testid="settings.allowed-show-more"
            shape="outlined"
            onClick={() => {
              allowed.showMore(leaf.key);
            }}
          >
            {moreLabel(leaf.notShown)}
          </Button>
        </div>
      )}
    </>
  );
}
