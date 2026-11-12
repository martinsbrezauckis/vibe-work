// Installed at document start in each engine's isolated world, before page event listeners.
// The checked-input receipt supplies the one-use guard. New documents fail closed briefly without one.
(() => {
  if (window.__vwPointGuard) return;
  let current = null;
  let keyboardFence = null;
  Object.defineProperty(window, "__vwPointGuard", { value: Object.freeze({
    set: guard => { current = guard; },
    clear: guard => { if (current === guard) current = null; },
    holdKeyboard: token => { keyboardFence = token; },
    releaseKeyboard: token => { if (keyboardFence === token) keyboardFence = null; },
  }) });
  // A document born during native dispatch has no receipt. Use its time origin, not registration/host
  // messages: after 300 ms new input is usable even if the host never discovers this document.
  // The native adapters admit submission for at most 200 ms from validation start (POINT_DISPATCH_LIMIT).
  // Event creation time also covers input queued while young but delivered after a busy page yields.
  const newborn = 300;
  let deniedSequence = false;
  for (const type of ["pointerdown", "pointerup", "mousedown", "mouseup", "click", "auxclick", "dblclick", "contextmenu", "wheel", "keydown", "keyup", "beforeinput", "input"])
    window.addEventListener(type, event => {
      if (keyboardFence && event.isTrusted && ["keydown", "keyup", "beforeinput", "input"].includes(event.type)) {
        event.preventDefault(); event.stopImmediatePropagation();
      } else if (current) current(event);
      else if (event.isTrusted) {
        const young = performance.now() < newborn || event.timeStamp < newborn;
        // Engines may synthesize the trailing click/dblclick only after a busy page yields, with a new
        // timestamp. It still belongs to the denied press. A fresh press after the grace period always
        // clears this bit before cancellation; keyboard/accessibility activation (detail 0) is independent.
        if (event.type === "pointerdown" || event.type === "mousedown") deniedSequence = young;
        if (young) deniedSequence = true;
        const activation = ["click", "auxclick", "dblclick", "contextmenu"].includes(event.type);
        const independent = event.detail === 0 && event.button !== 2;
        if (young || deniedSequence && activation && !independent) {
          event.preventDefault();
          event.stopImmediatePropagation();
        }
      }
    }, { capture: true, passive: false });
})();
