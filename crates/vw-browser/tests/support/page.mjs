// A real Chromium page behind the Rust browser test engine's JSON-lines pipe.
// Depends on the UI's playwright-core installation; called by support/mod.rs.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { createInterface } from "node:readline";
import { createServer } from "node:net";

const { chromium } = createRequire(process.env.VW_BROWSER_UI_PACKAGE)("playwright-core");
// A raw DevTools connection to the page target (as WebView2's own protocol channel, with flat child sessions),
// used only when the Rust engine evaluates frames through the production isolated-world registry.
const debugPort = await new Promise(resolve => { const s = createServer().listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => resolve(p)); }); });
// Site isolation as WebView2 ships it: cross-site frames run in processes (DevTools sessions) of their own.
const browser = await chromium.launch({ args: [`--remote-debugging-port=${debugPort}`, "--site-per-process"] });
const page = await browser.newPage({ viewport: { width: 300, height: 200 } });
let raw = null;
const rawEvents = [];
const rawPending = new Map();
let rawNext = 0;
const rawConnect = async () => {
  if (raw) return raw;
  // The fixture's only page (Playwright opens no startup window).
  const pages = (await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json()).filter(t => t.type === "page");
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
        await Promise.all(page.frames().map(frame => frame.evaluate(request.script).catch(() => null)));
      } else if (request.op === "frameCoverReady") {
        await page.waitForFunction("window.coverReady === true");
      } else if (request.op === "framesReady") {
        await page.waitForSelector("iframe");
        await page.waitForTimeout(200);
      } else if (request.op === "viewport") {
        await page.setViewportSize({width:request.width,height:request.height});
      } else if (request.op === "screenshot") {
        value = (await page.screenshot()).toString("base64");
      } else if (request.op === "click") {
        await page.mouse.click(request.x, request.y);
      }
      process.stdout.write(`${JSON.stringify({ value })}\n`);
    } catch (error) {
      process.stdout.write(`${JSON.stringify({ error: String(error) })}\n`);
    }
  }
} finally {
  await browser.close();
}
