// The visual check of one handbook page (Handbook Format 1.2). The ENGINE evaluates this in the hidden
// checker webview, inside the empty check document of the page server (`/.vw-check-<token>/`): it lays the page out
// in two iframes (desktop 1280 x 800, phone 390 x 844), measures each in the light and the dark theme (the
// `data-theme` attribute a reader sets, §6.4) and reports cut-off text, sideways scrolling, unreadable colours and
// script errors (prototype pages record theirs through the check route's prelude, `window.__vwCheckErrors`).
//
// Called by: `crates/vw-pages/src/checker.rs` (`start_script`), which fills in __ACCESSOR__, __PAGE__, __WAIT__.
// Invariants: reads only; the answer is untrusted page data (the Rust side caps every field again); the state lives
// behind a non-writable, non-enumerable function on the check document's window, so a page script in an iframe
// cannot overwrite it; every wait is bounded (each iframe __WAIT__ ms, then reported as not loaded).
(function () {
  var ACC = __ACCESSOR__, PAGE = __PAGE__, WAIT = __WAIT__;
  var state = { state: "pending" };
  try {
    Object.defineProperty(window, ACC, {
      value: function () { return JSON.stringify(state); },
      enumerable: false, configurable: false, writable: false,
    });
  } catch (e) {
    return JSON.stringify({ state: "failed", error: "accessor: " + e });
  }
  var VIEWS = [{ name: "desktop", w: 1280, h: 800 }, { name: "phone", w: 390, h: 844 }];
  var THEMES = ["light", "dark"];
  var MAX_ELEMENTS = 3000, MAX_ISSUES = 40;
  var issues = [], byKey = {}, errors = [], notes = [];
  var SKIP = { SCRIPT: 1, STYLE: 1, NOSCRIPT: 1, TEMPLATE: 1, TITLE: 1, HEAD: 1, META: 1, LINK: 1 };

  function clip(s, n) { s = String(s == null ? "" : s).replace(/\s+/g, " ").trim(); return s.length > n ? s.slice(0, n) + "…" : s; }
  function selector(el) {
    var parts = [];
    for (var e = el, d = 0; e && e.nodeType === 1 && d < 4; e = e.parentElement, d++) {
      var p = e.tagName.toLowerCase();
      if (e.id) { parts.unshift(p + "#" + e.id); break; }
      if (e.classList && e.classList.length) p += "." + e.classList[0];
      parts.unshift(p);
    }
    return clip(parts.join(" > "), 160);
  }
  function add(kind, view, theme, el, detail) {
    var sel = el ? selector(el) : "";
    // One issue per element for layout problems (their pixel numbers differ per width); colours per detail.
    var key = kind + "|" + sel + "|" + (kind === "contrast" || kind === "scriptError" || !sel ? detail : "");
    var label = view + (theme ? " " + theme : "");
    if (byKey[key]) { if (byKey[key].views.indexOf(label) < 0) byKey[key].views.push(label); return; }
    if (issues.length >= MAX_ISSUES) return;
    var item = { kind: kind, views: [label], element: sel, text: el ? clip(el.textContent, 80) : "", detail: clip(detail, 200) };
    byKey[key] = item; issues.push(item);
  }
  // CSS colour → [r, g, b, a] (computed values are rgb()/rgba() or color(srgb ...)).
  function rgba(c) {
    if (!c || c === "transparent") return [0, 0, 0, 0];
    var m = /rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)(?:\s*[,/]\s*([\d.]+%?))?\s*\)/.exec(c);
    if (m) { var a = m[4] === undefined ? 1 : (m[4].slice(-1) === "%" ? parseFloat(m[4]) / 100 : parseFloat(m[4])); return [+m[1], +m[2], +m[3], a]; }
    m = /color\(srgb\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)(?:\s*\/\s*([\d.]+))?\)/.exec(c);
    if (m) return [m[1] * 255, m[2] * 255, m[3] * 255, m[4] === undefined ? 1 : +m[4]];
    return null;
  }
  function over(top, bottom) {
    var a = top[3];
    return [top[0] * a + bottom[0] * (1 - a), top[1] * a + bottom[1] * (1 - a), top[2] * a + bottom[2] * (1 - a), 1];
  }
  function lum(c) {
    var v = [c[0], c[1], c[2]].map(function (x) { x /= 255; return x <= 0.03928 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4); });
    return 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
  }
  function ratio(a, b) { var x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); }
  function hex(c) { return "#" + [c[0], c[1], c[2]].map(function (x) { var h = Math.round(x).toString(16); return h.length < 2 ? "0" + h : h; }).join(""); }
  // The opaque colour behind `el`: its own and its ancestors' backgrounds over the canvas; null when an image or
  // gradient is in the way (unknown, not reported).
  function backdrop(el, win, canvas) {
    var layers = [];
    for (var e = el; e && e.nodeType === 1; e = e.parentElement) {
      var cs = win.getComputedStyle(e);
      if (cs.backgroundImage && cs.backgroundImage !== "none") return null;
      var c = rgba(cs.backgroundColor);
      if (!c) return null;
      if (c[3] > 0) layers.push(c);
      if (c[3] >= 1) break;
    }
    var result = canvas;
    for (var i = layers.length - 1; i >= 0; i--) result = over(layers[i], result);
    return result;
  }
  function ownText(el) {
    for (var n = el.firstChild; n; n = n.nextSibling) if (n.nodeType === 3 && /\S/.test(n.nodeValue)) return true;
    return false;
  }
  function visible(el, cs) {
    return cs.display !== "none" && cs.visibility !== "hidden" && parseFloat(cs.opacity) > 0.05 && el.getClientRects().length > 0;
  }

  function measure(view, theme, win, doc) {
    var root = doc.documentElement;
    root.setAttribute("data-theme", theme);
    var probe = doc.createElement("div");
    probe.style.cssText = "position:absolute;width:1px;height:1px;background:Canvas;color:CanvasText";
    (doc.body || root).appendChild(probe);
    var canvas = rgba(win.getComputedStyle(probe).backgroundColor) || [255, 255, 255, 1];
    if (canvas[3] < 1) canvas = over(canvas, [255, 255, 255, 1]);
    probe.remove();
    var rootCs = win.getComputedStyle(root);
    var width = win.innerWidth;
    if (root.scrollWidth > width + 1) {
      var clipped = rootCs.overflowX === "hidden" || rootCs.overflowX === "clip";
      var wide = [], all = doc.body ? doc.body.querySelectorAll("*") : [];
      for (var i = 0; i < all.length && i < MAX_ELEMENTS && wide.length < 3; i++) {
        var r = all[i].getBoundingClientRect(), pr = all[i].parentElement ? all[i].parentElement.getBoundingClientRect() : null;
        if (r.right > width + 1 && (!pr || pr.right <= width + 1)) wide.push(all[i]);
      }
      var what = "the page is " + root.scrollWidth + " px wide in a " + width + " px window";
      if (!wide.length) add(clipped ? "cutOff" : "sideways", view, null, null, what);
      wide.forEach(function (el) {
        add(clipped ? "cutOff" : "sideways", view, null, el, what + " (this element ends at " + Math.round(el.getBoundingClientRect().right) + " px)");
      });
    }
    var els = doc.body ? doc.body.querySelectorAll("*") : [];
    for (var k = 0; k < els.length && k < MAX_ELEMENTS; k++) {
      var el = els[k];
      if (SKIP[el.tagName]) continue;
      var cs = win.getComputedStyle(el);
      if (!visible(el, cs)) continue;
      var text = /\S/.test(el.textContent || "");
      if (text && el.clientWidth > 0) {
        var hx = (cs.overflowX === "hidden" || cs.overflowX === "clip") && el.scrollWidth > el.clientWidth + 1;
        var hy = (cs.overflowY === "hidden" || cs.overflowY === "clip") && el.scrollHeight > el.clientHeight + 1;
        if (hx || hy) add("cutOff", view, null, el, hx ? "text is cut off at the side (" + el.scrollWidth + " px of text in " + el.clientWidth + " px)" : "text is cut off at the bottom (" + el.scrollHeight + " px of text in " + el.clientHeight + " px)");
      }
      if (!ownText(el)) continue;
      var bg = backdrop(el, win, canvas);
      var fg = rgba(cs.color);
      if (!bg || !fg) continue;
      var shown = over(fg, bg);
      var size = parseFloat(cs.fontSize), weight = parseInt(cs.fontWeight, 10) || 400;
      var need = size >= 24 || (size >= 18.66 && weight >= 700) ? 3 : 4.5;
      var got = ratio(shown, bg);
      if (got < need) add("contrast", view, theme, el, "contrast " + got.toFixed(2) + ":1 (" + hex(shown) + " on " + hex(bg) + "), needs " + need + ":1");
    }
    return { bg: rgba(win.getComputedStyle(doc.body || root).backgroundColor), fg: rgba(win.getComputedStyle(doc.body || root).color) };
  }

  function load(view) {
    return new Promise(function (resolve) {
      var frame = document.createElement("iframe");
      frame.style.width = view.w + "px"; frame.style.height = view.h + "px";
      var done = false;
      var finish = function (ok) { if (!done) { done = true; resolve({ frame: frame, ok: ok }); } };
      frame.addEventListener("load", function () { finish(true); });
      setTimeout(function () { finish(false); }, WAIT);
      frame.src = PAGE;
      document.body.appendChild(frame);
    });
  }
  function settle(win) {
    var fonts = win.document.fonts && win.document.fonts.ready ? win.document.fonts.ready : Promise.resolve();
    return Promise.race([fonts, new Promise(function (r) { setTimeout(r, 1500); })]).then(function () {
      return new Promise(function (r) { setTimeout(r, 400); });
    });
  }

  var chain = Promise.resolve();
  VIEWS.forEach(function (view) {
    chain = chain.then(function () { return load(view); }).then(function (loaded) {
      var win = loaded.frame.contentWindow, doc = null;
      try { doc = loaded.frame.contentDocument; } catch { doc = null; }
      if (!loaded.ok || !doc || !doc.documentElement) { notes.push(view.name + ": the page did not load within " + WAIT + " ms"); return; }
      return settle(win).then(function () {
        var looks = THEMES.map(function (theme) { return measure(view.name, theme, win, doc); });
        if (JSON.stringify(looks[0]) === JSON.stringify(looks[1])) add("theme", view.name, null, null, "the page looks the same in light and dark: its stylesheet has no [data-theme] rules, so it does not follow the app's theme");
        var recorded = win.__vwCheckErrors;
        if (recorded && recorded.length) recorded.forEach(function (m) { if (errors.indexOf(m) < 0 && errors.length < 20) errors.push(String(m).slice(0, 300)); });
      });
    });
  });
  chain.then(function () {
    errors.forEach(function (m) { add("scriptError", "desktop and phone", null, null, m); });
    state = { state: "done", issues: issues, notes: notes };
  }, function (e) {
    state = { state: "failed", error: String(e).slice(0, 300) };
  });
  return JSON.stringify(state);
})()
