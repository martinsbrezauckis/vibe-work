/**
 * The composer's menus: the access mode menu (Vibe's profiles), the model and
 * thinking menu (`config/model/write`), and the `/` menu (GUI commands and the session's skills).
 * Depends on: the composer model (access, models, slash), Selector. Called by: Composer, ComposerTools.
 * Invariants: render only; choices expose their selected state; native access picks carry
 * the event's trust through deferred work; no cost is shown (`model/models.ts`: Vibe's view has
 * no prices).
 */
import { type ReactElement, useEffect, useRef } from "react";

import { Selector } from "../../components/Selector";

import type { AccessMode } from "./model/access";
import type { ModelChoice } from "./model/models";
import { THINKING_LEVELS } from "./model/models";
import { itemDescription, itemName, itemRuns, type MenuItem } from "./model/slash";
import { modelTerm, TEXT, thinkingTerm, wording } from "../../vocabulary";

/** The access mode picker passes the native option event's trust to the composer action. */
export function AccessMenu({
  modes,
  current,
  currentLabel,
  onPick,
  onOpen,
  onClose,
}: {
  modes: readonly AccessMode[];
  current: string | null;
  currentLabel?: string;
  onPick: (id: string, trustedClick: boolean) => void;
  onOpen?: () => void;
  onClose?: () => void;
}): ReactElement {
  return (
    <Selector
      compact
      includeValueInName
      dataChip="access"
      label={TEXT.accessMode}
      title={wording(TEXT.accessChoose, {
        name:
          currentLabel ?? modes.find((mode) => mode.id === current)?.label ?? TEXT.accessUnknown,
      })}
      value={current ?? TEXT.accessUnknown}
      options={modes.map((mode) => ({
        value: mode.id,
        label: mode.label,
        description: mode.description,
        tooltip: mode.tooltip,
        tone: mode.level === "yolo" ? "never" : mode.level === "safe" ? "allowed" : "neutral",
        dataMode: mode.id,
      }))}
      onChange={onPick}
      {...(onOpen === undefined ? {} : { onOpen })}
      {...(onClose === undefined ? {} : { onClose })}
    />
  );
}

/** Model and thinking each use the shared pick-one control. */
export function ModelMenu({
  models,
  alias,
  effort,
  onModel,
  onEffort,
  onOpen,
  onClose,
  modelLabel,
}: {
  models: readonly ModelChoice[];
  alias: string | null;
  effort: string | null;
  onModel: (alias: string) => void;
  onEffort: (level: string) => void;
  onOpen?: () => void;
  onClose?: () => void;
  modelLabel?: string;
}): ReactElement {
  const levels = models.find((m) => m.alias === alias)?.thinkingLevels ?? THINKING_LEVELS;
  const label =
    modelLabel ?? models.find((model) => model.alias === alias)?.label ?? TEXT.modelFallback;
  return (
    <>
      <Selector
        compact
        label={wording(TEXT.modelChoose, { name: label })}
        title={
          models.length === 0
            ? "No models are configured."
            : wording(TEXT.modelChoose, { name: label })
        }
        value={alias ?? label}
        {...(onOpen === undefined ? {} : { onOpen })}
        {...(onClose === undefined ? {} : { onClose })}
        onChange={(model) => {
          onModel(model);
        }}
        emptyLabel="No models are configured."
        options={models.map((model) => ({
          value: model.alias,
          label: modelTerm(model.alias, model.label).name,
          tooltip: modelTerm(model.alias, model.label).tooltip,
          description: `${model.contextLabel}${model.supportsImages ? " · sees images" : ""}`,
        }))}
      />
      <Selector
        compact
        label={wording(TEXT.thinkingChoose, {
          name: effort === null ? TEXT.modelDefaultThinking : thinkingTerm(effort).name,
        })}
        title={wording(TEXT.thinkingChoose, {
          name: effort === null ? TEXT.modelDefaultThinking : thinkingTerm(effort).name,
        })}
        value={effort ?? ""}
        {...(onOpen === undefined ? {} : { onOpen })}
        {...(onClose === undefined ? {} : { onClose })}
        onChange={(level) => {
          if (level !== "") onEffort(level);
        }}
        options={[
          ...(effort === null ? [{ value: "", label: TEXT.modelDefaultThinking }] : []),
          ...levels.map((level) => ({
            value: level,
            label: thinkingTerm(level).name,
            tooltip: thinkingTerm(level).tooltip,
          })),
        ]}
      />
    </>
  );
}

/** The name with the typed letters marked (`hit` is where they start; -1 marks nothing). */
function MarkedName({
  name,
  hit,
  length,
}: {
  name: string;
  hit: number;
  length: number;
}): ReactElement {
  if (hit < 0 || length === 0) return <>/{name}</>;
  return (
    <>
      /{name.slice(0, hit)}
      <mark>{name.slice(hit, hit + length)}</mark>
      {name.slice(hit + length)}
    </>
  );
}

/**
 * The `/` menu above the text box: one row per command, "/name" and ONE short description, about eight rows
 * visible and a scroll for the rest. `query` is the text after the slash (its letters are marked in the names); the
 * selected row follows the arrow keys and the mouse, and a small tooltip above the menu describes the action
 * and its keys. No match gives "No command matches".
 */
export function SlashMenu({
  items,
  index,
  query,
  onPick,
  onHover,
}: {
  items: readonly MenuItem[];
  index: number;
  query: string;
  onPick: (item: MenuItem) => void;
  onHover: (index: number) => void;
}): ReactElement {
  const selected = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    // jsdom has no scrollIntoView; a real webview keeps the arrow-key selection in sight.
    const row = selected.current;
    const scroll: unknown = row === null ? undefined : Reflect.get(row, "scrollIntoView");
    if (row !== null && typeof scroll === "function") scroll.call(row, { block: "nearest" });
  }, [index, items]);
  if (items.length === 0) {
    return (
      <div className="slash" role="listbox" aria-label="Commands and skills">
        <div className="slash-empty" role="status">
          No command matches
        </div>
      </div>
    );
  }
  const current = items[index];
  return (
    <div className="slash" role="listbox" aria-label="Commands and skills">
      {current === undefined ? null : (
        <div className="slash-tip" role="tooltip" data-slash-tip={itemName(current)}>
          {itemRuns(current)}
        </div>
      )}
      <div className="slash-list">
        {items.map((item, i) => {
          const section = item.kind === "command" ? "Vibe commands" : "Skills";
          const first = i === 0 || items[i - 1]?.kind !== item.kind;
          const heading = first ? <div className="secl">{section}</div> : null;
          const name = itemName(item);
          return (
            <div key={`${item.kind}:${name}`}>
              {heading}
              <div
                className="it"
                role="option"
                aria-selected={i === index}
                aria-description={itemRuns(item)}
                aria-label={itemRuns(item)}
                title={itemRuns(item)}
                ref={i === index ? selected : undefined}
                data-slash={name}
                onMouseDown={(e) => {
                  e.preventDefault();
                  onPick(item);
                }}
                onMouseMove={() => {
                  if (i !== index) onHover(i);
                }}
              >
                <b>
                  <MarkedName name={name} hit={item.hit} length={query.length} />
                </b>
                <span>{itemDescription(item)}</span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
