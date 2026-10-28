/** Renders permission and question cards with their offered answers. Depends on shared controls and card focus; called by the ask adapters. */
import {
  type KeyboardEvent,
  type MouseEvent,
  type ReactElement,
  type ReactNode,
  useEffect,
  useRef,
} from "react";
import { Button } from "../../components/Controls";
import { useFirstSight } from "../../hooks/useFirstSight";
import type { CoversPart } from "../../protocol/core";
import { TEXT, wording } from "../../vocabulary";
import { registerCardFocus } from "./cardFocus";
import { type AskButton, type AskChoice } from "./model";
import { focusCard, revealCardControl, useCardLayout } from "./useCardLayout";

/** What one card shows; the adapter fills it from Vibe's callback, a protected change or the ask metadata. */
export interface AskCardProps {
  /** Stable id: the callback id, `protected:{id}` or the ask id (first-sight motion, data attributes). */
  id: string;
  /** Extra frame class (`protected-change`, `qcard`). */
  className?: string;
  question: string;
  /** "Vibe · “Add city search”"; `whoTitle` holds the full session title for the tooltip. */
  who: string;
  whoTitle?: string;
  /** `.summary`: the exact what (mono). */
  summary?: string;
  /** `.reason` (orange): why it asks above a level, or a protected change's reason. */
  reason?: string | null;
  /** The covers words of "Allow for this session" (`null`/absent: no covers line). */
  covers?: readonly CoversPart[] | null;
  /** The payload: command, diff, input, files, question fields. */
  children?: ReactNode;
  buttons: readonly AskButton[];
  disabled: boolean;
  onChoose?: (choice: AskChoice) => void;
  /** The card-state line (sending, refused, not offered, native confirmation). */
  state: string;
  stateRole?: "status" | "alert";
  /** Buttons other than the answer set (a question's Submit / Skip). */
  actions?: ReactNode;
  /** Keys 1-4 answer (not for a question). */
  keys?: boolean;
  /** Extra attributes for automation (`data-callback-id`, `data-entry-id`). */
  data?: Readonly<Record<`data-${string}`, string>>;
  /** Wire decision per answer, kept on the buttons for automation (`data-decision`). */
  decisionOf?: (choice: AskChoice) => string;
  /** Told whether the focus is inside the card (`useCardClose`). */
  onFocusInside?: (inside: boolean) => void;
}

/** "1, 2 or 3": the keys of `buttons` for the card's description. */
function keyList(buttons: readonly AskButton[]): string {
  const keys = buttons.map((b) => String(b.key));
  return keys.length <= 1
    ? keys.join("")
    : `${keys.slice(0, -1).join(", ")} or ${keys.at(-1) ?? ""}`;
}

/** The card's answer keys for a screen reader. */
function description(buttons: readonly AskButton[], keys: boolean): string {
  return keys && buttons.length > 0 ? wording(TEXT.askKeysHint, { keys: keyList(buttons) }) : "";
}

/** True when the focused element takes typing (the arrival focus never steals it). */
function typing(): boolean {
  const active = document.activeElement;
  return (
    active instanceof HTMLElement &&
    (active.isContentEditable || active.matches("input, textarea, select, [role=textbox]"))
  );
}

/** True when the focus is inside another ask card: the user may be answering it by keyboard right now. */
function inAnotherCard(frame: HTMLElement | null): boolean {
  const active = document.activeElement;
  const card = active instanceof HTMLElement ? active.closest(".approval") : null;
  return card !== null && card !== frame;
}

/** The covers line: "Allow for this session covers {words}." with code pieces in mono. */
function CoversLine({ parts }: { parts: readonly CoversPart[] }): ReactElement {
  return (
    <span className="perms">
      Allow for this session covers{" "}
      {parts.map((part, i) => (part.code ? <code key={i}>{part.text}</code> : part.text))}.
    </span>
  );
}

/** Renders one ask. */
export function AskCard(props: AskCardProps): ReactElement {
  const { buttons, disabled, onChoose } = props;
  const fresh = useFirstSight(`card:${props.id}`);
  const frame = useRef<HTMLDivElement>(null);
  const maxHeight = useCardLayout(frame);
  useEffect(() => {
    if (fresh && !disabled && !typing() && !inAnotherCard(frame.current)) focusCard(frame.current);
  }, [fresh, disabled]);
  // "Go to the waiting prompt" asks for this card by id.
  useEffect(
    () =>
      registerCardFocus(props.id, () => {
        focusCard(frame.current);
      }),
    [props.id],
  );
  const choose = (choice: AskChoice) => {
    if (!disabled) onChoose?.(choice);
  };
  const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
    // A held key repeats: only its first press answers (the next card may already have the focus).
    if (props.keys === false || event.repeat) return;
    const target = event.target;
    if (
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement ||
      (target instanceof HTMLElement && target.closest('[role="combobox"], [role="listbox"]')) ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey
    )
      return;
    const button = buttons.find((b) => String(b.key) === event.key);
    if (button !== undefined) {
      event.preventDefault();
      choose(button.choice);
    }
  };
  return (
    <div
      ref={frame}
      style={{ maxHeight }}
      tabIndex={-1}
      data-testid="transcript.approval-card-div"
      className={`approval${props.className === undefined ? "" : ` ${props.className}`}${fresh ? " arrive" : ""}`}
      role="group"
      aria-label={`${props.question}${props.summary === undefined ? "" : `: ${props.summary}`}`}
      aria-description={description(buttons, props.keys !== false)}
      data-card-id={props.id}
      onKeyDown={onKey}
      onFocusCapture={(event) => {
        if (event.target instanceof HTMLElement) revealCardControl(event.target);
        props.onFocusInside?.(true);
      }}
      onBlurCapture={(event) => {
        const next = event.relatedTarget;
        // A native button loses focus when sending disables it. Keep the card's focus ownership until
        // the answer settles, so its replacement can return focus to the composer.
        if (next === null && event.target instanceof HTMLButtonElement && event.target.disabled)
          return;
        props.onFocusInside?.(next instanceof Node && event.currentTarget.contains(next));
      }}
      {...props.data}
    >
      <div className="ah">
        <span className="px needs" aria-hidden="true" />
        <b>{props.question}</b>
        <span className="sub" title={props.whoTitle ?? props.who}>
          {props.who}
        </span>
      </div>
      <div className="ask-body">
        {props.summary !== undefined || props.reason || props.covers ? (
          <div className="why">
            {props.summary === undefined ? null : <span className="summary">{props.summary}</span>}
            {props.reason ? <span className="reason">{props.reason}</span> : null}
            {props.covers && props.covers.length > 0 ? <CoversLine parts={props.covers} /> : null}
          </div>
        ) : null}
        {props.children}
      </div>
      {buttons.length > 0 || props.actions !== undefined ? (
        <div className="choices">
          {buttons.map((b) => {
            // The name is exactly what is on the button (its words and its key), so "Click Allow once" works.
            const name = props.keys === false ? b.label : `${b.label} ${String(b.key)}`;
            const decision = props.decisionOf?.(b.choice);
            const item = b.choice === "always" ? "always" : (decision ?? b.choice);
            const label = (
              <>
                {b.label}
                {props.keys === false ? null : (
                  <>
                    {" "}
                    <kbd>{String(b.key)}</kbd>
                  </>
                )}
              </>
            );
            // The second click of a double click answers nothing: the first one may have closed the card
            // that was under the pointer, and this one would land on the next card moved into its place.
            const onClick = (event: MouseEvent) => {
              if (event.detail > 1) return;
              choose(b.choice);
            };
            return b.choice === "no" ? (
              <Button
                key={b.choice}
                type="button"
                data-testid="transcript.approval-card-choose"
                data-item-id={item}
                data-choice={b.choice}
                data-decision={decision}
                disabled={disabled}
                aria-label={name}
                title={name}
                onClick={onClick}
                variant="danger"
                shape="outlined"
              >
                {label}
              </Button>
            ) : (
              <Button
                key={b.choice}
                type="button"
                data-testid="transcript.approval-card-choose"
                data-item-id={item}
                data-choice={b.choice}
                data-decision={decision}
                disabled={disabled}
                aria-label={name}
                title={name}
                onClick={onClick}
                appearance="choice"
                variant={b.choice === "once" ? "primary" : undefined}
              >
                {label}
              </Button>
            );
          })}
          {props.actions}
        </div>
      ) : null}
      <p className="card-state" role={props.stateRole ?? "status"}>
        {props.state}
      </p>
    </div>
  );
}
