// A real Chromium page behind the Rust browser test engine's JSON-lines pipe.
// Depends on the UI's playwright-core installation; called by support/mod.rs.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { createInterface } from "node:readline";
import { createServer } from "node:net";
import { installPointGuard } from "./point_guard.mjs";
import { devtoolsTargets } from "./devtools_targets.mjs";

const { chromium } = createRequire(process.env.VW_BROWSER_UI_PACKAGE)("playwright-core");
// A raw DevTools connection to the page target (as WebView2's own protocol channel, with flat child sessions),
// used only when the Rust engine evaluates frames through the production isolated-world registry.
const debugPort = await new Promise(resolve => { const s = createServer().listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => resolve(p)); }); });
// Site isolation as WebView2 ships it: cross-site frames run in processes (DevTools sessions) of their own.
// Exercise the Windows-default middle-button behavior even when this fixture runs on Linux.
const browser = await chromium.launch({ args: [`--remote-debugging-port=${debugPort}`, "--site-per-process", "--enable-blink-features=MiddleClickAutoscroll"] });
let captureDeviceScale = false;
const page = await browser.newPage({ viewport: { width: 300, height: 200 } });
let raw = null;
const rawEvents = [];
const rawPending = new Map();
let rawNext = 0;
const rawConnect = async () => {
  if (raw) return raw;
  // The fixture's only page (Playwright opens no startup window).
  const pages = (await devtoolsTargets(debugPort)).filter(t => t.type === "page");
  const target = pages.length === 1 ? pages[0] : null;
  if (!target) throw new Error("page target not found");
  raw = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { raw.onopen = resolve; raw.onerror = reject; });
  raw.onmessage = message => {
    const data = JSON.parse(message.data);
    if (data.id !== undefined && rawPending.has(data.id)) {
      rawPending.get(data.id)(data);
      rawPending.delete(data.id);
    } else if (data.method) {
      rawEvents.push({ session: data.sessionId || "", method: data.method, params: data.params || {} });
    }
  };
  return raw;
};
await page.addInitScript(readFileSync(new URL("../../../vw-platform/src/webview/frame_origin.js", import.meta.url), "utf8"));
const cdp = await page.context().newCDPSession(page);
await cdp.send("Page.enable");
const pointGuard = readFileSync(new URL("../../../vw-preview/src/js/point_guard.js", import.meta.url), "utf8");
const guardWorld = process.env.VW_TEST_CDP_FRAMES === "1" ? "vibe-work-agent"
  : process.env.VW_BROWSER_ISOLATED === "1" ? "checked-input-fixture" : null;
const guardConnection = await installPointGuard(debugPort, pointGuard, guardWorld);
const frameIds = new Map();
let nextFrameId = 0;
let generation = 0;
for (const event of ["frameattached", "framedetached", "framenavigated"]) {
  page.on(event, () => { generation++; });
}
const frameId = frame => {
  if (!frameIds.has(frame)) frameIds.set(frame, String(++nextFrameId));
  return frameIds.get(frame);
};
const fixture = `<!doctype html><title>Capture fixture</title>
<style>body { margin:0; width:1600px; height:1600px }
header { position:fixed; top:0; left:0; height:20px }
#bare { position:fixed; top:25px; left:0 }</style>
<header id="styled" style="color: red; opacity: 0.7 !important">Pinned</header>
<div id="bare">No inline style</div><div id="shadow"></div>
<script>document.querySelector('#shadow').attachShadow({mode:'open'}).innerHTML =
'<span id="inner" style="position:fixed;top:50px;left:0;color:blue">Shadow pin</span>';</script>`;
const frameFixture = `<title>Embedded login</title><form method="post">
<label>User <input id="user" autocomplete="username"></label>
<label>Password <input id="password" type="password"></label>
<label><input id="check" type="checkbox">Fixture checkbox</label>
<button id="send">Sign in</button></form>
<iframe src="https://nested.example.test/child"></iframe>`;
await page.route("**/*", (route) => route.fulfill({
  body: route.request().url().includes("consent.example.test") ? `<title>Privacy choices</title>
    <div role="dialog" aria-modal="true"><h1>Your privacy choices</h1>
    <p>We and our partners use cookies to support independent journalism.</p>
    <button id="accept">Accept all</button><button id="reject">Reject optional cookies</button>
    <button id="manage">Manage choices</button></div>`
    : route.request().url().includes("nested.example.test/new-guard") ? `<form><button style="position:fixed;inset:0;width:100%;height:100%">Send</button></form>
      <script>document.body.dataset.guardEffects='[]';
      const record=type=>{const effects=JSON.parse(document.body.dataset.guardEffects);effects.push(type);document.body.dataset.guardEffects=JSON.stringify(effects)};
      for(const type of ['pointerdown','mousedown','mouseup','click','auxclick','dblclick','contextmenu'])window.addEventListener(type,()=>record(type),true);
      document.querySelector('form').onsubmit=e=>{e.preventDefault();record('submit')};</script>`
    : route.request().url().includes("nested.example.test") ? '<button id="nested">Nested button</button>'
    : (route.request().url().includes("frame.example.test") || route.request().url().endsWith("/same-frame")) ? frameFixture : fixture,
  contentType: "text/html",
}));

try {
  for await (const line of createInterface({ input: process.stdin })) {
    try {
      const request = JSON.parse(line);
      let value = null;
      if (request.op === "publicPages") {
        await page.unroute("**/*");
      } else if (request.op === "mount") {
        await page.addInitScript(request.init);
        await page.goto(request.url);
      } else if (request.op === "navigate") {
        await page.goto(request.url);
      } else if (request.op === "evaluate") {
        value = await page.evaluate(request.script);
      } else if (request.op === "waitForFrame") {
        try {
          const frame = page.frames().find(frame => frame.url() === request.url)
            ?? await page.waitForEvent("framenavigated", {
              predicate: frame => frame.url() === request.url, timeout: 10000,
            });
          await frame.waitForLoadState("load", { timeout: 10000 });
          await frame.waitForSelector(request.selector, { state: "attached", timeout: 10000 });
        } catch (error) {
          throw new Error(`waiting for loaded frame ${request.url} containing ${request.selector}; observed frames: ${JSON.stringify(page.frames().map(frame => ({ url: frame.url(), detached: frame.isDetached() })))}; ${error}`, { cause: error });
        }
      } else if (request.op === "frames") {
        const frames = page.frames().filter(frame => !request.frame || frameId(frame) === request.frame);
        if (!frames.length) throw new Error("frame is stale or detached");
        const evaluate = async frame => {
          if (!request.isolated) return frame.evaluate(request.script);
          const locate = (parent, tree) => {
            if (parent === frame) return tree.frame.id;
            for (const [index, child] of parent.childFrames().entries()) {
              const found = tree.childFrames?.[index] && locate(child, tree.childFrames[index]);
              if (found) return found;
            }
            return null;
          };
          // With site isolation a cross-site or sandboxed frame lives in a process of its own: its frame tree belongs
          // to that frame's own DevTools session, not to the page's. Use the nearest frame (itself or an ancestor)
          // that has a session of its own.
          let session = cdp;
          let root = page.mainFrame();
          for (let f = frame; f && f !== page.mainFrame(); f = f.parentFrame()) {
            const own = await page.context().newCDPSession(f).catch(() => null);
            if (own) {
              session = own;
              root = f;
              break;
            }
          }
          try {
            const { frameTree } = await session.send("Page.getFrameTree");
            const nativeId = locate(root, frameTree);
            if (!nativeId) throw new Error("isolated frame is unavailable");
            const { executionContextId } = await session.send("Page.createIsolatedWorld", { frameId: nativeId, worldName: "checked-input-fixture" });
            const answer = await session.send("Runtime.evaluate", { expression: request.script, contextId: executionContextId, returnByValue: true });
            if (answer.exceptionDetails) throw new Error(JSON.stringify(answer.exceptionDetails));
            return answer.result.value;
          } finally {
            if (session !== cdp) await session.detach().catch(() => {});
          }
        };
        value = await Promise.all(frames.map(async frame => ({ id: frameId(frame), generation, main: frame === page.mainFrame(), url: frame.url(),
          origin: await frame.evaluate("window.__vwNativeFrameOrigin"), value: JSON.stringify(await evaluate(frame)) })));
      } else if (request.op === "cdp") {
        const socket = await rawConnect();
        const id = ++rawNext;
        const answer = await new Promise(resolve => {
          rawPending.set(id, resolve);
          socket.send(JSON.stringify({ id, method: request.method, params: request.params || {},
            ...(request.session ? { sessionId: request.session } : {}) }));
        });
        // Events first, in the order the protocol sent them before this answer.
        value = { events: rawEvents.splice(0), result: answer.result ?? null, error: answer.error?.message ?? null };
      } else if (request.op === "framesPageWorld") {
        // Page script of every frame (what a hostile page can run), never the agent's world.
        value = await Promise.all(page.frames().map(frame => frame.evaluate(request.script).catch(() => null)));
      } else if (request.op === "frameCoverReady") {
        await page.waitForFunction("window.coverReady === true");
      } else if (request.op === "framesReady") {
        await page.waitForSelector("iframe");
        await page.waitForTimeout(200);
      } else if (request.op === "viewport") {
        await page.setViewportSize({width:request.width,height:request.height});
        captureDeviceScale = Boolean(request.scale);
        if (captureDeviceScale) await cdp.send("Emulation.setDeviceMetricsOverride", {
          width: request.width, height: request.height, deviceScaleFactor: request.scale, mobile: false,
        });
      } else if (request.op === "screenshot") {
        // The CDP session that owns the emulated metrics captures the same device pixels as a native view.
        value = captureDeviceScale ? (await cdp.send("Page.captureScreenshot", { format: "png" })).data :
          (await page.screenshot()).toString("base64");
      } else if (request.op === "keyboard") {
        const input = request.input;
        if (typeof input.text === "string") await cdp.send("Input.insertText", {text:input.text});
        else {
          const key = input.key === "Space" ? " " : input.key;
          const modifiers = ["Alt", "Control", "Meta", "Shift"].reduce((bits, name, index) => bits | (input.modifiers?.includes(name) ? 1 << index : 0), 0);
          const codes = {Backspace:8,Tab:9,Enter:13,Shift:16,Control:17,Alt:18,Escape:27,Space:32,PageUp:33,PageDown:34,End:35,Home:36,ArrowLeft:37,ArrowUp:38,ArrowRight:39,ArrowDown:40,Insert:45,Delete:46,Meta:91};
          const windowsVirtualKeyCode = codes[input.key] || (key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0);
          const text = key.length === 1 && (modifiers & 7) === 0 ? key : key === "Enter" ? "\r" : "";
          const code = /^[a-z]$/i.test(key) ? `Key${key.toUpperCase()}` : /^\d$/.test(key) ? `Digit${key}` : ["Control","Shift","Alt","Meta"].includes(key) ? `${key}Left` : input.key;
          await cdp.send("Input.dispatchKeyEvent", {type:"keyDown",key,code,modifiers,windowsVirtualKeyCode,text});
          await cdp.send("Input.dispatchKeyEvent", {type:"keyUp",key,code,modifiers,windowsVirtualKeyCode});
        }
      } else if (request.op === "click") {
        const options = request.options || { button: 0, count: 1, modifiers: 0 };
        const keys = ["Alt", "Control", "Meta", "Shift"].filter((_, i) => options.modifiers & (1 << i));
        for (const key of keys) await page.keyboard.down(key);
        try {
          await page.mouse.click(request.x, request.y, { button: ["left", "middle", "right"][options.button], clickCount: options.count });
        } finally {
          for (const key of keys.reverse()) await page.keyboard.up(key);
        }
      } else if (request.op === "pointerSequence") {
        // Match WebView2's windows_sequence.rs exactly: no preparatory mouse move or implicit button state.
        const options = request.options;
        for (let clickCount = 1; clickCount <= options.count; clickCount++) {
          for (const type of ["mousePressed", "mouseReleased"]) {
            await cdp.send("Input.dispatchMouseEvent", {
              type, x: request.x, y: request.y, button: ["left", "middle", "right"][options.button],
              clickCount, modifiers: options.modifiers,
            });
          }
        }
      } else if (request.op === "wheel") {
        await cdp.send("Input.dispatchMouseEvent", {type:"mouseWheel", x:request.x, y:request.y, deltaX:request.delta[0], deltaY:request.delta[1]});
      } else if (request.op === "pages") {
        value = page.context().pages().length;
      } else if (request.op === "hover") {
        await page.mouse.move(request.x, request.y);
      }
      process.stdout.write(`${JSON.stringify({ value })}\n`);
    } catch (error) {
      process.stdout.write(`${JSON.stringify({ error: String(error) })}\n`);
    }
  }
} finally {
  guardConnection.close();
  await browser.close();
}
