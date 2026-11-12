// Document-start isolated guards for the Chromium fixture, including out-of-process child frames.
// Uses its disposable DevTools endpoint; page.mjs calls it before navigating to any test document.
import { devtoolsTargets } from "./devtools_targets.mjs";
export async function installPointGuard(port, source, worldName) {
  const targets = await devtoolsTargets(port);
  const socket = new WebSocket(targets.find(target => target.type === "page").webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
  let next = 0;
  const pending = new Map();
  const send = (sessionId, method, params = {}) => new Promise((resolve, reject) => {
    const id = ++next;
    pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
  });
  const install = async session => {
    await send(session, "Page.enable");
    await send(session, "Page.addScriptToEvaluateOnNewDocument", { source, ...(worldName ? { worldName } : {}), runImmediately: true });
    await send(session, "Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: true, flatten: true,
      filter: [{ type: "iframe", exclude: false }] });
    if (session) await send(session, "Runtime.runIfWaitingForDebugger");
  };
  socket.onmessage = event => {
    const message = JSON.parse(event.data);
    if (pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(message.error.message));
      else resolve(message.result);
    } else if (message.method === "Target.attachedToTarget") {
      void install(message.params.sessionId).catch(error => {
        process.stderr.write(`isolated fixture initialization failed: ${error}\n`);
      });
    }
  };
  await install("");
  return socket;
}
