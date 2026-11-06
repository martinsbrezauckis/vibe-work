// Native value validation and range key defaults for the built-in action script.
// Uses detached controls before live mutation; called by act.js with the native setter and event dispatcher.
/* global fire, setValue, viewOf, widgetDateValue */
/* exported requestedValue, rangeKey */
const requestedValue = (el, text) => {
  const widget = widgetDateValue(el, text);
  if (widget) return widget;
  if (el.localName !== "input" || !["range", "color", "date", "time", "datetime-local", "month", "week"].includes(el.type))
    return { value: text };
  const probe = el.cloneNode(false);
  setValue(probe, text);
  const value = probe.value;
  let valid = value === text;
  if (el.type === "color") valid = /^#[0-9a-f]{6}$/i.test(text) && value === text.toLowerCase();
  if (el.type === "range") valid = text.trim() !== "" && Number.isFinite(Number(text)) && Number(value) === Number(text);
  if (["time", "datetime-local"].includes(el.type) && value && !valid) {
    // Native controls remove zero seconds/fractions when serializing an otherwise valid time.
    const normalize = (s) => s.replace(/(\d{2}:\d{2}):00(?:\.0+)?$/, "$1");
    valid = normalize(value) === normalize(text);
  }
  return valid ? { value } : { status: "invalidValue", kind: el.type };
};

const rangeKey = (el, key) => {
  if (el.localName !== "input" || el.type !== "range" ||
      !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"].includes(key)) return null;
  const before = el.value;
  const number = (text, fallback) => text.trim() !== "" && Number.isFinite(Number(text)) ? Number(text) : fallback;
  const min = number(el.min, 0), max = Math.max(min, number(el.max, 100));
  const step = el.step === "any" ? (max - min) / 100 : Math.max(0, number(el.step, 1)) || 1;
  const direction = viewOf(el).getComputedStyle(el).direction === "rtl" ? -1 : 1;
  const up = key === "ArrowUp" || key === "PageUp" || (key === "ArrowRight" && direction === 1) || (key === "ArrowLeft" && direction === -1);
  const delta = key.startsWith("Page") ? Math.max(step, (max - min) / 10) : step;
  setValue(el, String(key === "Home" ? min : key === "End" ? max : Math.max(min, Math.min(max, Number(before) + (up ? delta : -delta)))));
  if (el.value !== before) {
    fire(el, "input", "Event", { bubbles: true });
    fire(el, "change", "Event", { bubbles: true });
  }
  return el.value === before ? "the range is already at its limit" : "changed the range value";
};
