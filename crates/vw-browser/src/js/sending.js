// Detect form activation before input. Scripted clicks/keys and final native point validation share this rule.
// Follow native label associations and composed ancestry; an unreadable activation needs send approval.
/* exported postsForm */
function postsForm(element, enter = false) {
  try {
    if (!element) return true;
    const name = node => (node.localName ?? node.tagName).toLowerCase();
    let control = element.nodeType === 3 ? element.parentElement : element;
    for (let depth = 0; control && !["input", "button", "label"].includes(name(control)); depth += 1) {
      if (depth >= 256) return true;
      control = control.assignedSlot ?? control.parentElement ?? control.getRootNode().host ?? null;
    }
    if (control && name(control) === "label") {
      control = control.control;
      if (control === undefined) return true;
      // Enter/Space activates the label by click; it does not send Enter into its associated text field.
      enter = false;
    }
    if (!control || !["input", "button"].includes(name(control))) return false;
    if (typeof control.type !== "string") return true;
    const submits = node => ["input", "button"].includes(name(node)) && ["submit", "image"].includes(node.type);
    if (!enter && !submits(control)) return false;
    if (enter && ["button", "reset", "checkbox", "radio", "file", "hidden"].includes(control.type)) return false;
    const form = control.form;
    if (form === null) return false;
    if (!form) return true;
    // Named fields can shadow form.method and form.elements. Read the native properties, as the browser does.
    const native = property => Object.getOwnPropertyDescriptor(HTMLFormElement.prototype, property).get.call(form);
    let submitter = control;
    if (enter && !submits(control)) {
      const associated = [...native("elements")];
      for (const image of form.getRootNode().querySelectorAll("input"))
        if (image.type === "image" && image.form === form) associated.push(image);
      submitter = associated.filter(node => submits(node) && node.form === form)
        .sort((a, b) => a === b ? 0 : a.compareDocumentPosition(b) & 2 ? 1 : -1)[0];
    }
    const override = submitter?.getAttribute("formmethod");
    const method = override === null || override === undefined ? native("method") : submitter.formMethod;
    // Scripted Enter on a field uses requestSubmit() without a submitter, so its form method also applies.
    return !["get", "dialog"].includes(method.toLowerCase()) ||
      enter && !submits(control) && !["get", "dialog"].includes(native("method").toLowerCase());
  } catch {
    return true;
  }
}
