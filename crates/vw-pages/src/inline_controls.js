/* App-owned handbook controls in the viewer's top document. Depends on native sync/poll and sandbox layout messages.
 * Loaded at document start by PagesService; page scripts never share this realm. Only trusted gestures queue writes. */
(() => {
  if (window.top !== window || window.origin === "null") return;
  const controls = new Map();
  const kinds = new Set(["choice", "checklist", "rank", "text"]);
  const documentId = crypto.randomUUID();
  const pending = new Map();
  let sequence = 0;
  let pagePath = "";
  let frame = null;
  let lastSnapshot = "";
  let picking = null;
  let saveError = null;
  const values = Object.create(null);
  window.addEventListener(
    "click",
    (event) => {
      if (!picking) return;
      const root = event.target.closest?.(".hb-inline-control");
      if (!root) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      picking.result = {
        state: "picked",
        ids: [root.dataset.hbInput],
        path: `div#${root.dataset.hbInput}`,
        tag: "div",
        role: "group",
        text: root.textContent.slice(0, 2000),
        quote: null,
        url: location.href,
      };
    },
    true,
  );
  const element = (tag, cls, text) => {
    const node = document.createElement(tag);
    node.className = cls;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const post = (data) => frame?.contentWindow?.postMessage(data, "*");
  const navigate = async (href) => {
    api("flush");
    while (pending.size)
      await new Promise((resolve) => setTimeout(resolve, 25));
    if (!saveError) location.href = href;
  };
  window.addEventListener("click", (event) => {
    const link = event.target.closest?.("a[href]");
    if (!event.isTrusted || event.defaultPrevented || !link) return;
    const url = new URL(link.href, location.href);
    if (
      url.pathname === location.pathname &&
      url.search === location.search &&
      url.hash
    )
      return;
    api("flush");
    if (!pending.size) return;
    event.preventDefault();
    void navigate(url.href);
  });
  const sizeObserver = new ResizeObserver((entries) => {
    if (!frame) return;
    const inputs = [];
    for (const entry of entries) {
      const control = controls.get(entry.target.dataset.hbInput);
      if (!control || control.root !== entry.target || control.root.hidden)
        continue;
      const height = Math.ceil(
        entry.borderBoxSize?.[0]?.blockSize ?? entry.target.offsetHeight,
      );
      if (height !== control.height) {
        control.height = height;
        inputs.push({ id: control.input.id, height });
      }
    }
    if (inputs.length)
      post({
        type: "hb-layout-init",
        inputs,
        theme: document.documentElement.dataset.theme,
      });
  });
  const broadcast = () => {
    const snapshot = JSON.stringify(values);
    if (snapshot === lastSnapshot) return;
    lastSnapshot = snapshot;
    post({ type: "hb-answers", answers: JSON.parse(snapshot) });
  };
  const commit = (control, value, event) => {
    if (!event.isTrusted || picking !== null) return;
    values[control.input.id] = value;
    pending.set(control.input.id, {
      input: control.input.id,
      value,
      sequence: ++sequence,
    });
    control.paint(value);
    broadcast();
  };
  const button = (cls, label, handler) => {
    const node = element("button", cls, label);
    node.type = "button";
    node.addEventListener("click", handler);
    return node;
  };
  const grow = (note) => {
    note.style.height = "auto";
    note.style.height = `${note.scrollHeight}px`;
  };
  const build = (input) => {
    const root = element("div", `hb-inline-control hb-inline-${input.kind}`);
    root.dataset.hbInput = input.id;
    root.setAttribute("role", "group");
    root.setAttribute("aria-label", input.question || input.id);
    root.append(element("p", "hb-inline-question", input.question || input.id));
    const control = {
      root,
      input,
      paint: () => {},
      signature: "",
      timer: 0,
      height: 0,
      flush: () => {},
      schema: JSON.stringify(input),
    };
    if (input.kind === "text") {
      const note = element("textarea", "hb-inline-note");
      note.rows = 1;
      note.maxLength = 2000;
      note.setAttribute("aria-label", input.question || input.id);
      note.placeholder = "Add a note…";
      let dirty = false;
      note.addEventListener("input", (event) => {
        if (event.isTrusted) dirty = true;
        clearTimeout(control.timer);
        control.flush = () => {};
        grow(note);
      });
      const save = (event) => {
        if (!event.isTrusted || !dirty) return;
        clearTimeout(control.timer);
        const text = note.value;
        control.flush = () => {
          clearTimeout(control.timer);
          dirty = false;
          commit(control, text.trim() ? { type: "text", text } : null, event);
          control.flush = () => {};
        };
        control.timer = setTimeout(control.flush, 150);
      };
      note.addEventListener("blur", save);
      note.addEventListener("keydown", (event) => {
        if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
          event.preventDefault();
          save(event);
        }
      });
      control.paint = (value) => {
        const text = value?.type === "text" ? value.text : "";
        if (note.value !== text) {
          clearTimeout(control.timer);
          control.flush = () => {};
          note.value = text;
          dirty = false;
          grow(note);
        }
      };
      root.append(note);
    } else if (input.kind === "rank") {
      const list = element("ol", "hb-inline-rank-list");
      const rows = new Map();
      for (const option of input.options) {
        const row = element("li", "hb-inline-rank-row");
        row.dataset.option = option.id;
        row.append(element("span", "hb-inline-label", option.label));
        for (const [delta, word, symbol] of [
          [-1, "up", "↑"],
          [1, "down", "↓"],
        ]) {
          const move = button("hb-inline-move", symbol, (event) => {
            const order = [
              ...(values[input.id]?.order ?? input.options.map((o) => o.id)),
            ];
            const index = order.indexOf(option.id);
            const to = index + delta;
            if (to < 0 || to >= order.length) return;
            [order[index], order[to]] = [order[to], order[index]];
            commit(control, { type: "rank", order }, event);
          });
          move.setAttribute("aria-label", `Move “${option.label}” ${word}`);
          row.append(move);
        }
        rows.set(option.id, row);
      }
      control.paint = (value) => {
        const order = value?.order ?? input.options.map((o) => o.id);
        order.forEach((id, index) => {
          const row = rows.get(id);
          if (!row) return;
          row.children[1].disabled = index === 0;
          row.children[2].disabled = index === order.length - 1;
          list.append(row);
        });
      };
      root.append(list);
    } else {
      const options = element("div", "hb-inline-options");
      for (const option of input.options) {
        const pick = button("hb-inline-option", option.label, (event) => {
          if (input.kind === "choice")
            commit(control, { type: "choice", option: option.id }, event);
          else {
            const checked = new Set(values[input.id]?.checked ?? []);
            if (checked.has(option.id)) checked.delete(option.id);
            else checked.add(option.id);
            commit(
              control,
              {
                type: "checklist",
                checked: input.options
                  .map((o) => o.id)
                  .filter((id) => checked.has(id)),
              },
              event,
            );
          }
        });
        pick.dataset.option = option.id;
        options.append(pick);
      }
      control.paint = (value) => {
        for (const pick of options.children) {
          pick.setAttribute(
            "aria-pressed",
            String(
              input.kind === "choice"
                ? value?.option === pick.dataset.option
                : (value?.checked ?? []).includes(pick.dataset.option),
            ),
          );
        }
      };
      root.append(options);
    }
    const clear = button("hb-inline-clear", "Clear", (event) =>
      commit(control, null, event),
    );
    root.append(clear);
    const paint = control.paint;
    control.paint = (value) => {
      paint(value);
      clear.hidden = value == null;
      control.signature = JSON.stringify(value);
    };
    control.paint(null);
    return control;
  };
  const layoutSizes = () => {
    if (!frame) return;
    post({
      type: "hb-layout-init",
      inputs: [...controls].map(([id, c]) => ({ id, height: c.height })),
      theme: document.documentElement.dataset.theme,
    });
  };
  const retire = (id) => {
    const control = controls.get(id);
    clearTimeout(control.timer);
    sizeObserver.unobserve(control.root);
    control.root.remove();
    controls.delete(id);
    pending.delete(id);
    delete values[id];
  };
  window.addEventListener("message", (event) => {
    if (!frame || event.source !== frame.contentWindow) return;
    const data = event.data;
    if (data?.type === "hb-ready") {
      lastSnapshot = "";
      broadcast();
      layoutSizes();
    } else if (
      data?.type === "hb-layout" &&
      Array.isArray(data.boxes) &&
      data.boxes.length <= 500
    ) {
      for (const box of data.boxes) {
        const control = controls.get(box.id);
        if (
          !control ||
          ![box.x, box.y, box.width, box.height].every(
            (n) =>
              typeof n === "number" && Number.isFinite(n) && Math.abs(n) < 1e7,
          )
        )
          continue;
        control.root.hidden =
          box.width <= 0 ||
          box.height <= 0 ||
          box.y + box.height < 0 ||
          box.y > innerHeight;
        Object.assign(control.root.style, {
          position: "fixed",
          left: `${box.x}px`,
          top: `${box.y}px`,
          width: `${Math.max(0, box.width)}px`,
        });
      }
    } else if (data?.type === "hb-navigate" && typeof data.href === "string") {
      void navigate(data.href);
    } else if (
      data?.type === "hb-picked" &&
      picking &&
      data.name === picking.name
    ) {
      picking.result = data.result;
    }
  });
  const api = (op, data) => {
    if (op === "flush") {
      for (const control of controls.values()) control.flush();
      op = "poll";
    }
    if (op === "poll")
      return JSON.stringify({
        document: documentId,
        path: decodeURIComponent(location.pathname.split("/").slice(2).join("/")),
        ready: document.readyState !== "loading",
        initialized: pagePath !== "",
        changes: [...pending.values()],
      });
    if (op === "pick") {
      picking = { name: data, result: { state: "pending" } };
      post({ type: "hb-pick", name: data });
      return true;
    }
    if (op === "pick-poll")
      return JSON.stringify(picking?.result ?? { state: "gone" });
    if (op === "pick-cancel") {
      post({ type: "hb-pick-cancel" });
      picking = null;
      return;
    }
    if (op !== "sync" || document.readyState === "loading") return;
    if (data.document !== undefined && data.document !== documentId) return;
    const page = data.page;
    saveError = data.error ?? null;
    if (pagePath && pagePath !== page.page) return;
    pagePath = page.page;
    frame = document.querySelector("iframe[data-hb-prototype]");
    if (!document.getElementById("hb-inline-style")) {
      const style = element("style", "");
      style.id = "hb-inline-style";
      style.textContent = window.__hbInlineStyle ?? "";
      document.head.prepend(style);
    }
    for (const ack of data.ack ?? []) {
      if ((pending.get(ack.input)?.sequence ?? Infinity) <= ack.sequence)
        pending.delete(ack.input);
    }
    let created = false;
    const declared = new Map(
      page.inputs
        .filter((item) => kinds.has(item.input.kind))
        .map((item) => [item.input.id, item.input]),
    );
    for (const [id, control] of controls) {
      const input = declared.get(id);
      if (!input || control.schema !== JSON.stringify(input)) retire(id);
    }
    for (const item of page.inputs) {
      const input = item.input;
      if (!kinds.has(input.kind)) continue;
      let control = controls.get(input.id);
      if (!control) {
        const target = frame
          ? document.body
          : document.getElementById(input.id);
        if (
          !target ||
          (!frame &&
            (!target.matches("div.hb-input[data-input]") ||
              target.dataset.input !== input.kind))
        )
          continue;
        control = build(input);
        controls.set(input.id, control);
        created = true;
        if (frame) {
          control.root.hidden = true;
          target.append(control.root);
        } else target.replaceChildren(control.root);
        sizeObserver.observe(control.root);
      }
      if (!pending.has(input.id)) {
        const value = item.answer?.value ?? null;
        values[input.id] = value;
        if (control.signature !== JSON.stringify(value)) control.paint(value);
      }
    }
    let status = document.getElementById("hb-inline-status");
    if (data.error) {
      if (!status) {
        status = element("p", "hb-inline-status");
        status.id = "hb-inline-status";
        status.setAttribute("role", "alert");
        document.body.append(status);
      }
      status.textContent = data.error;
    } else status?.remove();
    broadcast();
    if (created) layoutSizes();
    return true;
  };
  Object.defineProperty(window, "__hbInline", {
    value: api,
    configurable: true,
  });
})();
