/**
 * The Browser tab's lists: the session's servers it can open (with the owning process), its free leased
 * ports, "address in use" explanations with their options, downloads, and refused navigations.
 * Depends on: store.ts, protocol types. Called by: PreviewTab.
 * Invariants: process names, output lines, URLs and file names are untrusted text and only ever shown as text;
 * stopping a process is offered only where the core's explanation offers it (one of our sessions' processes).
 */
import type { ReactElement } from "react";

import type { BusyOption, BusyPortView, PreviewTarget, PreviewView } from "../../protocol/core";
import type { PreviewList, Previews } from "./store";

function TargetRow({
  target,
  onOpen,
  busy,
  shown,
}: {
  target: PreviewTarget;
  onOpen: () => void;
  busy: boolean;
  shown: boolean;
}): ReactElement {
  // The process id is a developer detail: it stays in the tooltip.
  const who =
    target.pid === null
      ? "nothing listens yet (leased)"
      : `${target.processName ?? "process"}${
          target.proven ? "" : " · found in this folder, not started by Vibe Work"
        }`;
  const tip = target.pid === null ? undefined : `Process id ${String(target.pid)}`;
  return (
    <li
      className="preview-target"
      data-port={target.port}
      data-listening={target.listening}
      aria-current={shown ? "true" : undefined}
    >
      <span className="preview-url">{target.url}</span>
      <span className="preview-who" title={tip}>
        {who}
      </span>
      <button type="button" className="preview-btn" disabled={busy} onClick={onOpen}>
        {shown ? "Open again" : "Open"}
      </button>
    </li>
  );
}

/** The port of a loopback URL, `null` when there is none. */
function portOf(url: string | null): number | null {
  if (url === null) return null;
  try {
    const parsed = new URL(url);
    return parsed.port === "" ? 80 : Number(parsed.port);
  } catch {
    return null;
  }
}

function optionLabel(option: BusyOption): string {
  switch (option.kind) {
    case "useNextLeasedPort":
      return `Use port ${String(option.port)}`;
    case "stopOwner":
      return `Stop pid ${String(option.pid)}`;
    case "openRunning":
      return "Open the running one";
    case "retry":
      return "Try again";
  }
}

function BusyRow({
  sessionId,
  item,
  previews,
  busy,
}: {
  sessionId: string;
  item: BusyPortView;
  previews: Previews;
  busy: boolean;
}): ReactElement {
  const port = item.port;
  const act = (option: BusyOption) => {
    if (option.kind === "stopOwner" && port !== null) void previews.stopOwner(sessionId, port);
    else if (option.kind === "openRunning") void previews.open(sessionId, { url: option.url });
    else if (option.kind === "useNextLeasedPort")
      void previews.open(sessionId, { port: option.port });
    else void previews.load(sessionId);
  };
  return (
    <li className="preview-busy" role="note">
      <p>
        Port {port ?? "?"} is in use by {item.holder}
        {item.holderPid !== null ? ` (pid ${String(item.holderPid)})` : ""}.
      </p>
      {item.line !== null && <code className="preview-line">{item.line}</code>}
      <span className="preview-options">
        {item.options.map((option) => (
          <button
            key={optionLabel(option)}
            type="button"
            className="preview-btn"
            disabled={busy}
            onClick={() => {
              act(option);
            }}
          >
            {optionLabel(option)}
          </button>
        ))}
      </span>
    </li>
  );
}

/** Downloads and refused navigations of an open preview. */
export function PreviewNotes({
  preview,
  showRefused = true,
}: {
  preview: PreviewView;
  showRefused?: boolean;
}): ReactElement | null {
  if (preview.downloads.length === 0 && (!showRefused || preview.refused.length === 0)) return null;
  return (
    <div className="preview-notes">
      {preview.downloads.length > 0 && (
        <ul aria-label="Downloads" className="preview-downloads">
          {preview.downloads.map((d) => (
            <li key={d.path} data-state={d.state} title={d.path}>
              {d.fileName} · {d.state === "done" ? "saved" : d.state}
            </li>
          ))}
        </ul>
      )}
      {showRefused && preview.refused.length > 0 && (
        <p className="preview-refused">
          Not opened (not this session's server): {preview.refused[preview.refused.length - 1]}
        </p>
      )}
    </div>
  );
}

/** The servers list and busy ports of a session. */
export function Servers({
  sessionId,
  list,
  previews,
  busy,
  emptyState = true,
}: {
  sessionId: string;
  list: PreviewList;
  previews: Previews;
  busy: boolean;
  /** Page-only guidance; the toolbar popover contains server choices and busy-port actions. */
  emptyState?: boolean;
}): ReactElement {
  const running = list.targets.filter((t) => t.listening);
  const shownPort = portOf(list.preview?.url ?? null);
  const free = list.targets.filter((t) => !t.listening);
  return (
    <div className="preview-servers">
      {list.busy.length > 0 && (
        <ul aria-label="Ports in use" className="preview-busy-list">
          {list.busy.map((item) => (
            <BusyRow
              key={`${String(item.port ?? "?")}-${item.holder}`}
              sessionId={sessionId}
              item={item}
              previews={previews}
              busy={busy}
            />
          ))}
        </ul>
      )}
      {running.length > 0 ? (
        <ul aria-label="This session's servers" className="preview-targets">
          {running.map((t) => (
            <TargetRow
              key={t.port}
              target={t}
              busy={busy}
              shown={shownPort === t.port}
              onOpen={() => void previews.open(sessionId, { port: t.port })}
            />
          ))}
        </ul>
      ) : emptyState && (list.preview?.url ?? null) === null ? (
        <p className="empty">
          No server of this session is running. Ask the agent to start one; its ports are{" "}
          {free.map((t) => t.port).join(", ") || "not leased yet"}.
        </p>
      ) : null}
    </div>
  );
}
