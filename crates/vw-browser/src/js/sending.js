// Detect form activation before input. Scripted clicks/keys and final native point validation share this rule.
// Follow native label associations and composed ancestry; an unreadable activation needs send approval.
/* exported postsForm, implicitSubmission, submitWithOutcome */
// Observe the browser's actual validation/submission events without pre-validating or bypassing handlers.
function submitWithOutcome({form, submitter}) {
  let invalid = false, submitted = null;
  const rejected = event => { if (event.target?.form === form) invalid = true; };
  const sent = event => { if (event.target === form) submitted = event; };
  const win = form.ownerDocument.defaultView;
  win.addEventListener("invalid", rejected, true);
  win.addEventListener("submit", sent, true);
  try {
    const requestSubmit = win.HTMLFormElement.prototype.requestSubmit;
    if (submitter) requestSubmit.call(form, submitter);
    else requestSubmit.call(form);
  } finally {
    win.removeEventListener("invalid", rejected, true);
    win.removeEventListener("submit", sent, true);
  }
  if (invalid) return "Native validation blocked submission. Read the invalid field messages and correct the fields before submitting again.";
  if (submitted?.defaultPrevented) return "The page handled and canceled the native submit event. Read the resulting page state to confirm its outcome.";
  if (submitted) return "The native submit event ran. Read the resulting page to confirm the submission outcome.";
  return "Submission was requested, but no validation or submit event was observed. Read the page before trying again.";
}
// The HTML implicit-submission fields also block a buttonless form when more than one is associated.
// Return the exact form/submitter for both classification and dispatch; unreadable state throws to fail closed.
function implicitSubmission(control) {
  const textField = node => {
    if (node.localName !== "input") return false;
    if (typeof node.type !== "string") throw new Error("unreadable input type");
    return ["text", "search", "tel", "url", "email", "password", "date", "month", "week", "time",
      "datetime-local", "number"].includes(node.type);
  };
  if (!textField(control)) return null;
  const form = control.form;
  if (form === null) return null;
  const associated = [...Object.getOwnPropertyDescriptor(HTMLFormElement.prototype, "elements").get.call(form)];
  // form.elements omits image submitters; external associated buttons still participate in tree order.
  for (const image of form.getRootNode().querySelectorAll("input"))
    if (image.type === "image" && image.form === form) associated.push(image);
  const submitter = associated.filter(node => ["input", "button"].includes(node.localName) &&
    ["submit", "image"].includes(node.type) && node.form === form)
    .sort((a, b) => a === b ? 0 : a.compareDocumentPosition(b) & 2 ? 1 : -1)[0];
  // A disabled default button blocks submission, including a button disabled by its fieldset.
  if (submitter ? submitter.matches(":disabled") : associated.filter(textField).length > 1) return null;
  return { form, submitter };
}

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
    let form = control.form;
    let submitter = control;
    if (!submits(control)) {
      if (!enter) return false;
      const submission = implicitSubmission(control);
      if (!submission) return false;
      ({ form, submitter } = submission);
    }
    if (form === null) return false;
    if (!form) return true;
    // Named fields can shadow form.method and form.elements. Read the native properties, as the browser does.
    const native = property => Object.getOwnPropertyDescriptor(HTMLFormElement.prototype, property).get.call(form);
    const override = submitter?.getAttribute("formmethod");
    const method = override === null || override === undefined ? native("method") : submitter.formMethod;
    return !["get", "dialog"].includes(method.toLowerCase());
  } catch {
    return true;
  }
}
