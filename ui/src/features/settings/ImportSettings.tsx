/**
 * General Settings: "Import from another agent". Look for setups shows what the other coding agents'
 * setups hold, grouped (instructions, skills, MCP servers, and what cannot be brought, with why). The person ticks
 * rows and presses Bring selected; the core's native confirmation lists every file it writes before anything
 * changes. The result says what was brought, with Undo; the last bring that can still be undone comes from the core
 * (it keeps the brings of this run), so its Undo is shown again when the card opens.
 * Depends on AppContext (core), shared controls and the core's `importSetup` command; called by SettingsPage.
 * Invariants: nothing is read until the person asks; a key is never shown, its row says where it goes once
 * (vocabulary `importKeyToVault`); rows that cannot be brought have no tick box. What a scan found, a bring's result and a problem are said once by
 * the app's announcer (their lines are no live regions).
 */
import { type ReactElement, useEffect, useRef, useState } from "react";

import { Button } from "../../components/Controls";
import { toCallError } from "../../protocol/client/types";
import type {
  ImportFound as Found,
  ImportKind as Kind,
  ImportResult,
  ImportSummary,
} from "../../protocol/core";
import { announce } from "../../state/announcer";
import { TEXT, wording } from "../../vocabulary";
import { useApp } from "../shell/AppContext";

const GROUPS: ReadonlyArray<{ kinds: Kind[]; title: string }> = [
  { kinds: ["instructions"], title: "Instructions" },
  { kinds: ["skill"], title: "Skills" },
  { kinds: ["mcpServer"], title: "Connectors (MCP servers)" },
  { kinds: ["permissions", "rules"], title: "Permissions and rules" },
  { kinds: ["hooks"], title: "Hooks" },
  { kinds: ["subagent", "command"], title: "Subagents and commands" },
];

/** The status word of a row. */
export function statusWord(row: Found): string {
  return row.status === "ready" ? "Ready" : row.status === "changed" ? "Changed" : "Can't bring";
}

/** What a scan found, in one sentence for the announcer. */
export function scanSaid(rows: readonly Found[]): string {
  if (rows.length === 0) return TEXT.importScanNone;
  if (rows.length === 1) return TEXT.importScanFoundOne;
  return wording(TEXT.importScanFound, { count: String(rows.length) });
}

/** The core's last bring of this run that can still be undone; `undefined` when it could not be read. */
async function lastBring(
  core: NonNullable<ReturnType<typeof useApp>["core"]>,
): Promise<ImportSummary | null | undefined> {
  try {
    const result = await core.run({ type: "importSetup", request: { op: "last" } });
    return result.type === "importSetup" && result.result.kind === "last"
      ? result.result.summary
      : undefined;
  } catch {
    // Without the core's answer the card shows no Undo; the next bring shows its own.
    return undefined;
  }
}

/** The plan's rows, ticks and the last bring. */
export function ImportSettings(): ReactElement {
  const { core } = useApp();
  const [rows, setRows] = useState<Found[] | null>(null);
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
  const [summary, setSummary] = useState<ImportSummary | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const run = async (request: Parameters<NonNullable<typeof core>["run"]>[0]) => {
    if (core === null) return null;
    setBusy(true);
    setProblem(null);
    try {
      const result = await core.run(request);
      return result.type === "importSetup" ? result.result : null;
    } catch (thrown: unknown) {
      const error = toCallError(thrown).error;
      if (error.code !== "confirmationDeclined") setProblem(error.message);
      return null;
    } finally {
      setBusy(false);
    }
  };
  const scan = async (said = true) => {
    const result: ImportResult | null = await run({ type: "importSetup", request: { op: "scan" } });
    if (result?.kind === "plan") {
      setRows(result.rows);
      setPicked(new Set());
      if (said) announce(scanSaid(result.rows));
    }
  };
  // The last bring of this run that can still be undone: the core keeps it, so leaving the card loses nothing. It is
  // read when the card opens and after an Undo (then the bring before it, if any), only from the core's memory:
  // nothing of the other agents is read. A summary read back this way is not announced again.
  const restored = useRef<string | null>(null);
  const showLast = (last: ImportSummary | null) => {
    restored.current = last?.bring ?? null;
    setSummary(last);
  };
  useEffect(() => {
    if (core === null) return;
    let live = true;
    void lastBring(core).then((last) => {
      if (!live || last === undefined) return;
      restored.current = last?.bring ?? null;
      setSummary(last);
    });
    return () => {
      live = false;
    };
  }, [core]);
  // A problem is said assertively, the result of a bring politely; their lines below are no live regions.
  useEffect(() => {
    if (problem !== null) announce(problem, { assertive: true });
  }, [problem]);
  useEffect(() => {
    if (summary !== null && summary.bring !== restored.current)
      announce(
        `Brought ${String(summary.brought)}, changed ${String(summary.changed)}, left ${String(summary.left.length)}.`,
      );
  }, [summary]);
  const bring = async () => {
    const picks = (rows ?? [])
      .filter((r) => picked.has(r.id))
      .map((r) => ({ id: r.id, fingerprint: r.fingerprint }));
    const result = await run({ type: "importSetup", request: { op: "apply", picks } });
    if (result?.kind === "brought") {
      setSummary(result.summary);
      await scan(false);
    }
  };
  const undo = async (id: string) => {
    const result = await run({ type: "importSetup", request: { op: "undo", bring: id } });
    if (result?.kind === "undone") {
      setSummary(null);
      if (core !== null) {
        const last = await lastBring(core);
        if (last !== undefined) showLast(last);
      }
      await scan(false);
    }
  };
  const toggle = (id: string) => {
    setPicked((now) => {
      const next = new Set(now);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };
  return (
    <section aria-labelledby="import-settings-title" data-setting="import-setup">
      <h3 id="import-settings-title">Import from another agent</h3>
      <p className="d">
        Instructions, skills and connectors from the other coding agents on this computer. Vibe Work
        shows what it found first; nothing in their folders changes.
      </p>
      <Button data-testid="settings.import-scan" disabled={busy} onClick={() => void scan()}>
        Look for setups
      </Button>
      {problem !== null && <p className="key-problem">{problem}</p>}
      {summary !== null && (
        <p data-testid="settings.import-summary">
          Brought {summary.brought}, changed {summary.changed}, left {summary.left.length}.{" "}
          <Button
            data-testid="settings.import-undo"
            appearance="key-link"
            disabled={busy}
            onClick={() => void undo(summary.bring)}
          >
            Undo
          </Button>
        </p>
      )}
      {/* Each row left behind, as the core reports it: its name and why it was not brought. */}
      {summary !== null && summary.left.length > 0 && (
        <ul className="d">
          {summary.left.map((why) => (
            <li key={why}>{why}</li>
          ))}
        </ul>
      )}
      {rows !== null && rows.length === 0 && <p className="d">{TEXT.importScanNone}</p>}
      {rows !== null &&
        GROUPS.map((group) => {
          const shown = rows.filter((r) => group.kinds.includes(r.kind));
          if (shown.length === 0) return null;
          return (
            <div key={group.title} className="import-group">
              <h4>{group.title}</h4>
              <ul>
                {shown.map((row) => (
                  <li key={row.id} data-item-id={row.id} data-status={row.status}>
                    {row.status === "cantBring" ? null : (
                      <input
                        data-testid="settings.import-pick"
                        type="checkbox"
                        aria-label={`Bring ${row.name}`}
                        checked={picked.has(row.id)}
                        onChange={() => {
                          toggle(row.id);
                        }}
                      />
                    )}{" "}
                    <strong>{row.name}</strong> · {row.agentLabel} · {statusWord(row)}
                    {row.target !== null && <> → {row.target}</>}
                    {row.detail !== null && (
                      <>
                        {" "}
                        · <code>{row.detail}</code>
                      </>
                    )}
                    {row.keys.length > 0 && row.status !== "cantBring" && (
                      <> · {TEXT.importKeyToVault}</>
                    )}
                    {row.note !== null && <span className="d"> — {row.note}</span>}
                  </li>
                ))}
              </ul>
            </div>
          );
        })}
      {rows !== null && rows.some((r) => r.status !== "cantBring") && (
        <Button
          data-testid="settings.import-bring"
          disabled={busy || picked.size === 0}
          onClick={() => void bring()}
        >
          Bring selected
        </Button>
      )}
    </section>
  );
}
