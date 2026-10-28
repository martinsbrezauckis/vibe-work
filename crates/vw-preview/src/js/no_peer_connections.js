/*
 * No WebRTC in this document and in every same-origin frame it creates: the page viewer and the
 * page checker run this before any page script, in every frame the engine injects into. A prototype page has no
 * network by its CSP (`connect-src 'none'`), but CSP does not govern WebRTC: without this a page could reach a
 * STUN/TURN server it names and send what the user typed into the mockup.
 *
 * What it removes: `RTCPeerConnection`, `webkitRTCPeerConnection`, `RTCDataChannel` (and their related constructors
 * that only make sense with a peer connection). A frame the engine may not inject into (an `about:blank` or `srcdoc`
 * frame the page creates and reads at once) is cleaned by this realm synchronously: after every DOM insertion method
 * returns, when `contentWindow`/`contentDocument` is read, and for frames the parser inserts (a mutation observer runs
 * before the next parser-inserted script). Frames of other origins cannot be touched from here; the engine injects
 * into them itself.
 *
 * Depends on: nothing (plain DOM). Called by: vw-webview (an all-frames initialization script when a mount asks for
 * `no_peer_connections`).
 */
(() => {
  const NAMES = [
    "RTCPeerConnection",
    "webkitRTCPeerConnection",
    "RTCDataChannel",
    "RTCRtpSender",
    "RTCRtpReceiver",
    "RTCRtpTransceiver",
    "RTCIceCandidate",
    "RTCSessionDescription",
    "RTCDtlsTransport",
    "RTCIceTransport",
    "RTCSctpTransport",
    "RTCCertificate",
  ];
  const MARK = "__vwNoPeer";
  const scrub = (win) => {
    try {
      if (!win || win[MARK]) return;
      for (const name of NAMES) {
        try {
          delete win[name];
        } catch { /* A non-configurable property is handled by the defineProperty fallback below. */ }
        if (name in win) {
          try {
            Object.defineProperty(win, name, { value: undefined, configurable: false, writable: false });
          } catch { /* Some webview globals cannot be redefined; continue scrubbing the other names. */ }
        }
      }
      Object.defineProperty(win, MARK, { value: true });
    } catch {
      // A frame of another origin: not reachable from here (the engine injects into it).
    }
  };
  const scrubFrames = (root) => {
    try {
      for (let i = 0; i < root.frames.length; i += 1) {
        scrub(root.frames[i]);
        scrubFrames(root.frames[i]);
      }
    } catch { /* Cross-origin frame trees cannot be inspected from this document. */ }
  };
  scrub(window);
  const after = (proto, name) => {
    const original = proto && Object.getOwnPropertyDescriptor(proto, name);
    if (!original || typeof original.value !== "function") return;
    const wrapped = {
      [name](...args) {
        const result = original.value.apply(this, args);
        scrubFrames(window);
        return result;
      },
    }[name];
    Object.defineProperty(proto, name, { ...original, value: wrapped });
  };
  const afterSetter = (proto, name) => {
    const original = proto && Object.getOwnPropertyDescriptor(proto, name);
    if (!original || typeof original.set !== "function") return;
    Object.defineProperty(proto, name, {
      ...original,
      set(value) {
        original.set.call(this, value);
        scrubFrames(window);
      },
    });
  };
  const beforeGetter = (proto, name) => {
    const original = proto && Object.getOwnPropertyDescriptor(proto, name);
    if (!original || typeof original.get !== "function") return;
    Object.defineProperty(proto, name, {
      ...original,
      get() {
        const value = original.get.call(this);
        if (value) scrub(name === "contentDocument" ? value.defaultView : value);
        return value;
      },
    });
  };
  for (const name of ["appendChild", "insertBefore", "replaceChild"]) after(Node.prototype, name);
  for (const name of ["append", "prepend", "after", "before", "replaceWith", "insertAdjacentElement", "insertAdjacentHTML", "replaceChildren"])
    after(Element.prototype, name);
  for (const name of ["append", "prepend", "replaceChildren"]) after(Document.prototype, name);
  for (const name of ["write", "writeln"]) after(Document.prototype, name);
  for (const name of ["insertNode", "surroundContents"]) after(typeof Range === "function" ? Range.prototype : null, name);
  for (const name of ["innerHTML", "outerHTML"]) afterSetter(Element.prototype, name);
  if (typeof ShadowRoot === "function") {
    afterSetter(ShadowRoot.prototype, "innerHTML");
    after(ShadowRoot.prototype, "setHTMLUnsafe");
  }
  after(Element.prototype, "setHTMLUnsafe");
  for (const proto of [
    typeof HTMLIFrameElement === "function" ? HTMLIFrameElement.prototype : null,
    typeof HTMLFrameElement === "function" ? HTMLFrameElement.prototype : null,
    typeof HTMLObjectElement === "function" ? HTMLObjectElement.prototype : null,
  ]) {
    beforeGetter(proto, "contentWindow");
    beforeGetter(proto, "contentDocument");
  }
  // Frames the parser inserts: the observer runs at the microtask checkpoint before the next parser-inserted script.
  try {
    new MutationObserver(() => scrubFrames(window)).observe(document, { childList: true, subtree: true });
  } catch { /* The load listener below also scrubs frames when observation is unavailable. */ }
  addEventListener("load", () => scrubFrames(window), true);
})();
