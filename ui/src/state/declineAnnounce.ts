/**
 * A person's No on a card and the turn it ended, said to screen readers as one line. Both arrive within moments of
 * each other, in either order, while the focus moves from the card to the composer, and a screen reader that reads
 * the newly focused composer can drop one of two separate lines (Windows Narrator heard only the card's "Declined
 * writing … by you", never "Declined. The turn ended."). So the second of the two says the card's line with "The
 * turn ended." added: the last line said carries both. A decline whose turn goes on (Vibe Work's own asks) stays the
 * card's line alone; a turn that ended at a No nobody answered on a card here says "Declined. The turn ended.".
 * Depends on: nothing. Called by: the card close (features/ask/useCardClose) and the turn-end announcements
 * (features/transcript/replyAnnouncements); both then pass the text to the app's announcer.
 */

/** How close (ms) a card's decline and its chat's declined turn end must be to be said as one line. */
export const PAIR_MS = 3000;
/** The turn-level line when no card decline of this window pairs with it. */
export const TURN_DECLINED = "Declined. The turn ended.";
const ENDED = "The turn ended.";

const cards = new Map<string, { at: number; text: string }>();
const turns = new Map<string, number>();

function joined(text: string): string {
  return `${text.replace(/[.\s]+$/u, "")}. ${ENDED}`;
}

/** The line to say for a card the person declined in `chat` (its audit text `text`). */
export function cardDeclinedLine(chat: string, text: string, now = performance.now()): string {
  const ended = turns.get(chat);
  if (ended !== undefined && now - ended <= PAIR_MS) {
    turns.delete(chat);
    cards.delete(chat);
    return joined(text);
  }
  cards.set(chat, { at: now, text });
  return text;
}

/** The line to say when `chat`'s turn ended at the person's No. */
export function turnDeclinedLine(chat: string, now = performance.now()): string {
  const card = cards.get(chat);
  if (card !== undefined && now - card.at <= PAIR_MS) {
    cards.delete(chat);
    turns.delete(chat);
    return joined(card.text);
  }
  turns.set(chat, now);
  return TURN_DECLINED;
}

/** Tests: forget every pending half. */
export function resetDeclineLines(): void {
  cards.clear();
  turns.clear();
}
