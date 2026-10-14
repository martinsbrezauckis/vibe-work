/**
 * Settings choices for the default model and new-chat thinking level. Models and levels come
 * from the runtime. The model pick is saved by the core without a session; the thinking preference lives
 * in app settings and is applied before a new chat's first turn.
 * Depends on: AppContext, hooks, modelChoice, defaultReasoning, SelectRow, Selector. Called by: settings/SettingsPage.
 * Invariants: nothing is written until the user picks. Unsupported saved levels stay saved while
 * the UI explains the fallback to the model's own default.
 */
import { useEffect, useState } from "react";
import { Selector } from "../../components/Selector";
import { useStore } from "../../state/createStore";
import { offeredReasoning } from "../../state/defaultReasoning";
import { modelTerm, TEXT, thinkingTerm, wording } from "../../vocabulary";

import { readModels, writeDefaultModel, type ModelList } from "../../state/modelChoice";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { SelectRow } from "../settings/vibeSettingRows";

/** The section (renders nothing without a running Vibe). */
export function ModelSettings() {
  const { core, composer } = useApp();
  const savedLevel = useStore(composer.store, (s) => s.defaultReasoning);
  const running = useCoreState((s) => s.server.phase === "running");
  const [list, setList] = useState<ModelList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);

  useEffect(() => {
    if (core === null || !running) return;
    let live = true;
    readModels(core)
      .then((models) => {
        if (live) setList(models);
      })
      .catch((thrown: unknown) => {
        if (live) setError(thrown instanceof Error ? thrown.message : "Could not read the models.");
      });
    return () => {
      live = false;
    };
  }, [core, running]);

  if (core === null || !running) return null;
  const pick = (alias: string) => {
    setSaving(true);
    setError(null);
    writeDefaultModel(core, alias)
      .then(() => {
        setList((l) => (l === null ? l : { ...l, current: alias }));
        setSaved(alias);
      })
      .catch((thrown: unknown) => {
        setError(thrown instanceof Error ? thrown.message : "Could not change the model.");
      })
      .finally(() => {
        setSaving(false);
      });
  };
  const model = list?.models.find((m) => m.alias === list.current);
  const savedLabel = list?.models.find((m) => m.alias === saved)?.label ?? saved;
  const levels = model?.thinkingLevels ?? [];
  const level = offeredReasoning(savedLevel, levels);

  return (
    <section aria-labelledby="model-settings-title" data-setting="default-model">
      <h3 id="model-settings-title">{TEXT.defaultModel}</h3>
      {list === null && error === null && <p className="key-line">Reading the models...</p>}
      {list !== null && list.models.length === 0 && (
        <p className="key-line">Vibe lists no models yet.</p>
      )}
      {list !== null && list.models.length > 0 && (
        <div className="check">
          <span>New sessions start with</span>
          <Selector
            label={TEXT.defaultModel}
            title={
              model === undefined
                ? TEXT.defaultModelDescription
                : modelTerm(model.alias, model.label).tooltip
            }
            value={list.current ?? ""}
            disabled={saving}
            options={[
              ...(list.current === null ? [{ value: "", label: "Choose" }] : []),
              ...list.models.map((m) => ({
                value: m.alias,
                label: modelTerm(m.alias, m.label).name,
              })),
            ]}
            onChange={(value) => {
              if (value !== "") pick(value);
            }}
          />
        </div>
      )}
      {saved !== null && error === null && (
        <p className="key-field-hint" role="status">
          Saved. New sessions start with {savedLabel}.
        </p>
      )}
      {error !== null && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
      <SelectRow
        id="default-thinking"
        label={TEXT.defaultThinkingLevel}
        detail={TEXT.defaultThinkingDescription}
        value={level ?? ""}
        options={[
          {
            value: "",
            label: TEXT.modelDefaultThinking,
            tooltip: TEXT.modelDefaultThinkingDescription,
          },
          ...levels.map((l) => ({
            value: l,
            label: thinkingTerm(l).name,
            tooltip: thinkingTerm(l).tooltip,
          })),
        ]}
        disabled={list === null || model === undefined || saving}
        setBy={null}
        onChange={(value) => {
          composer.setDefaultReasoning(value === "" ? null : value);
        }}
      />
      {savedLevel !== null && level === null && model !== undefined && (
        <p className="key-field-hint" role="status">
          {wording(TEXT.thinkingUnavailable, {
            level: thinkingTerm(savedLevel).name,
            model: model.label,
          })}
        </p>
      )}
    </section>
  );
}
