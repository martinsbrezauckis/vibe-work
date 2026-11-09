/** Renders permission and question cards with their offered answers. Depends on shared controls and card focus; called by the ask adapters. */
import {
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
  type ReactElement,
  type ReactNode,
  useEffect,
  useId,
  useRef,
} from "react";
import { Button } from "../../components/Controls";
import { useFirstSight } from "../../hooks/useFirstSight";
import type { CoversPart } from "../../protocol/core";
import { TEXT, wording } from "../../vocabulary";
import { visibleText } from "./visibleText";
import { registerCardFocus } from "./cardFocus";
import { usePermissionArrival } from "./permissionArrival";
import { type AskButton, type AskChoice } from "./model";
import { focusCard, jumpToCard, revealCardControl, useCardLayout } from "./useCardLayout";

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
  /**
   * Controls that must stay on screen while the payload scrolls (a protected change's "Show all {n} files"): shown
   * between the scrolling body and the answers, never inside the body (voice control presses only what is shown).
   */
  footer?: ReactNode;
  /** Buttons other than the answer set (a question's Submit / Skip). */
  actions?: ReactNode;
  /** Keys 1-4 answer (not for a question). */
  keys?: boolean;
  /**
   * A question's own keys (number keys pick options, Enter submits): text fields forward only Enter.
   * Includes whether the arrival grace has passed. The card's own answer keys are then off.
   */
  onKeys?: (event: KeyboardEvent<HTMLDivElement>, ready: boolean) => void;
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

/** Frames the arrival focus is retried for while the card's row is not shown yet (it is shown within one or two). */
const ARRIVAL_FOCUS_FRAMES = 30;

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
      {parts.map((part, i) =>
        part.code ? <code key={i}>{visibleText(part.text)}</code> : visibleText(part.text),
      )}
      .
    </span>
  );
}

/** Renders one ask. */
export function AskCard(props: AskCardProps): ReactElement {
  return (
    <PermissionCard
      key={props.id}
      {...props}
      question={visibleText(props.question)}
      who={visibleText(props.who)}
      {...(props.whoTitle === undefined ? {} : { whoTitle: visibleText(props.whoTitle) })}
      {...(props.summary === undefined ? {} : { summary: visibleText(props.summary) })}
      reason={props.reason == null ? null : visibleText(props.reason)}
      state={visibleText(props.state)}
    />
  );
}

function PermissionCard(props: AskCardProps): ReactElement {
  const { buttons, disabled, onChoose } = props;
  const arrival = usePermissionArrival(props.keys !== false);
  const fresh = useFirstSight(`card:${props.id}`);
  const frame = useRef<HTMLDivElement>(null);
  // The answer button a primary pointer is pressed on (cleared when the pointer leaves it or is cancelled): the
  // click that ends that press is a pointer click even when its `detail` is 0, as the Debug API's ui/click is
  // (a pointer press and release, then the element's own click()). Keyboard activations never press a pointer.
  const pressed = useRef<AskChoice | null>(null);
  const maxHeight = useCardLayout(frame);
  // Each answer is described by the card's question and summary: a screen reader moving straight to a button
  // does not announce the surrounding group's name, so the question would never be spoken.
  const textId = useId();
  const describedBy = props.summary === undefined ? `${textId}-q` : `${textId}-q ${textId}-s`;
  useEffect(() => {
    if (!arrival.visible || !fresh || disabled) return;
    // The transcript's virtual list shows a new row only once it has measured it (`visibility: hidden` until then,
    // a frame or two), and a hidden element refuses focus: retry on the next frames until the focus lands, unless
    // the user starts typing or moves the focus meanwhile.
    const before = document.activeElement;
    let left = ARRIVAL_FOCUS_FRAMES;
    let pending = 0;
    const attempt = () => {
      const card = frame.current;
      if (card === null || typing() || inAnotherCard(card)) return;
      if (left < ARRIVAL_FOCUS_FRAMES && document.activeElement !== before) return;
      focusCard(card, "newest");
      if (!card.contains(document.activeElement) && left-- > 0)
        pending = requestAnimationFrame(attempt);
    };
    attempt();
    return () => {
      cancelAnimationFrame(pending);
    };
  }, [arrival.visible, fresh, disabled]);
  // "Go to the waiting prompt" asks for this card by id.
  useEffect(
    () =>
      !arrival.visible
        ? undefined
        : registerCardFocus(props.id, () => {
            jumpToCard(frame.current);
          }),
    [props.id, arrival.visible],
  );
  const choose = (choice: AskChoice) => {
    if (disabled || onChoose === undefined) return;
    // macOS can activate a button without focusing it. Every accepted local answer returns to Message.
    props.onFocusInside?.(true);
    onChoose(choice);
  };
  const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
    if (props.onKeys !== undefined) {
      const target = event.target;
      const typed =
        (target instanceof HTMLInputElement &&
          target.type !== "radio" &&
          target.type !== "checkbox") ||
        target instanceof HTMLTextAreaElement;
      if (
        (!typed || event.key === "Enter") &&
        !event.nativeEvent.isComposing &&
        // eslint-disable-next-line @typescript-eslint/no-deprecated -- WebKit's IME confirmation Enter still uses 229.
        event.nativeEvent.keyCode !== 229 &&
        !event.altKey &&
        !event.ctrlKey &&
        !event.metaKey
      )
        props.onKeys(event, arrival.keysReady() && !event.repeat);
      return;
    }
    // A held key repeats: only its first press answers (the next card may already have the focus).
    if (props.keys === false) return;
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
    if (!arrival.keysReady() || event.repeat) {
      if (button !== undefined || event.key === "Enter" || event.key === " ")
        event.preventDefault();
      return;
    }
    if (button !== undefined) {
      event.preventDefault();
      choose(button.choice);
    }
  };
  return (
    <div
      ref={frame}
      hidden={!arrival.visible}
      style={
        {
          "--ask-max-height": maxHeight === undefined ? undefined : `${String(maxHeight)}px`,
        } as CSSProperties
      }
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
        // Sending disables buttons and option inputs (including through a fieldset). WebKit blurs them
        // without a new target; retain ownership so every answer path returns focus to the composer.
        if (
          next === null &&
          event.target instanceof HTMLElement &&
          event.target.matches(":disabled")
        )
          return;
        props.onFocusInside?.(next instanceof Node && event.currentTarget.contains(next));
      }}
      {...props.data}
    >
      <div className="ah">
        <span className="px needs" aria-hidden="true" />
        <b id={`${textId}-q`}>{props.question}</b>
        <span className="sub" title={props.whoTitle ?? props.who}>
          {props.who}
        </span>
      </div>
      <div className="ask-body">
        {props.summary !== undefined || props.reason || props.covers ? (
          <div className="why">
            {props.summary === undefined ? null : (
              <span className="summary" id={`${textId}-s`}>
                {props.summary}
              </span>
            )}
            {props.reason ? <span className="reason">{props.reason}</span> : null}
            {props.covers && props.covers.length > 0 ? <CoversLine parts={props.covers} /> : null}
          </div>
        ) : null}
        {props.children}
      </div>
      {props.footer === undefined || props.footer === null ? null : (
        <div className="ask-foot">{props.footer}</div>
      )}
      {buttons.length > 0 || props.actions !== undefined ? (
        <div className="choices">
          {buttons.map((b) => {
            // Named by its words only: voice control ("Click Allow once", macOS Voice Control) needs the whole name,
            // and a key digit in it stops the spoken words from matching. The key is its keyboard shortcut.
            const name = b.label;
            const shortcut = props.keys === false ? undefined : String(b.key);
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
              const pointer = event.detail > 0 || pressed.current === b.choice;
              pressed.current = null;
              if (event.detail > 1) return;
              // A keyboard click (Enter or Space on the button) waits for the arrival grace like the answer keys.
              if (!pointer && !arrival.keysReady()) return;
              choose(b.choice);
            };
            const press = {
              onPointerDown: (event: PointerEvent) => {
                pressed.current = event.isPrimary && event.button === 0 ? b.choice : null;
              },
              onPointerLeave: () => {
                pressed.current = null;
              },
              onPointerCancel: () => {
                pressed.current = null;
              },
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
                aria-keyshortcuts={shortcut}
                aria-describedby={describedBy}
                title={name}
                onClick={onClick}
                {...press}
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
                aria-keyshortcuts={shortcut}
                aria-describedby={describedBy}
                title={name}
                onClick={onClick}
                {...press}
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
