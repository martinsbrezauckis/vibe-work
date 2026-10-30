/**
 * Settings › Models and agent, "Compaction point": one row per configured model with the point (in tokens) where
 * Vibe compacts a chat's history. Default removes the user's own value; a pick writes the model's
 * `auto_compact_threshold` through `config/write` (`state/vibeSettings` `writeConfig`).
 * Depends on: state/compaction (rows and their choices), Controls, Selector, composer contextWindow (the model's
 * context label), vocabulary. Called by: settings/AgentDefaultsSettings, which owns the read and the write.
 * Invariants: only the choices `state/compaction` offers are shown; a row another layer sets is disabled and says
 * who sets it; nothing is written until the user picks.
 */
import type { ReactElement } from "react";

import { SettingsRow } from "../../components/Controls";
import { Selector } from "../../components/Selector";
import type { CompactionRow } from "../../state/compaction";
import type { ConfigOp } from "../../state/vibeSettings";
import { modelTerm, TEXT, wording } from "../../vocabulary";
import { contextWindowLabel } from "../composer/model/contextWindow";

const tokens = (n: number) => n.toLocaleString("en-US");

/** The JSON-pointer path of a model's threshold (`~` and `/` escaped as Vibe reads them). */
export function thresholdPath(alias: string): string {
  return `/models/${alias.replaceAll("~", "~0").replaceAll("/", "~1")}/auto_compact_threshold`;
}

/** The section; `lockLine` renders who sets a locked row, `onWrite` writes one model's ops. */
export function CompactionRows(props: {
  rows: readonly CompactionRow[];
  busy: boolean;
  lockLine: (setBy: string | null) => ReactElement | null;
  onWrite: (ops: ConfigOp[]) => void;
}): ReactElement | null {
  if (props.rows.length === 0) return null;
  return (
    <section aria-labelledby="compaction-point-title" data-setting="compaction-point">
      <h3 id="compaction-point-title">{TEXT.compactionPoint}</h3>
      {props.rows.map((row) => {
        const name = modelTerm(
          row.alias,
          row.model["display_name"] ?? row.model["displayName"],
        ).name;
        const label = wording(TEXT.compactionPointFor, { model: name });
        const window = row.model["max_context_length"] ?? row.model["maxContextLength"];
        const context = contextWindowLabel({
          alias: row.alias,
          ...(typeof row.model["name"] === "string" ? { name: row.model["name"] } : {}),
          ...(typeof row.model["provider"] === "string" ? { provider: row.model["provider"] } : {}),
          ...(typeof window === "number" ? { maxContextLength: window } : {}),
        });
        return (
          <SettingsRow key={row.alias} data-item-id={row.alias}>
            <span>
              {name}
              <span className="d">{context}</span>
              {props.lockLine(row.setBy)}
            </span>
            <Selector
              testId="settings.compaction-point"
              itemId={row.alias}
              label={label}
              title={label}
              value={row.own === null ? "" : String(row.own)}
              options={[
                {
                  value: "",
                  label: wording(TEXT.compactionDefault, { count: tokens(row.fallback) }),
                },
                ...row.choices.map((n) => ({ value: String(n), label: tokens(n) })),
              ]}
              disabled={props.busy || row.setBy !== null}
              onChange={(value) => {
                const path = thresholdPath(row.alias);
                props.onWrite(
                  value === ""
                    ? [{ op: "remove", path }]
                    : [{ op: "set", path, value: Number(value) }],
                );
              }}
            />
          </SettingsRow>
        );
      })}
    </section>
  );
}
