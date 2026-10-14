/**
 * Vite dev-server plugin: lets a normal browser run the UI against a real core.
 *
 * The Debug API refuses any request that carries an `Origin` header and requires a bearer token,
 * so a page cannot call it directly. This plugin serves `/__vw/*` on the dev server and forwards
 * each request to the Debug API with the token added from the descriptor file named by
 * `VW_DEBUG_DESCRIPTOR` and from the token file it names (both re-read on every request, so a
 * restarted host or a regenerated token works). The token never
 * reaches the browser.
 *
 * Because the plugin holds the token, it must not become a way for another web page to drive the
 * core. Every request must therefore be same-origin: `Host` is the dev server's own, `Origin`
 * (when present) is the dev server's own, and `Sec-Fetch-Site` (when sent) is `same-origin`.
 * Anything else is a 403 before the Debug API is contacted.
 *
 * Development only (`apply: "serve"`); a production build contains none of this. Depends on:
 * node http/fs. Called by: vite.config.ts.
 */
import { readFileSync } from "node:fs";
import { request as httpRequest, type IncomingMessage, type ServerResponse } from "node:http";

import type { Plugin } from "vite";

const PREFIX = "/__vw/";

interface Descriptor {
  port: number;
  token: string;
}

function readDescriptor(path: string): Descriptor | null {
  try {
    const json = JSON.parse(readFileSync(path, "utf8")) as { port?: unknown; tokenFile?: unknown };
    if (typeof json.port !== "number" || typeof json.tokenFile !== "string") return null;
    // The descriptor names the token file; read it per request so a regenerated token works.
    const token = readFileSync(json.tokenFile, "utf8").trim();
    return token === "" ? null : { port: json.port, token };
  } catch {
    return null;
  }
}

function refuse(res: ServerResponse, status: number, message: string): void {
  res.statusCode = status;
  res.setHeader("content-type", "application/json");
  res.end(JSON.stringify({ error: { code: "denied", message } }));
}

/** True when the request came from a page served by this dev server. */
export function sameOrigin(req: IncomingMessage): boolean {
  const host = req.headers.host;
  if (host === undefined) return false;
  const origin = req.headers.origin;
  if (origin !== undefined && origin !== `http://${host}`) return false;
  const site = req.headers["sec-fetch-site"];
  if (site !== undefined && site !== "same-origin") return false;
  // A dev server on loopback only answers its own names (DNS rebinding defence).
  return /^(127\.0\.0\.1|localhost)(:\d+)?$/.test(host);
}

/** The plugin; inert unless `VW_DEBUG_DESCRIPTOR` names a descriptor file. */
export function devBridge(): Plugin {
  return {
    name: "vw-dev-bridge",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const url = req.url ?? "";
        if (!url.startsWith(PREFIX)) {
          next();
          return;
        }
        const descriptorPath = process.env["VW_DEBUG_DESCRIPTOR"];
        if (descriptorPath === undefined) {
          refuse(
            res,
            503,
            "set VW_DEBUG_DESCRIPTOR to the Debug API descriptor file to use browser dev mode",
          );
          return;
        }
        if (!sameOrigin(req)) {
          refuse(res, 403, "the dev bridge only serves pages of this dev server");
          return;
        }
        const descriptor = readDescriptor(descriptorPath);
        if (descriptor === null) {
          refuse(
            res,
            503,
            "the Debug API descriptor is missing or unreadable (is the core running?)",
          );
          return;
        }
        const headers: Record<string, string> = {
          host: `127.0.0.1:${String(descriptor.port)}`,
          authorization: `Bearer ${descriptor.token}`,
        };
        for (const name of ["content-type", "content-length", "accept"]) {
          const value = req.headers[name];
          if (typeof value === "string") headers[name] = value;
        }
        const upstream = httpRequest(
          {
            host: "127.0.0.1",
            port: descriptor.port,
            method: req.method ?? "GET",
            path: `/${url.slice(PREFIX.length)}`,
            headers,
          },
          (answer) => {
            res.writeHead(answer.statusCode ?? 502, answer.headers);
            answer.pipe(res);
          },
        );
        upstream.on("error", () => {
          if (!res.headersSent) refuse(res, 502, "the Debug API did not answer");
          else res.end();
        });
        res.on("close", () => {
          upstream.destroy();
        });
        req.pipe(upstream);
      });
    },
  };
}
