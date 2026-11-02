/** Fits the waiting card above the composer; CSS reserves body reading space and lets long payloads scroll.
 * Depends on the transcript viewport and ResizeObserver. Called by AskCard; directed focus reveals the whole card.
 */
import { useLayoutEffect, useState, type RefObject } from "react";

/** Constrains a card to the transcript's current free height, including its outer spacing. */
export function useCardLayout(frame: RefObject<HTMLDivElement | null>): number | undefined {
  const [height, setHeight] = useState<number>();
  useLayoutEffect(() => {
    const card = frame.current;
    const viewport = card?.closest<HTMLElement>(".tx-list");
    if (!card || !viewport) return;
    const fit = () => {
      const style = getComputedStyle(card);
      const spacing = (parseFloat(style.marginTop) || 0) + (parseFloat(style.marginBottom) || 0);
      if (viewport.clientHeight > 0) setHeight(Math.max(0, viewport.clientHeight - spacing));
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(viewport);
    return () => {
      observer.disconnect();
    };
  }, [frame]);
  return height;
}

/** Focuses the first answer; arrival preserves the newest prompt's view, directed navigation reveals its own card. */
export function focusCard(
  card: HTMLDivElement | null,
  reveal: "focused" | "newest" = "focused",
): void {
  if (!card) return;
  // A question's first option (its picked one, if any) comes before Submit and Skip: focusing never answers.
  (
    card.querySelector<HTMLElement>(".q-opts input:checked:not(:disabled)") ??
    card.querySelector<HTMLElement>(".q-opts input:not(:disabled)") ??
    card.querySelector<HTMLElement>(".choices button:not(:disabled)") ??
    card
  ).focus({
    preventScroll: true,
  });
  // Several cards can finish measuring together. An older card's delayed arrival focus must not scroll the
  // newest question and answers away after the transcript has followed them. Focus ownership stays unchanged.
  const latest =
    reveal === "newest"
      ? [
          ...(card.closest(".tx-list")?.querySelectorAll<HTMLElement>(".approval:not([hidden])") ??
            []),
        ].at(-1)
      : undefined;
  (latest ?? card).scrollIntoView({ block: "nearest" });
}

/**
 * "Go to the waiting prompt": focuses and reveals the card, then ends a scroll still running in the transcript.
 * A keyboard scroll (Home, PageUp) animates, and an instant reveal does not stop that animation, so the list would
 * carry the card away afterwards. A smooth scroll to the revealed position replaces it without moving the list.
 */
export function jumpToCard(card: HTMLDivElement | null): void {
  focusCard(card);
  const list = card?.closest<HTMLElement>(".tx-list");
  list?.scrollTo({ top: list.scrollTop, behavior: "smooth" });
}

/** Reveals the focused answer inside each scroll frame. */
export function revealCardControl(target: HTMLElement): void {
  if (!target.matches("input,textarea,select,button")) return;
  target.scrollIntoView({
    block: "nearest",
    inline: "nearest",
  });
}
