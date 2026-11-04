// Bounded form label/context discovery for the shared page scripts. Depends on common.js helpers;
// used by observation, labelOf and form prioritization. Context never reads input values.
/* global clip, fieldLabelText, hideKeys, matches, visible -- Shared script helpers. */
/* exported formControl, nearbyLabel, controlSearchText, labelledToggle */
const labelledToggle = el => el.localName === "label" && el.control &&
  matches(el.control, 'input[type=checkbox],input[type=radio]') ? el.control : null;
const formControl = el => matches(el,
  'input:not([type=hidden]),textarea,select,[role=textbox],[role=searchbox],[role=combobox],' +
  '[role=checkbox],[role=radio],[role=switch],[role=slider],[contenteditable=""],[contenteditable=true]') ||
  Boolean(labelledToggle(el));

// Many forms put an unassociated label in one column and a custom widget in the next. Only a small
// preceding label/legend in the same local row is eligible; never borrow another field's own label.
const nearbyLabel = el => {
  let fallback = '';
  for (let cur = el, depth = 0; cur && depth < 6; cur = cur.parentElement, depth += 1) {
    if (matches(cur, 'form,main,article,body,nav')) break;
    const sibling = cur.previousElementSibling;
    if (sibling) {
      const labels = sibling.matches('label,legend') ? [sibling] : sibling.querySelectorAll('label,legend');
      if (labels.length === 1) {
        const label = labels[0];
        if (!label.control && !label.querySelector('input,select,textarea') && visible(label))
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
