// HTML drag-and-drop between two observed refs. Inserted inside act.js; uses its checks and result helpers.
// Never reads files or field values; only the page's dragstart handler populates the DataTransfer.
/* global REQ, actionRef, checks, fire, point, result, viewOf -- act.js prelude. */
/* exported dragTo */
const dragTo = (source, target, hitPoint) => {
  const found = actionRef(REQ.to);
  if (found.found.length === 0) return result("stale");
  if (!found.target) return result("ambiguous");
  const destination = found.target;
  if (source.ownerDocument !== destination.ownerDocument)
    return result("error", { message: "drag between documents is unavailable" });
  const checked = checks(destination, "drag");
  if (checked.failed.length > 0) return result("notActionable", { ...checked, target });
  const transfer = new (viewOf(source).DataTransfer)();
  const event = (element, type, position) => fire(element, type, "DragEvent", {
    ...point(element, position), dataTransfer: transfer,
  });
  if (!event(source, "dragstart", hitPoint))
    return result("unmet", { target });
  event(destination, "dragenter", checked.hitPoint);
  const accepted = !event(destination, "dragover", checked.hitPoint);
  if (accepted) event(destination, "drop", checked.hitPoint);
  else event(destination, "dragleave", checked.hitPoint);
  event(source, "dragend", hitPoint);
  return result(accepted ? "done" : "unmet", { target });
};
