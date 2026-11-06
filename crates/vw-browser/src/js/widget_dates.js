// Date text for an attached numeric Bootstrap datepicker, using only its declared format.
// Called by form_values.js; the page environment supplies untrusted format data, never a success receipt.
/* global viewOf */
/* exported widgetDateValue */
const widgetDateValue = (el, text) => {
  if (el.localName !== "input" || el.type !== "text") return null;
  const attribute = "data-vw-widget-date-format";
  el.removeAttribute(attribute);
  el.dispatchEvent(new (viewOf(el).Event)("vw-widget-date-format", { bubbles: true, composed: true }));
  const format = el.getAttribute(attribute);
  el.removeAttribute(attribute);
  if (format === null) return null;
  const pieces = format.split(/(yyyy|mm|dd|m|d)/);
  const tokens = pieces.filter((_, i) => i % 2);
  const separators = pieces.filter((_, i) => !(i % 2));
  if (tokens.length !== 3 || new Set(tokens.map((t) => t[0])).size !== 3 ||
      separators[0] || separators[3] || !separators.slice(1, 3).every((s) => /^[./ -]{1,3}$/.test(s)))
    return /^\d{4}-\d{2}-\d{2}$/.test(text) ? { status: "unsupportedWidgetDate" } : { value: text, widgetDate: true };
  if (!text) return { value: "", widgetDate: true };
  const iso = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text);
  const pattern = pieces.map((piece, i) => i % 2 ? (piece === "yyyy" ? "(\\d{4})" : "(\\d{1,2})") : piece.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("");
  const local = !iso && new RegExp(`^${pattern}$`).exec(text);
  const date = iso ? { y: +iso[1], m: +iso[2], d: +iso[3] } : {};
  if (local) tokens.forEach((token, i) => { date[token[0]] = +local[i + 1]; });
  const leap = date.y % 4 === 0 && (date.y % 100 !== 0 || date.y % 400 === 0);
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  if (!(date.y >= 1 && date.m >= 1 && date.m <= 12 && date.d >= 1 && date.d <= days[date.m - 1]))
    return { status: "invalidWidgetDate", kind: format.toUpperCase() };
  const value = pieces.map((piece, i) => i % 2 ? String(date[piece[0]]).padStart(piece.length, "0") : piece).join("");
  return { value, widgetDate: true };
};
