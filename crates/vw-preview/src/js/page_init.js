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
  window.addEventListener("contextmenu", (event) => {
    closeContextMenu();
    contextChoice = "";
    contextElement = "selected text";
    contextTarget = null;
    contextTokenize = false;
    contextReady = false;
    contextIntent = false;
    if (!event.isTrusted) return;
    const selection = window.getSelection();
    let value = selection?.containsNode?.(event.target, true) ? String(selection.toString()).trim() : "";
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
    if (!value || value.length > 4096) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    contextChoice = value;
    const menu = document.createElement("div");
    menu.setAttribute("role", "menu");
    menu.style.cssText = "position:fixed;z-index:2147483647;background:#242429;color:#fff;border:1px solid #666;border-radius:6px;padding:4px;box-shadow:0 6px 20px #0008;font:13px system-ui";
    menu.style.left = Math.max(0, Math.min(event.clientX, window.innerWidth - 160)) + "px";
    menu.style.top = Math.max(0, Math.min(event.clientY, window.innerHeight - 40)) + "px";
    const item = document.createElement("button");
    item.type = "button";
    item.setAttribute("role", "menuitem");
    item.textContent = "Save to vault";
    item.style.cssText = "display:block;background:transparent;color:inherit;border:0;border-radius:4px;padding:8px 12px;cursor:pointer;font:inherit";
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
    menu.appendChild(item);
    document.documentElement.appendChild(menu);
    contextMenu = menu;
    item.focus();
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
  } catch (_) {
    // Already locked by the page: the engine's clearing at the next mount still applies.
  }
  try {
    new MutationObserver((list) => {
      mutations += list.length;
    }).observe(document, { subtree: true, childList: true, characterData: true, attributes: true });
  } catch (_) {
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
