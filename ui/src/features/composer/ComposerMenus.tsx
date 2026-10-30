/**
 * The composer's menus: the access mode menu (Vibe's profiles), the model and
 * thinking menu (`config/model/write`), and the `/` menu (GUI commands and the session's skills).
 * Depends on: the composer model (access, models, slash), Selector. Called by: Composer, ComposerTools.
 * Invariants: render only; choices expose their selected state; native access picks carry
 * the event's trust through deferred work; no cost is shown (`model/models.ts`: Vibe's view has
 * no prices).
 */
import { type ReactElement, useEffect, useLayoutEffect, useRef } from "react";

import { createPortal } from "react-dom";

import { Selector } from "../../components/Selector";

import type { AccessMode } from "./model/access";
import type { ModelChoice } from "./model/models";
import { THINKING_LEVELS } from "./model/models";
import { itemDescription, itemName, itemRuns, type MenuItem } from "./model/slash";
import { modelTerm, TEXT, thinkingTerm, wording } from "../../vocabulary";
import { effectiveLevel } from "../../state/thinkingLevels";

/** The access mode picker passes the native option event's trust to the composer action. */
export function AccessMenu({
  modes,
  current,
  currentLabel,
  onPick,
  onOpen,
  onClose,
  defaultLabel,
}: {
  modes: readonly AccessMode[];
  current: string | null;
  currentLabel?: string;
  onPick: (id: string, trustedClick: boolean) => void;
  onOpen?: () => void;
  onClose?: () => void;
  defaultLabel?: string;
}): ReactElement {
  return (
    <Selector
      testId="composer.composer-menus-selector"
      compact
      includeValueInName
      dataChip="access"
      label={TEXT.accessMode}
      {...(defaultLabel === undefined ? {} : { hint: `New chats start in ${defaultLabel}` })}
      title={wording(TEXT.accessChoose, {
        name:
          currentLabel ?? modes.find((mode) => mode.id === current)?.label ?? TEXT.accessUnknown,
      })}
      value={current ?? TEXT.accessUnknown}
      {...(currentLabel === undefined ? {} : { valueLabel: currentLabel })}
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
  effort: picked,
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
  const choice = models.find((m) => m.alias === alias);
  const levels = choice?.thinkingLevels ?? THINKING_LEVELS;
  // A model that always thinks shows its set level as High, its only choice.
  const effort = effectiveLevel(choice?.alwaysThinks === true, picked);
  const label =
    modelLabel ?? models.find((model) => model.alias === alias)?.label ?? TEXT.modelFallback;
  return (
    <>
      <Selector
        testId="composer.composer-menus-on-model"
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
        testId="composer.composer-menus-on-effort"
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
  const anchor = useRef<HTMLSpanElement>(null);
  const overlay = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const place = () => {
      const composer = anchor.current?.closest(".composer");
      const menu = overlay.current;
      if (!composer || !menu) return;
      const box = composer.getBoundingClientRect();
      const above = Math.max(0, box.top - 16);
      const below = Math.max(0, window.innerHeight - box.bottom - 42);
      const up = above >= 200 || above >= below;
      const height = Math.min(372, up ? above : below);
      menu.style.position = "fixed";
      menu.style.maxHeight = `${String(height)}px`;
      menu.style.top = `${String(Math.max(8, up ? box.top - height - 8 : Math.min(box.bottom + 8, window.innerHeight - height - 34)))}px`;
      menu.style.left = `${String(Math.max(8, Math.min(box.left, window.innerWidth - 288)))}px`;
      menu.style.width = `${String(Math.min(560, box.width, window.innerWidth - 16))}px`;
      menu.style.bottom = "auto";
      menu.style.right = "auto";
    };
    place();
    const observer = new ResizeObserver(place);
    const composer = anchor.current?.closest(".composer");
    if (composer) observer.observe(composer);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, []);
  const selected = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    // jsdom has no scrollIntoView; a real webview keeps the arrow-key selection in sight.
    const row = selected.current;
    const scroll: unknown = row === null ? undefined : Reflect.get(row, "scrollIntoView");
    if (row !== null && typeof scroll === "function") scroll.call(row, { block: "nearest" });
  }, [index, items]);
  const current = items[index];
  const menu =
    items.length === 0 ? (
      <div
        ref={overlay}
        data-testid="composer.composer-menus-slash"
        className="slash"
        role="listbox"
        aria-label="Commands and skills"
      >
        <div className="slash-empty" role="status">
          No command matches
        </div>
      </div>
    ) : (
      <div
        ref={overlay}
        data-testid="composer.composer-menus-slash-2"
        className="slash"
        role="listbox"
        aria-label="Commands and skills"
      >
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
              <div data-item-id={`${item.kind}:${name}`} key={`${item.kind}:${name}`}>
                {heading}
                <div
                  data-testid="composer.composer-menus-it"
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
                  }}
                  onClick={() => {
                    onPick(item);
                  }}
                  onMouseMove={() => {
                    if (i !== index) onHover(i);
                  }}
                >
                  <b>
                    {item.kind === "manage" ? (
                      "Manage skills…"
                    ) : (
                      <MarkedName name={name} hit={item.hit} length={query.length} />
                    )}
                  </b>
                  <span>{itemDescription(item)}</span>
                </div>
              </div>
            );
          })}
        </div>
      </div>
    );
  return (
    <>
      <span ref={anchor} hidden />
      {createPortal(menu, document.body)}
    </>
  );
}
