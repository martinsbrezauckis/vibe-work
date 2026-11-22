/**
 * What the person said, matched to one question of an agent's question card (voice chat answers questions by voice).
 * Depends on: transcript/model/callbacks (`Question`, `Picked`). Called by: state/voiceQuestion, tests.
 * Invariants: pure; never a guess. An option is chosen only by its whole label, its number ("two", "2", "the second
 * one", "option 2"), unique close one-word label, or words that belong to that option alone ("the blue one" when
 * only one label has "blue").
 * Several options (multi-select only) are said together with "and" or commas, and every part must be one option.
 * Words that touch no option are the person's own answer where the card allows one. Anything else (two options
 * fit, a part fits none, own words not allowed) is unclear: the caller asks again, then leaves the card.
 * Command words and number words are English; labels match in any language.
 */
import type { Picked, Question } from "../transcript/model/callbacks";

/** The meaning of one utterance for the question being asked. */
export type Heard =
  { kind: "pick"; picked: Picked } | { kind: "skip" } | { kind: "repeat" } | { kind: "unclear" };

const SKIP = new Set([
  "skip",
  "cancel",
  "skip it",
  "cancel it",
  "skip this",
  "skip that",
  "skip all",
  "skip them",
  "skip question",
  "skip questions",
  "skip the question",
  "skip the questions",
  "skip this question",
  "cancel the question",
  "cancel the questions",
]);
const REPEAT = new Set([
  "repeat",
  "repeat that",
  "repeat it",
  "repeat the question",
  "repeat the options",
  "again",
  "say again",
  "say it again",
  "say that again",
  "read it again",
  "what are the options",
  "what were the options",
]);
/** Words that carry no choice: "I'd like the blue one, please" is "blue". */
const FILLER = new Set([
  "the",
  "a",
  "an",
  "option",
  "number",
  "choice",
  "answer",
  "please",
  "i",
  "id",
  "ill",
  "im",
  "want",
  "would",
  "like",
  "choose",
  "pick",
  "select",
  "take",
  "go",
  "with",
  "lets",
  "it",
  "its",
  "is",
  "that",
  "this",
  "my",
  "say",
  "um",
  "uh",
  "er",
  "one",
  "and",
  "or",
  "of",
  "to",
  "in",
  "for",
  "on",
]);
const CARDINAL = ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];
const ORDINAL = [
  "first",
  "second",
  "third",
  "fourth",
  "fifth",
  "sixth",
  "seventh",
  "eighth",
  "ninth",
  "tenth",
];

/** Words that make a short utterance talk about the card rather than an answer ("go back", "not the second one"). */
const CONTROL = new Set([
  "not",
  "no",
  "dont",
  "never",
  "back",
  "wait",
  "stop",
  "skip",
  "cancel",
  "repeat",
  "again",
  "mind",
]);
/** The longest utterance in which a control word means "not an answer". */
const CONTROL_WORDS = 4;
/** Words spoken before a command that do not change it: "uh, skip", "okay repeat". */
const LEAD = new Set(["please", "um", "uh", "er", "okay", "ok", "so", "oh"]);

/** True when `w` is a number word or digits ("two", "second", "2", "2nd"). */
function isNumber(w: string): boolean {
  return (
    /^\d+(?:st|nd|rd|th)?$/.test(w) ||
    CARDINAL.includes(w) ||
    ORDINAL.includes(w) ||
    ["won", "to", "too", "for", "fore"].includes(w)
  );
}

/** Lower-case words of `text` (letters and digits of any script; apostrophes joined: "I'd" is "id"). */
export function words(text: string): string[] {
  return text
    .normalize("NFKC")
    .toLowerCase()
    .replace(/['’]/g, "")
    .split(/[^\p{L}\p{N}]+/u)
    .filter((w) => w !== "");
}

/** The 1-based option number said ("two", "2", "2nd", "second", "the second one", "option two"), or `null`. */
function numberSaid(said: readonly string[]): number | null {
  // A bare number can be transcribed as an English homophone. Longer phrases (especially "not to") stay unclear.
  if (said.length === 1) {
    const word = said[0];
    if (word === "won") return 1;
    if (word === "to" || word === "too") return 2;
    if (word === "for" || word === "fore") return 4;
  }
  const kept = said.filter((w) => w === "one" || !FILLER.has(w));
  const rest =
    kept.length === 2 && kept[1] === "one" && ORDINAL.includes(kept[0] ?? "")
      ? kept.slice(0, 1)
      : kept;
  if (rest.length !== 1) return null;
  const w = rest[0] ?? "";
  const digits = /^(\d{1,2})(?:st|nd|rd|th)?$/.exec(w);
  if (digits !== null) return Number(digits[1]);
  const cardinal = CARDINAL.indexOf(w);
  if (cardinal >= 0) return cardinal + 1;
  const ordinal = ORDINAL.indexOf(w);
  return ordinal >= 0 ? ordinal + 1 : null;
}

/** A short English label may be transcribed as a nearby word ("Blue" -> "Blow", "Red" -> "Read"). */
function closeWord(a: string, b: string): boolean {
  // Other scripts still get exact matching; ASCII here keeps the edit count independent of grapheme boundaries.
  if (!/^[a-z]{3,12}$/.test(a) || !/^[a-z]{3,12}$/.test(b)) return false;
  const limit = Math.min(a.length, b.length) === 3 ? 1 : 2;
  if (a[0] !== b[0] || a[1] !== b[1]) return false;
  let previous = Array.from({ length: b.length + 1 }, (_, i) => i);
  for (let i = 1; i <= a.length; i++) {
    const current = [i];
    for (let j = 1; j <= b.length; j++) {
      current[j] = Math.min(
        (previous[j] as number) + 1,
        (current[j - 1] as number) + 1,
        (previous[j - 1] as number) + (a[i - 1] === b[j - 1] ? 0 : 1),
      );
    }
    previous = current;
  }
  return (previous[b.length] as number) <= limit;
}

/** One option for one spoken part: its index, "none" (touches no option) or "unclear". */
function optionFor(question: Question, said: readonly string[]): number | "none" | "unclear" {
  const labels = question.options.map((o) => words(o.label));
  const joined = said.join(" ");
  const exactMatches = labels.flatMap((l, i) => (l.join(" ") === joined ? [i] : []));
  if (exactMatches.length > 1) return "unclear";
  const exact = exactMatches[0] ?? -1;
  const n = numberSaid(said);
  const close =
    said.length === 1
      ? labels.flatMap((l, i) =>
          l.length === 1 && closeWord(said[0] as string, l[0] as string) ? [i] : [],
        )
      : [];
  const wordMatches =
    said.length === 1 ? labels.flatMap((l, i) => (l.includes(said[0] as string) ? [i] : [])) : [];
  // A close whole label can compete with an exact word inside another label.
  if (new Set([...close, ...wordMatches]).size > 1) return "unclear";
  // Free text only: a number is the answer itself ("four").
  if (n !== null && labels.length === 0) return "none";
  // Number and label candidates must agree; an out-of-range number cannot be repaired into a label.
  if (n !== null && (n < 1 || n > labels.length || (exact >= 0 && exact !== n - 1)))
    return "unclear";
  if (n !== null && close.some((i) => i !== n - 1)) return "unclear";
  // A number that is also a word of a label ("2" beside "Version 2") could mean either.
  if (n !== null && exact < 0 && labels.some((l) => said.some((w) => isNumber(w) && l.includes(w))))
    return "unclear";
  if (n !== null) return n - 1;
  if (exact >= 0) return exact;
  // A command or negation may resemble a label, but is not repaired into one.
  if (said.length <= CONTROL_WORDS && said.some((w) => CONTROL.has(w))) return "unclear";
  if (close.length === 1) return close[0] as number;
  const meaningful = said.filter((w) => !FILLER.has(w));
  if (meaningful.length === 0) return "unclear";
  // Numbers or card talk beside options ("one or two", "not the second one", "go back") are not own words.
  if (labels.length > 0 && meaningful.some(isNumber)) return "unclear";
  // Every meaningful word inside one label only: "let's go with dark mode", "the blue one" for "Blue sky" beside
  // "Red sea". A sentence with other words too ("call it Rex the dog") is not a choice of "dog".
  const inside = labels.flatMap((l, i) => (meaningful.every((w) => l.includes(w)) ? [i] : []));
  if (inside.length === 1) return inside[0] as number;
  const touches = labels.some((l) => meaningful.some((w) => l.includes(w)));
  return touches ? "unclear" : "none";
}

/** Matches `text` (the transcript of one spoken answer) to `question`. */
export function hearAnswer(question: Question, text: string): Heard {
  const said = words(text);
  if (said.length === 0) return { kind: "unclear" };
  const phrase = said.filter((w) => !LEAD.has(w)).join(" ");
  const labelSaid = question.options.some((o) => words(o.label).join(" ") === phrase);
  // "Skip" that is also an option's whole label could mean either: the number decides.
  if (SKIP.has(phrase)) return labelSaid ? { kind: "unclear" } : { kind: "skip" };
  if (REPEAT.has(phrase) && !labelSaid) return { kind: "repeat" };
  if (!labelSaid && said.length <= 5 && (said.includes("repeat") || phrase.endsWith("again")))
    return { kind: "repeat" };
  const own = text.trim().replace(/\s+/g, " ");
  const single = optionFor(question, said);
  if (typeof single === "number")
    return { kind: "pick", picked: { labels: [question.options[single]?.label ?? ""], other: "" } };
  if (question.multiSelect) {
    const parts = text
      .split(/,|;|&|\band\b|\bplus\b/iu)
      .map(words)
      .filter((p) => p.length > 0);
    const found = parts.map((p) => optionFor(question, p));
    if (parts.length > 1 && found.every((f) => typeof f === "number")) {
      const chosen = [...new Set(found)].sort((a, b) => a - b);
      return {
        kind: "pick",
        picked: { labels: chosen.map((i) => question.options[i]?.label ?? ""), other: "" },
      };
    }
    if (found.some((f) => f !== "none")) return { kind: "unclear" };
  }
  // A one- or two-character STT result beside choices is too little evidence for an Other answer.
  if (single === "none" && question.options.length > 0 && words(own).join("").length < 3)
    return { kind: "unclear" };
  if (single === "none" && question.allowOther && own !== "")
    return { kind: "pick", picked: { labels: [], other: own } };
  return { kind: "unclear" };
}
