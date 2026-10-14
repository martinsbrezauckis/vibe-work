/**
 * The rows and the read/write hook of the Vibe settings in Settings: a selector row, a switch row,
 * the "set by" line, and `useVibeSettings` (read through a session, write, read again).
 * Depends on: AppContext, hooks, state/vibeSettings, state/modelChoice. Called by: AgentDefaultsSettings.
 * Invariants: a row a higher layer sets is disabled and says who sets it; the hook shows failures as text.
 * `apply` writes through the open chat and reloads it (Vibe takes these changes on a chat that exists).
 * `applyToNewChats` is for a setting Vibe refuses to change on a chat that exists (Subagents): the write never
 * touches the open chat, the row says the change is for new chats, and a short-lived chat the core could not
 * delete again is shown as a notification.
 */
import { type ReactElement, useCallback, useEffect, useState } from "react";

import { Selector } from "../../components/Selector";
import { sessionForPick } from "../../state/modelChoice";
import { type SubagentsSaved, type VibeSettings, readVibeSettings } from "../../state/vibeSettings";
import { useApp } from "../shell/AppContext";
import { useCoreState, useShellState } from "../shell/hooks";

const SETTERS: Record<string, string> = {
  environment: "Vibe Work for this run",
  admin: "your administrator",
  overrides: "a setting of this session",
};

/** Shown under a setting that was saved for new chats only. */
const NEW_CHATS_NOTE =
  "Saved for new chats. Chats you already have open keep the setting they started with.";

const LEFTOVER_TEXT =
  "The change was saved, but a helper chat that Vibe Work used for it could not be deleted. " +
  "It is in your chat list as a temporary chat and is deleted when Vibe Work quits; you can also delete it yourself.";

/** Reads the settings through a session and keeps them; `apply` writes and reads again. */
export function useVibeSettings() {
  const { core, toasts, sessionTools } = useApp();
  const running = useCoreState((s) => s.server.phase === "running");
  const selected = useShellState((s) => s.selected);
  const via = core === null ? null : sessionForPick(core, selected);
  const [view, setView] = useState<VibeSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (core === null || !running || via === null) return;
    let live = true;
    readVibeSettings(core, via)
      .then((v) => {
        if (live) setView(v);
      })
      .catch((thrown: unknown) => {
        if (live)
          setError(thrown instanceof Error ? thrown.message : "Could not read Vibe's settings.");
      });
    return () => {
      live = false;
    };
  }, [core, running, via]);
  const apply = useCallback(
    (work: (sessionId: string) => Promise<void>) => {
      if (core === null || via === null) return;
      setBusy(true);
      setError(null);
      work(via)
        .then(() => readVibeSettings(core, via))
        .then((v) => {
          setView(v);
        })
        .catch((thrown: unknown) => {
          setError(thrown instanceof Error ? thrown.message : "Could not save the change.");
        })
        .finally(() => {
          setBusy(false);
        });
    },
    [core, via],
  );
  const [savedForNewChats, setSavedForNewChats] = useState(false);
  /**
   * Saves a setting for new chats (`work` goes through the core's short-lived chat, never the open one), then reads
   * the settings again. The value a new chat starts with, as `work` reports it, replaces the Subagents value of
   * the re-read: that read goes through the open chat, whose answer for this setting may still be the value it
   * started with, and the row is about what new chats get.
   */
  const applyToNewChats = useCallback(
    (work: () => Promise<SubagentsSaved>) => {
      if (core === null || via === null) return;
      setBusy(true);
      setError(null);
      setSavedForNewChats(false);
      work()
        .then(async (saved) => {
          setSavedForNewChats(true);
          if (saved.leftover !== null) {
            toasts.push({
              kind: "warn",
              bellKind: "failure",
              text: LEFTOVER_TEXT,
              sessionId: saved.leftover,
            });
            // The chat is on the core's temporary list now: the list shows its "Temporary" chip.
            void sessionTools?.refresh();
          }
          const fresh = await readVibeSettings(core, via);
          setView(
            saved.enabled === null
              ? fresh
              : { ...fresh, subagents: { ...fresh.subagents, value: saved.enabled } },
          );
        })
        .catch((thrown: unknown) => {
          setError(thrown instanceof Error ? thrown.message : "Could not save the change.");
        })
        .finally(() => {
          setBusy(false);
        });
    },
    [core, via, toasts, sessionTools],
  );
  return {
    core,
    running,
    via,
    view,
    error,
    busy,
    apply,
    applyToNewChats,
    newChatsNote: savedForNewChats ? NEW_CHATS_NOTE : null,
  };
}

function lockLine(setBy: string | null): ReactElement | null {
  return setBy === null ? null : (
    <span className="d">Set by {SETTERS[setBy] ?? setBy}; it cannot be changed here.</span>
  );
}

/** A row with a selector. */
export function SelectRow(props: {
  id: string;
  label: string;
  detail: string;
  value: string;
  options: readonly { value: string; label: string; tooltip?: string }[];
  disabled: boolean;
  setBy: string | null;
  onChange: (value: string) => void;
}): ReactElement {
  return (
    <div className="check" data-setting={props.id}>
      <span>
        {props.label}
        <span className="d">{props.detail}</span>
        {lockLine(props.setBy)}
      </span>
      <Selector
        label={props.label}
        title={
          props.options.find((option) => option.value === props.value)?.tooltip ?? props.detail
        }
        value={props.value}
        options={props.options}
        disabled={props.disabled || props.setBy !== null}
        onChange={props.onChange}
      />
    </div>
  );
}

/** A row with a switch. */
export function SwitchRow(props: {
  id: string;
  label: string;
  detail: string;
  on: boolean | null;
  disabled: boolean;
  setBy: string | null;
  onFlip: (on: boolean) => void;
  /** A line under the detail after a change, e.g. that it applies to new chats only. */
  note?: string | null;
}): ReactElement {
  return (
    <div className="check" data-setting={props.id}>
      <span>
        {props.label}
        <span className="d">{props.detail}</span>
        {lockLine(props.setBy)}
        {props.note !== undefined && props.note !== null && (
          <span className="d" role="status">
            {props.note}
          </span>
        )}
      </span>
      <button
        type="button"
        className="switch"
        role="switch"
        aria-label={props.label}
        aria-checked={props.on === true}
        disabled={props.disabled || props.on === null || props.setBy !== null}
        onClick={() => {
          if (props.on !== null) props.onFlip(!props.on);
        }}
      />
    </div>
  );
}
