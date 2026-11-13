// Bounded form label/context discovery for the shared page scripts. Depends on common.js helpers;
// used by observation, labelOf and form prioritization. Context never reads input values.
/* global REQ, clip, fieldLabelText, hideKeys, matches, secretKind, vaultFilled, visible -- Shared script helpers. */
/* exported formControl, nearbyLabel, controlSearchText, labelledToggle, labelledSubmit */
const labelledToggle = el => el.localName === "label" && el.control &&
  matches(el.control, 'input[type=checkbox],input[type=radio]') ? el.control : null;
// Native label activation also reaches a hidden submitter, without a role or pointer cursor on its label.
const labelledSubmit = el => el.localName === "label" && el.control &&
  ["submit", "image"].includes(el.control.type) ? el.control : null;
// A shadow label can be an empty slot in DOM text while its assigned content is the visible label.
/* exported renderedLabelText */
const renderedLabelText = el => {
  const text = el.innerText || el.textContent || "";
  if (text.trim()) return text;
  const slots = el.localName === "slot" ? [el] : Array.from(el.querySelectorAll("slot"));
  return slots.map(slot => slot.assignedNodes({flatten:true}).map(node => node.textContent || "").join(" ")).join(" ");
};
const FORM_CONTROLS = 'input:not([type=hidden]),textarea,select,[role=textbox],[role=searchbox],[role=combobox],' +
  '[role=checkbox],[role=radio],[role=switch],[role=slider],[contenteditable=""],[contenteditable=true],[contenteditable=plaintext-only]';
const formControl = el => matches(el, FORM_CONTROLS) || Boolean(labelledToggle(el));

// Many forms put an unassociated label in one column and a custom widget in the next. Only a small
// preceding label/legend in the same local row is eligible; never borrow another field's own label, also not an
// unassociated one that shares a preceding row with that row's own control.
const nearbyLabel = el => {
  let fallback = '';
  for (let cur = el, depth = 0; cur && depth < 6; cur = cur.parentElement, depth += 1) {
    if (matches(cur, 'form,main,article,body,nav')) break;
    const sibling = cur.previousElementSibling;
    if (sibling) {
      const labels = sibling.matches('label,legend') ? [sibling] : sibling.querySelectorAll('label,legend');
      if (labels.length === 1) {
        const label = labels[0];
        if (!label.control && !label.querySelector('input,select,textarea') && visible(label) &&
          (sibling === label || !sibling.querySelector(FORM_CONTROLS)))
          return clip(label.innerText || label.textContent, 160);
      }
      if (!fallback && !labels.length && sibling.children.length <= 1 &&
          !sibling.matches('input,button,select,textarea,a') && !sibling.querySelector('input,button,select,textarea,a') && visible(sibling)) {
        const text = (sibling.innerText || sibling.textContent || '').trim();
        if (text.length <= 160) fallback = clip(text, 160);
      }
    }
    if (cur.localName === 'fieldset') {
      const legend = cur.querySelector(':scope > legend');
      return legend && visible(legend) ? clip(legend.innerText || legend.textContent, 160) : '';
    }
  }
  return fallback;
};

const controlSearchText = original => {
  const el = labelledToggle(original) || original;
  if (!formControl(el)) return '';
  const parts = [fieldLabelText(el), el.getAttribute('placeholder'), nearbyLabel(el)];
  if (el.localName === 'select' && !vaultFilled(el) && !secretKind(el)) {
    // Selected text is already the readable value. Also find by its underlying value, or by an
    // offered option beyond the short preview. Only matching display evidence crosses the cap.
    const options = Array.from(el.options || []).filter(option => !option.hidden &&
      !option.closest('[hidden]') && getComputedStyle(option).display !== 'none');
    parts.unshift(...options.filter(option => option.selected).map(option => option.value));
    if (typeof REQ.find === 'string') {
      const needle = REQ.find.toLowerCase();
      const option = options.find(option => String(option.label || option.text).toLowerCase().includes(needle));
      if (option) parts.unshift(clip(option.label || option.text, 300));
    }
  }
  // React-select places its placeholder and selected display value next to its empty text input.
  // These selectors describe display nodes, not a container's entire text or hidden options.
  if (el.getAttribute('role') === 'combobox') {
    const root = el.getRootNode();
    for (const id of String(el.getAttribute('aria-describedby') || '').split(/\s+/).filter(Boolean).slice(0, 4)) {
      const target = root.getElementById?.(id);
      if (target && visible(target)) parts.push(target.innerText || target.textContent);
    }
    for (let up = el.parentElement, depth = 0; up && depth < 3; up = up.parentElement, depth += 1) {
      if (matches(up, 'form,main,body') || up.querySelectorAll('input').length !== 1) break;
      for (const display of Array.from(up.querySelectorAll(
        '[class*="__single-value"],[class*="-singleValue"],[class*="__placeholder"],[class*="-placeholder"]')).slice(0, 8)) {
        if (visible(display)) parts.push(display.innerText || display.textContent);
      }
    }
  }
  return clip(hideKeys([...new Set(parts.filter(Boolean))].join(' '), el), 600);
};
