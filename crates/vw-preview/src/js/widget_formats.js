// Read an attached Bootstrap datepicker's declared format for isolated browser actions.
// Installed with the page environment in every frame; transfers only untrusted format data through the DOM.
// It performs no editing, evaluation, network access or host calls, and never supplies an action result.
(() => {
  const name = "__vw_widget_date_format";
  if (Object.prototype.hasOwnProperty.call(window, name)) return;
  Object.defineProperty(window, name, { value: true });
  document.addEventListener("vw-widget-date-format", (event) => {
    const el = event.composedPath()[0] || event.target;
    if (el?.localName !== "input" || el.type !== "text") return;
    try {
      const widget = window.jQuery?.(el)?.data?.("datepicker");
      if (!widget) return;
      const format = widget.o?.format;
      if (format === undefined || format === null) return;
      let text = typeof format === "string" ? format : "";
      if (Array.isArray(format?.parts) && Array.isArray(format?.separators) &&
          format.parts.length === 3 && format.separators.length === 4) {
        text = format.separators[0] + format.parts.map((part, i) => part + format.separators[i + 1]).join("");
      }
      el.setAttribute("data-vw-widget-date-format", String(text).slice(0, 64));
    } catch {
      // A page may replace its widget library. Do not turn that exception into a successful format read.
    }
  }, true);
})();
