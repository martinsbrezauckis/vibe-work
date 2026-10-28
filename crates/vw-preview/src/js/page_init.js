// Vibe Work browser page environment, installed at document start in every page of a session's
// browser. It keeps the facts the observe/act scripts need and nothing the page can use against the user.
// What the user types is readable to the agent like any page text, so there is no typing tracker; secret fields
// are recognised by their kind (`js/common.js` in vw-browser):
// - dialogs (alert/confirm/prompt) raised while an agent action runs are answered at once (ok / cancel / empty)
//   and logged for the receipt, so an action never hangs on a dialog; outside that window the page's own
//   dialogs work as usual for the user;
// - a mutation counter (settling after an action) and a stability store (actionability);
// - service workers are off : a worker outlives its page and could send requests later under
//   another page's rights; the engine also clears registrations at every mount.
// The accessor name is random per mount and non-enumerable: a collision guard, not a secret.
(() => {
  const NAME = __VW_NAME__;
  // The page's own menu wears the app's menu tokens (light and dark values, filled in at mount) for the page's light
  // or dark: a declared `color-scheme`, `prefers-color-scheme` when the page offers both, else the colour the page
  // paints; the app's theme when the page shows none of these.
  const APP_THEME = __VW_THEME__;
  const MENU_TOKENS = __VW_MENU__;
  const pageTheme = () => {
    try {
      const scheme = String(getComputedStyle(document.documentElement).colorScheme || "").split(/\s+/);
      const light = scheme.includes("light");
      const dark = scheme.includes("dark");
      if (dark !== light) return dark ? "dark" : "light";
      if (dark && light) return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
      for (const el of [document.body, document.documentElement]) {
        if (!el) continue;
        const rgb = /rgba?\(([\d.]+),\s*([\d.]+),\s*([\d.]+)(?:,\s*([\d.]+))?/.exec(getComputedStyle(el).backgroundColor);
        if (rgb && (rgb[4] === undefined || Number(rgb[4]) >= 0.5)) {
          return 0.2126 * rgb[1] + 0.7152 * rgb[2] + 0.0722 * rgb[3] < 128 ? "dark" : "light";
        }
      }
    } catch {
      // A page that breaks style reads gets the app's theme.
    }
    return APP_THEME;
  };
  if (Object.prototype.hasOwnProperty.call(window, NAME)) return;
  const dialogs = [];
  const stability = new WeakMap();
  let mutations = 0;
  let agentUntil = 0;
  let contextChoice = "";
  let contextElement = "selected text";
  let contextTarget = null;
  let contextTokenize = false;
  let contextReady = false;
  let contextIntent = false;
  let contextExpires = 0;
  let contextMenu = null;
  // Point Vibe at this: what the right click was on, to select it again for the shell's pick (a selection on any
  // web page; on a page of this computer also an element's own text).
  let pointRange = null;
  let pointIntent = false;
  let pointExpires = 0;
  const closeContextMenu = () => {
    if (contextMenu) contextMenu.remove();
    contextMenu = null;
  };
  // The page can read the intent accessor, but only a trusted page-menu click can set it.
  Object.defineProperty(window, "__vw_context_menu_intent", {
    value: Object.freeze({ take: () => {
      const ready = contextIntent && Date.now() < contextExpires;
      contextIntent = false;
      return ready;
    } }),
    enumerable: false,
    configurable: false,
    writable: false,
  });
  // Point Vibe at this: the same one-use, click-only pattern; the selection stays in the page and the shell's pick
  // reads it there.
  Object.defineProperty(window, "__vw_point_intent", {
    value: Object.freeze({ take: () => {
      const ready = pointIntent && Date.now() < pointExpires;
      pointIntent = false;
      return ready;
    } }),
    enumerable: false,
    configurable: false,
    writable: false,
  });
  const localPage = () => /^https?:$/.test(location.protocol) && ["localhost", "127.0.0.1", "[::1]"].includes(location.hostname);
  window.addEventListener("contextmenu", (event) => {
    closeContextMenu();
    contextChoice = "";
    contextElement = "selected text";
    contextTarget = null;
    contextTokenize = false;
    contextReady = false;
    contextIntent = false;
    pointRange = null;
    pointIntent = false;
    if (!event.isTrusted) return;
    const selection = window.getSelection();
    const selected = selection?.containsNode?.(event.target, true) && String(selection.toString()).trim() !== "";
    let value = selected ? String(selection.toString()).trim() : "";
    const webPage = /^https?:$/.test(location.protocol);
    if (selected && webPage && selection.rangeCount > 0) pointRange = selection.getRangeAt(0).cloneRange();
    const copyRange = selected && selection.rangeCount > 0 ? selection.getRangeAt(0).cloneRange() : null;
    if (!value) {
      const target = event.target;
      const el = target?.closest?.("input,textarea,code,pre,[data-copy],[data-copy-value],[data-clipboard-text]") || target;
      if (el?.nodeType === 1) {
        const tag = String(el.localName || "element").toLowerCase();
        const field = tag === "input" || tag === "textarea";
        const attr = el.getAttribute?.("data-copy-value") || el.getAttribute?.("data-clipboard-text") || el.getAttribute?.("data-copy");
        if (field || attr || el.childElementCount === 0 || tag === "code" || tag === "pre") {
          value = String(field ? el.value : (attr || el.textContent || "")).trim();
          contextElement = tag.slice(0, 30);
          if (!field && !attr) {
            contextTarget = el;
            contextTokenize = true;
          }
        }
      }
    }
    // An element's own text (not a form field) can be pointed at too: it is selected again on the click.
    if (!pointRange && contextTarget && localPage()) {
      pointRange = document.createRange();
      pointRange.selectNodeContents(contextTarget);
    }
    const saveable = !!value && value.length <= 4096;
    if (!saveable && !pointRange) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    contextChoice = saveable ? value : "";
    const menu = document.createElement("div");
    menu.setAttribute("role", "menu");
    const theme = pageTheme();
    const tokens = MENU_TOKENS[theme];
    menu.dataset.vwTheme = theme;
    menu.style.cssText = `position:fixed;z-index:2147483647;min-width:160px;background:${tokens.raised};color:${tokens["text-2"]};border-radius:10px;padding:4px;box-shadow:${tokens.shadow.replace("var(--line-2)", tokens["line-2"])};font:13px "Inter Variable",Inter,system-ui,sans-serif;color-scheme:${theme}`;
    menu.style.left = Math.max(0, Math.min(event.clientX, window.innerWidth - 160)) + "px";
    menu.style.top = Math.max(0, Math.min(event.clientY, window.innerHeight - 130)) + "px";
    const button = (label) => {
      const b = document.createElement("button");
      b.type = "button";
      b.setAttribute("role", "menuitem");
      b.textContent = label;
      b.style.cssText = "display:block;width:100%;box-sizing:border-box;text-align:left;background:transparent;color:inherit;border:0;outline:none;border-radius:6px;padding:7px 10px;cursor:pointer;font:inherit";
      // Hover and focus as the app's menu: the active background and the main text colour.
      const on = () => {
        b.style.background = tokens.active;
        b.style.color = tokens.text;
      };
      const off = () => {
        if (document.activeElement === b) return;
        b.style.background = "transparent";
        b.style.color = "inherit";
      };
      b.addEventListener("mouseenter", on);
      b.addEventListener("focus", on);
      b.addEventListener("mouseleave", off);
      b.addEventListener("blur", () => {
        b.style.background = "transparent";
        b.style.color = "inherit";
      });
      return b;
    };
    if (pointRange) {
      const range = pointRange;
      const point = button("Point Vibe at this");
      point.addEventListener("click", (click) => {
        click.preventDefault();
        click.stopImmediatePropagation();
        if (click.isTrusted && contextMenu === menu) {
          const again = window.getSelection();
          again?.removeAllRanges();
          again?.addRange(range);
          pointIntent = true;
          pointExpires = Date.now() + 10000;
        }
        contextChoice = "";
        contextTarget = null;
        contextTokenize = false;
        closeContextMenu();
      }, true);
      menu.appendChild(point);
    }
    // Copy: this menu replaces the system's one, so the selection can still be copied (a trusted click only).
    if (copyRange) {
      const copy = button("Copy");
      copy.addEventListener("click", (click) => {
        click.preventDefault();
        click.stopImmediatePropagation();
        if (click.isTrusted && contextMenu === menu) {
          const again = window.getSelection();
          again?.removeAllRanges();
          again?.addRange(copyRange);
          document.execCommand("copy");
        }
        contextChoice = "";
        contextTarget = null;
        contextTokenize = false;
        closeContextMenu();
      }, true);
      menu.appendChild(copy);
    }
    const item = button("Save to vault");
    item.addEventListener("click", (click) => {
      click.preventDefault();
      click.stopImmediatePropagation();
      if (click.isTrusted && contextMenu === menu) {
        contextReady = true;
        contextIntent = true;
        contextExpires = Date.now() + 60000;
        const expiry = contextExpires;
        window.setTimeout(() => {
          if (contextExpires === expiry) {
            contextChoice = "";
            contextTarget = null;
            contextTokenize = false;
            contextReady = false;
            contextIntent = false;
          }
        }, 60000);
      } else {
        contextChoice = "";
        contextTarget = null;
        contextTokenize = false;
      }
      closeContextMenu();
    }, true);
    if (saveable) menu.appendChild(item);
    document.documentElement.appendChild(menu);
    contextMenu = menu;
    menu.firstChild?.focus();
  }, true);
  window.addEventListener("pointerdown", (event) => {
    if (contextMenu && !contextMenu.contains(event.target)) {
      closeContextMenu();
      contextChoice = "";
      contextTarget = null;
      contextTokenize = false;
    }
  }, true);
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      closeContextMenu();
      contextChoice = "";
      contextTarget = null;
      contextTokenize = false;
    }
  }, true);
  const answers = { alert: undefined, confirm: false, prompt: null };
  for (const kind of Object.keys(answers)) {
    const original = window[kind];
    if (typeof original !== "function") continue;
    window[kind] = function (message) {
      if (Date.now() < agentUntil) {
        dialogs.push({ kind, message: String(message === undefined ? "" : message).slice(0, 300) });
        if (dialogs.length > 20) dialogs.shift();
        return answers[kind];
      }
      return original.apply(this, arguments);
    };
  }
  try {
    const refuse = () => Promise.reject(new DOMException("Service workers are off in this browser", "SecurityError"));
    if (window.ServiceWorkerContainer) {
      Object.defineProperty(ServiceWorkerContainer.prototype, "register", { value: refuse, configurable: false });
    }
  } catch {
    // Already locked by the page: the engine's clearing at the next mount still applies.
  }
  try {
    new MutationObserver((list) => {
      mutations += list.length;
    }).observe(document, { subtree: true, childList: true, characterData: true, attributes: true });
  } catch {
    // A document without a root yet: the counter stays 0 (settling then waits its minimum).
  }
  const api = {
    dialogs,
    stability,
    mutations: () => mutations,
    agentActing: (ms) => {
      agentUntil = Date.now() + Math.max(0, Math.min(Number(ms) || 0, 10000));
    },
    takeContextChoice: () => {
      const value = contextReady && Date.now() < contextExpires ? contextChoice : "";
      const element = contextElement;
      const target = contextTarget;
      const tokenize = contextTokenize;
      contextChoice = "";
      contextElement = "selected text";
      contextTarget = null;
      contextTokenize = false;
      contextReady = false;
      contextIntent = false;
      return { value, element, target, tokenize };
    },
  };
  Object.defineProperty(window, NAME, { value: api, enumerable: false, configurable: false, writable: false });
})();
