// Focus and selection-preserving insertion for act.js; uses retained refs, native setters and final readback.
/* global ENV, REQ, clip, deepActive, fire, result, secretKind, setValue, viewOf */
/* exported controlAction, controlPostcondition */
// Temporary non-interactive target outline, scoped to this isolated document and removed on clear or expiry.
function highlightTarget(node, clear) {
 const view=node.ownerDocument.defaultView,key=Symbol.for('vw.highlight.target');
 view[key]?.remove();
 if(clear)return {highlighted:false,cleared:true};
 const box=node.ownerDocument.createElement('div');box.dataset.vwHighlight='';box.setAttribute('aria-hidden','true');
 const styles={all:'initial',position:'fixed',boxSizing:'border-box',pointerEvents:'none',zIndex:'2147483647',border:'3px solid Highlight',borderRadius:'4px'};
 for(const [name,value] of Object.entries(styles))box.style.setProperty(name.replace(/[A-Z]/g,c=>'-'+c.toLowerCase()),value,'important');
 let timer,frame;const remove=()=>{clearTimeout(timer);view.cancelAnimationFrame(frame);box.remove();if(view[key]?.box===box)delete view[key];};
 const update=()=>{
  if(!node.isConnected){remove();return;}
  const r=node.getBoundingClientRect();
  for(const [name,value] of Object.entries({left:r.x-3,top:r.y-3,width:r.width+6,height:r.height+6}))box.style.setProperty(name,value+'px','important');
  frame=view.requestAnimationFrame(update);
 };
 node.ownerDocument.documentElement.append(box);view[key]={box,node,remove};update();timer=setTimeout(remove,1500);
 const r=box.getBoundingClientRect();
 return {highlighted:box.isConnected&&r.width>0&&r.height>0,cleared:false,bounds:{x:r.x,y:r.y,width:r.width,height:r.height},expiresAfterMs:1500};
}

const controlPostcondition = (el, target) => {
  if (REQ.action === "highlight") {
    const state=viewOf(el)[Symbol.for('vw.highlight.target')],shown=Boolean(state?.node===el&&state.box.isConnected);
    return result(shown !== Boolean(REQ.clear) ? "done" : "unmet",{target,effect:shown?"highlighted":"highlight cleared"});
  }
  if (REQ.action === "focus" || REQ.action === "blur") {
    const focused = deepActive(el.ownerDocument) === el;
    return result(focused === (REQ.action === "focus") ? "done" : "unmet", { target, effect: focused ? "focused" : "blurred" });
  }
  const actual = String(el.isContentEditable ? el.textContent : el.value);
  return result(ENV.insertExpected?.get(el) === actual ? "done" : "unmet", { target, value: clip(actual, 300) });
};
const controlAction = (el, target) => {
  if (REQ.action === "highlight") {highlightTarget(el,REQ.clear);return controlPostcondition(el,target);}
  if (REQ.action === "focus" || REQ.action === "blur") {
    if (REQ.action === "focus") el.focus({ preventScroll: true });
    else el.blur();
    return controlPostcondition(el, target);
  }
  const text = String(REQ.text ?? "");
  if (!el.isContentEditable && typeof el.selectionStart !== "number")
    return result("error", { message: "This control has no text selection. Use type to replace its value." });
  el.focus({ preventScroll: true });
  if (secretKind(el) || !el.isConnected || deepActive(el.ownerDocument) !== el) return result("verificationChanged");
  const previous = String(el.isContentEditable ? el.textContent : el.value);
  const start = el.selectionStart, end = el.selectionEnd;
  const selection = viewOf(el).getSelection();
  let range;
  if (el.isContentEditable) {
    range = selection.rangeCount ? selection.getRangeAt(0).cloneRange() : el.ownerDocument.createRange();
    if (!el.contains(range.startContainer) || !el.contains(range.endContainer)) {
      range.selectNodeContents(el); range.collapse(false);
    }
  }
  if (!fire(el, "beforeinput", "InputEvent", { bubbles: true, composed: true, cancelable: true, inputType: "insertText", data: text }))
    return result("unmet", { target });
  if (secretKind(el)) return result("verificationChanged");
  if (!el.isConnected || deepActive(el.ownerDocument) !== el || String(el.isContentEditable ? el.textContent : el.value) !== previous)
    return result("verificationChanged");
  let expected;
  if (el.isContentEditable) {
    range.deleteContents();
    const node = el.ownerDocument.createTextNode(text);
    range.insertNode(node); range.setStartAfter(node); range.collapse(true);
    selection.removeAllRanges(); selection.addRange(range);
    expected = el.textContent;
  } else {
    expected = previous.slice(0, start) + text + previous.slice(end);
    setValue(el, expected);
    el.setSelectionRange(start + text.length, start + text.length);
  }
  ENV.insertExpected ||= new WeakMap();
  ENV.insertExpected.set(el, expected);
  fire(el, "input", "InputEvent", { bubbles: true, composed: true, inputType: "insertText", data: text });
  return controlPostcondition(el, target);
};
