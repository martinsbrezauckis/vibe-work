/**
 * Settings, "Models and agent" additions (//, Vibe 2.26.1): what new
 * sessions start with (access mode), what the agent may use (background
 * processes, subagents), when a silent answer counts as stalled, and the default model's sampling and output cap.
 * Every value is read from Vibe (`config/fields/read`) and written with Vibe's own methods
 * through a session (`state/vibeSettings.ts`); Full access as the default asks the native confirmation.
 * Depends on: state/vibeSettings, vibeSettingRows (rows and the read/write hook), composer access modes.
 * Called by: settings/SettingsPage (Models and agent).
 * Invariants: a value another layer sets (environment, administrator) is shown with who sets it and is not editable;
 * nothing is written until the user changes a control; failures are shown as text.
 * Subagents is the exception to "written through the open chat": Vibe refuses to change it on a chat that exists
 * and then breaks that chat, so it is saved for new chats through the core's short-lived chat and the row says so.
 */
import type { ReactElement } from "react";

import {
  type ConfigOp,
  modelPath,
  writeConfig,
  writeSubagentsForNewChats,
} from "../../state/vibeSettings";
import { ACCESS_MODES } from "../composer/model/access";
import { TEXT } from "../../vocabulary";
import { SelectRow, SwitchRow, useVibeSettings } from "./vibeSettingRows";

const TIMEOUTS = [0, 60, 120, 300, 600];
const TOP_P = [0.8, 0.9, 0.95, 1];
const CAPS = [4096, 8192, 16384, 32768, 65536];

/** Models and agent: new-session defaults and what the agent may use. */
export function AgentDefaultsSettings(): ReactElement | null {
  const { core, running, via, view, error, busy, apply, applyToNewChats, newChatsNote } =
    useVibeSettings();
  if (core === null || !running) return null;
  const model = view?.models.find((m) => m.alias === view.activeModel) ?? null;
  const set = (ops: ConfigOp[], reason: string, confirm = false) => {
    apply((sid) => writeConfig(core, sid, ops, reason, confirm));
  };
  return (
    <section aria-labelledby="agent-defaults-title">
      <h3 id="agent-defaults-title">New sessions and the agent</h3>
      {via === null && (
        <p className="key-line">
          Needs a session: Vibe reads and saves these settings through one.
        </p>
      )}
      {via !== null && view === null && error === null && (
        <p className="key-line">Reading the settings...</p>
      )}
      {view !== null && (
        <>
          <SelectRow
            id="default-access-mode"
            label={TEXT.accessMode}
            detail={TEXT.accessDefaultDescription}
            value={view.defaultAgent.value ?? ""}
            options={[
              ...(view.defaultAgent.value === null
                ? [{ value: "", label: TEXT.accessDefaultFallback }]
                : []),
              ...ACCESS_MODES.filter((m) => m.id !== "smart-approve").map((m) => ({
                value: m.id,
                label: m.label,
                tooltip: m.tooltip,
              })),
            ]}
            disabled={busy}
            setBy={view.defaultAgent.setBy}
            onChange={(id) => {
              set(
                [{ op: "set", path: "/default_agent", value: id }],
                "default access mode",
                id === "auto-approve",
              );
            }}
          />
          <SwitchRow
            id="background-processes"
            label="Background processes"
            detail="The agent may start commands that keep running (a dev server) and check on them. Off: commands run to the end before the agent goes on."
            on={view.backgroundProcesses.value}
            disabled={busy}
            setBy={view.backgroundProcesses.setBy}
            onFlip={(on) => {
              set(
                [{ op: "set", path: "/enable_background_processes", value: on }],
                "background processes",
              );
            }}
          />
          <SwitchRow
            id="subagents"
            label="Subagents"
            detail="The agent may hand parts of a task to helper agents. Off: it works alone."
            on={view.subagents.value}
            disabled={busy}
            setBy={view.subagents.setBy}
            note={newChatsNote}
            onFlip={(on) => {
              // Vibe refuses this change on a chat that exists and then breaks that chat: never through the open one.
              applyToNewChats(() => writeSubagentsForNewChats(core, on));
            }}
          />
          <SelectRow
            id="stream-idle-timeout"
            label="A silent answer counts as stalled after"
            detail="Once the model has started answering, this much silence makes Vibe retry. Off waits as long as the request may take."
            value={String(view.streamIdleTimeout.value ?? 0)}
            options={TIMEOUTS.map((t) => ({
              value: String(t),
              label: t === 0 ? "Off" : `${String(t)} seconds`,
            }))}
            disabled={busy}
            setBy={view.streamIdleTimeout.setBy}
            onChange={(v) => {
              set(
                [{ op: "set", path: "/api_stream_idle_timeout", value: Number(v) }],
                "stalled answer timeout",
              );
            }}
          />
          {model !== null && (
            <>
              <SelectRow
                id="model-top-p"
                label={`Sampling (top p) for ${model.label}`}
                detail="Lower keeps answers more focused. The provider's default suits most work."
                value={model.topP === null ? "" : String(model.topP)}
                options={[
                  { value: "", label: "Provider default" },
                  ...TOP_P.map((p) => ({ value: String(p), label: String(p) })),
                ]}
                disabled={busy}
                setBy={null}
                onChange={(v) => {
                  const path = modelPath(model.alias, "top_p");
                  set(
                    v === "" ? [{ op: "remove", path }] : [{ op: "set", path, value: Number(v) }],
                    "model sampling",
                  );
                }}
              />
              <SelectRow
                id="model-max-output"
                label={`Longest answer for ${model.label}`}
                detail="A cap on the tokens of one answer. An answer cut off at it stops there."
                value={model.maxOutputTokens === null ? "" : String(model.maxOutputTokens)}
                options={[
                  { value: "", label: "Provider default" },
                  ...CAPS.map((c) => ({ value: String(c), label: `${String(c / 1024)}k tokens` })),
                ]}
                disabled={busy}
                setBy={null}
                onChange={(v) => {
                  const path = modelPath(model.alias, "max_output_tokens");
                  set(
                    v === "" ? [{ op: "remove", path }] : [{ op: "set", path, value: Number(v) }],
                    "model output cap",
                  );
                }}
              />
            </>
          )}
        </>
      )}
      {error !== null && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
