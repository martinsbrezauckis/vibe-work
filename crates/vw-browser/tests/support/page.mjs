// A real Chromium page behind the Rust browser test engine's JSON-lines pipe.
// Depends on the UI's playwright-core installation; called by support/mod.rs.
import { createRequire } from "node:module";
import { createInterface } from "node:readline";

const { chromium } = createRequire(process.env.VW_BROWSER_UI_PACKAGE)("playwright-core");
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 300, height: 200 } });
const fixture = `<!doctype html><title>Capture fixture</title>
<style>body { margin:0; width:1600px; height:1600px }
header { position:fixed; top:0; left:0; height:20px }
#bare { position:fixed; top:25px; left:0 }</style>
<header id="styled" style="color: red; opacity: 0.7 !important">Pinned</header>
<div id="bare">No inline style</div><div id="shadow"></div>
<script>document.querySelector('#shadow').attachShadow({mode:'open'}).innerHTML =
'<span id="inner" style="position:fixed;top:50px;left:0;color:blue">Shadow pin</span>';</script>`;
await page.route("**/*", (route) => route.fulfill({ body: fixture, contentType: "text/html" }));

try {
  for await (const line of createInterface({ input: process.stdin })) {
    try {
      const request = JSON.parse(line);
      let value = null;
      if (request.op === "mount") {
        await page.addInitScript(request.init);
        await page.goto(request.url);
      } else if (request.op === "navigate") {
        await page.goto(request.url);
      } else if (request.op === "evaluate") {
        value = await page.evaluate(request.script);
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
