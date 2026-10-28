/**
 * The install offer block of the Tools tabs: heading, the loading line, the facts (`dl.set-facts`), the one sentence
 * of an offer that shows again after a refused click (updated / ran out, right above the actions) and, for a skill
 * row, the confirm and Cancel buttons. The marketplace detail shows the block right above its own Install button
 * instead, so there the sentence ends the block. A fresh offer looks like any offer: no per-fact marks.
 * Depends on: shared Controls, the offer model (`state/installOffers.ts`). Called by: SkillRow, MarketDetail.
 * Invariants: renders only; every decision (texts, facts, focus targets) comes from the offer model.
 */
import { type KeyboardEvent, type ReactElement, type RefObject, useEffect, useRef } from "react";

import { Button } from "../../components/Controls";
import {
  confirmLabel,
  noticeText,
  OFFER_READING,
  offerFacts,
  offerHeading,
  type OfferFocus,
  type OpenOffer,
} from "../../state/installOffers";

/**
 * Moves focus to `ref` when the offer model asks for `part` of `target` (each request once, by its serial).
 * @param focus the model's latest focus request.
 * @param target the skill name or marketplace entry id this element belongs to.
 * @param part which element this is.
 * @param ref the element to focus.
 */
export function useOfferFocus(
  focus: OfferFocus | null,
  target: string,
  part: OfferFocus["part"],
  ref: RefObject<HTMLElement | null>,
): void {
  useEffect(() => {
    if (focus?.target === target && focus.part === part) ref.current?.focus();
  }, [focus, target, part, ref]);
}

/** Escape closes the offer (focus goes back to Install…, the model decides). */
export function escapeCloses(onCancel: () => void) {
  return (event: KeyboardEvent): void => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    onCancel();
  };
}

/**
 * One offer block.
 * @param open the open offer.
 * @param focus the model's focus request (the notice and, with buttons, the confirm button take it).
 * @param onCancel Cancel and Escape.
 * @param onConfirm the confirm button; omit it where the page has its own Install button (Marketplace).
 * @param testId the block's test id (`skills.offer`, `marketplace.offer`).
 */
export function InstallOffer({
  open,
  focus,
  onCancel,
  onConfirm,
  testId,
}: {
  open: OpenOffer;
  focus: OfferFocus | null;
  onCancel: () => void;
  onConfirm?: () => void;
  testId: string;
}): ReactElement {
  const notice = useRef<HTMLParagraphElement>(null);
  const confirm = useRef<HTMLButtonElement>(null);
  useOfferFocus(focus, open.target, "notice", notice);
  // Without its own buttons (Marketplace) the ref stays empty and the page focuses its Install button.
  useOfferFocus(focus, open.target, "confirm", confirm);
  const facts = offerFacts(open);
  const busy = open.phase !== "shown";
  return (
    <div
      className="install-offer"
      data-testid={testId}
      data-offer-target={open.target}
      data-phase={open.phase}
      onKeyDown={escapeCloses(onCancel)}
    >
      <h4>{offerHeading(open.mode)}</h4>
      {open.phase === "reading" ? (
        <p className="key-line" role="status">
          {OFFER_READING}
        </p>
      ) : null}
      {facts.length === 0 || open.phase === "reading" ? null : (
        <dl className="set-facts">
          {facts.map((fact) => (
            <div className="check" key={fact.key} data-fact={fact.key}>
              <dt>{fact.label}</dt>
              <dd>{fact.value}</dd>
            </div>
          ))}
        </dl>
      )}
      {open.notice === null || open.phase === "reading" ? null : (
        <p
          className="server-note"
          role="alert"
          tabIndex={-1}
          ref={notice}
          data-testid={`${testId}-notice`}
        >
          {noticeText(open.notice, open.mode)}
        </p>
      )}
      {onConfirm === undefined || open.phase === "reading" ? null : (
        <div className="install-offer-actions">
          <Button
            data-testid={`${testId}-confirm`}
            ref={confirm}
            variant="primary"
            shape="outlined"
            disabled={busy}
            onClick={onConfirm}
          >
            {confirmLabel(open)}
          </Button>
          <Button
            data-testid={`${testId}-cancel`}
            shape="outlined"
            disabled={busy}
            onClick={onCancel}
          >
            Cancel
          </Button>
        </div>
      )}
    </div>
  );
}
