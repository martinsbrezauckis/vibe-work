// Offline copies of Bootstrap component ownership and jQuery UI's default-format/singleton lifecycle.
// Called by widget_dates.rs in the page world; the production driver runs in a separate isolated world.
/* global window, document */
(() => {
  const input = document.getElementById("date");
  const label = input.closest("label");
  const form = document.createElement("form");
  label.replaceWith(form);
  form.append(label);
  const submit = document.createElement("button");
  submit.type = "submit";
  submit.textContent = "Submit";
  form.append(submit);
  const calendar = document.createElement("div");
  calendar.id = "calendar";
  calendar.textContent = "Calendar";
  const wrapper = document.createElement("div");
  input.replaceWith(wrapper);
  wrapper.append(input);
  window.hidden = 0;
  window.blurs = 0;
  window.submitted = 0;
  window.widgetDate = null;
  input.addEventListener("focus", () => document.body.append(calendar));
  input.addEventListener("blur", () => window.blurs++);
  document.addEventListener("submit", (event) => { event.preventDefault(); window.submitted++; });
  input.addEventListener("keyup", () => { window.widgetDate = input.value; });
  const hide = () => { window.hidden++; calendar.remove(); };
  const bootstrap = {
    o: { format: { parts: ["mm", "dd", "yyyy"], separators: ["", "/", "/", ""] } },
    inputField: [input],
    hide,
  };
  const ui = { input: [input], settings: {} };
  const jquery = (node) => ({
    data: (name) => name !== "datepicker" ? null : window.fixtureDateKind === "bootstrap"
      ? (node === wrapper ? bootstrap : null) : (node === input ? ui : null),
    datepicker: (command, option) => {
      if (node !== input) throw new Error("not attached");
      if (command === "option" && option === "dateFormat") return jquery.datepicker._defaults.dateFormat;
      if (command === "hide" && jquery.datepicker._curInst === ui) hide();
    },
  });
  if (window.fixtureDateKind === "jqueryUi") {
    jquery.datepicker = { _defaults: { dateFormat: "mm/dd/yy" }, _curInst: ui };
  }
  window.jQuery = jquery;
})();
