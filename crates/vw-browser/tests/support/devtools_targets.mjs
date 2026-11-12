// Read-only DevTools target discovery for disposable Chromium fixtures.
// Called by the guard bootstrap and raw frame channel; no input command is retried here.
export async function devtoolsTargets(port) {
  let last;
  const deadline = performance.now() + 5000;
  while (performance.now() < deadline) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/json/list`, {
        headers: { Connection: "close" },
        signal: AbortSignal.timeout(Math.max(1, Math.ceil(Math.min(1500, deadline - performance.now())))),
      });
      if (!response.ok) throw new Error(`DevTools discovery returned HTTP ${response.status}`);
      return await response.json();
    } catch (error) {
      last = error;
      await new Promise(resolve => setTimeout(resolve, Math.min(50, Math.max(0, deadline - performance.now()))));
    }
  }
  throw new Error(`Fixture DevTools discovery failed: ${last}; cause: ${last?.cause ?? "unavailable"}`, { cause: last });
}
