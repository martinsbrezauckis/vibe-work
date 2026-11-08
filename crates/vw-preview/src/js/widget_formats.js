// Attached datepicker formats and calendar close in the widget's own JavaScript world.
// Called through DOM events by isolated browser actions; page data never supplies an action receipt.
(() => {
  const name = "__vw_widget_date_format";
  if (Object.prototype.hasOwnProperty.call(window, name)) return;
  Object.defineProperty(window, name, { value: true });
  const attached = (el) => {
    const jq = window.jQuery;
    if (typeof jq !== "function") return null;
    // Bootstrap components store the instance on the wrapper, with inputField identifying its input.
    for (let node = el; node?.nodeType === 1; node = node.parentElement) {
      const widget = jq(node)?.data?.("datepicker");
      if (widget?.o?.format != null && (node === el || widget.inputField?.[0] === el)) {
        let format = widget.o.format;
        if (Array.isArray(format?.parts) && Array.isArray(format?.separators) &&
            format.parts.length === 3 && format.separators.length === 4) {
          format = format.separators[0] + format.parts.map((part, i) => part + format.separators[i + 1]).join("");
        }
        return { format: typeof format === "string" ? format : "", hide: () => widget.hide?.() };
      }
    }
    // jQuery UI keeps defaults outside the input's settings and shares one popup across inputs.
    const widget = jq(el)?.data?.("datepicker");
    if (jq.datepicker && widget?.input?.[0] === el && typeof jq(el)?.datepicker === "function") {
      const format = jq(el).datepicker("option", "dateFormat");
      return {
        format: typeof format === "string" ? format.replace(/yy/g, "yyyy") : "",
        hide: () => jq(el).datepicker("hide"),
      };
    }
    return null;
  };
  document.addEventListener("vw-widget-date-format", (event) => {
    const el = event.composedPath()[0] || event.target;
    if (el?.localName !== "input" || el.type !== "text") return;
    try {
      const widget = attached(el);
      if (widget) el.setAttribute("data-vw-widget-date-format", widget.format.slice(0, 64));
    } catch {
      // A replaced library cannot turn a failed format read into a successful edit.
    }
  }, true);
  document.addEventListener("vw-widget-date-commit", (event) => {
    const el = event.composedPath()[0] || event.target;
    if (el?.localName !== "input" || el.type !== "text") return;
    try {
      const widget = attached(el);
      if (!widget) return;
      // The type action commits only after the page has seen the field's focus, so no later focus reopens it.
      widget.hide();
    } catch {
      // The isolated world still verifies the actual field value.
    }
  }, true);
})();
