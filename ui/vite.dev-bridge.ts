/**
 * Vite dev-server plugin: lets a normal browser run the UI against a real core.
 *
 * The Debug API refuses any request that carries an `Origin` header and requires a bearer token,
 * so a page cannot call it directly. This plugin serves `/__vw/*` on the dev server and forwards
 * each request to the Debug API with VW_DEBUG_API_TOKEN supplied by the person, or the plaintext
 * export of an opted-in disposable development fixture. VW_DEBUG_DESCRIPTOR names the listener.
 * Both are re-read on each request. The Debug API token never reaches the browser.
 *
 * Because the plugin holds the token, it must not become a way for another web page to drive the
 * core. A private per-run URL printed in the dev terminal exchanges a random credential for an
 * HttpOnly, SameSite=Strict cookie. Public HTML/module requests never receive that credential.
 * Every proxy request requires the cookie, including EventSource. It must also be same-origin:
 * `Host` is the dev server's own, `Origin`
 * (when present) is the dev server's own, and `Sec-Fetch-Site` (when sent) is `same-origin`.
 * Anything else is a 403 before the Debug API is contacted.
 *
 * Development only (`apply: "serve"`); a production build contains none of this. Depends on:
 * node crypto/http/fs. Called by: vite.config.ts.
 */
import { randomBytes } from "node:crypto";
import { readFileSync } from "node:fs";
import { request as httpRequest, type IncomingMessage, type ServerResponse } from "node:http";

import type { Plugin } from "vite";

const PREFIX = "/__vw/";
const COOKIE = "vw-dev-bridge";
const ENTRY_PARAM = "__vw_dev";

interface Descriptor {
  port: number;
  token: string;
}

function readDescriptor(path: string): Descriptor | null {
  try {
    const json = JSON.parse(readFileSync(path, "utf8")) as { port?: unknown; tokenFile?: unknown };
    if (typeof json.port !== "number") return null;
    const token =
      process.env["VW_DEBUG_API_TOKEN"]?.trim() ??
      (typeof json.tokenFile === "string" ? readFileSync(json.tokenFile, "utf8").trim() : "");
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

/** Browser metadata defense, independent of the required client credential. */
export function sameOrigin(req: IncomingMessage, navigation = false): boolean {
  const host = req.headers.host;
  if (host === undefined) return false;
  const origin = req.headers.origin;
  if (origin !== undefined && origin !== `http://${host}`) return false;
  const site = req.headers["sec-fetch-site"];
  // Opening the private terminal link is a top-level navigation with Sec-Fetch-Site: none.
  // Proxy requests never get that exception, even with the cookie.
  if (site !== undefined && site !== "same-origin" && !(navigation && site === "none"))
    return false;
  // A dev server on loopback only answers its own names (DNS rebinding defence).
  return /^(127\.0\.0\.1|localhost)(:\d+)?$/.test(host);
}

/** The plugin; inert unless `VW_DEBUG_DESCRIPTOR` names a descriptor file. */
export function devBridge(): Plugin {
  return {
    name: "vw-dev-bridge",
    apply: "serve",
    configureServer(server) {
      const credential = randomBytes(32).toString("hex");
      const cookie = `${COOKIE}=${credential}`;
      const printUrls = server.printUrls.bind(server);
      server.printUrls = () => {
        printUrls();
        const local = server.resolvedUrls?.local[0];
        if (local !== undefined && process.env["VW_DEBUG_DESCRIPTOR"] !== undefined) {
          const entry = new URL(local);
          entry.searchParams.set(ENTRY_PARAM, credential);
          server.config.logger.info(`Browser dev UI: ${entry.href}`);
        }
      };
      server.middlewares.use((req, res, next) => {
        const url = req.url ?? "";
        if (!url.startsWith(PREFIX)) {
          // Only the private terminal link can issue the cookie. An unauthenticated local
          // client can fetch the public page, but cannot bootstrap Debug API authority from it.
          const entry = new URL(url, "http://localhost");
          if (
            req.method === "GET" &&
            entry.pathname === "/" &&
            entry.searchParams.get(ENTRY_PARAM) === credential &&
            sameOrigin(req, true)
          ) {
            res.setHeader("set-cookie", `${cookie}; Path=${PREFIX}; HttpOnly; SameSite=Strict`);
            res.setHeader("cache-control", "no-store");
            res.setHeader("referrer-policy", "no-referrer");
            entry.searchParams.delete(ENTRY_PARAM);
            res.writeHead(303, { location: `${entry.pathname}${entry.search}` });
            res.end();
            return;
          }
          next();
          return;
        }
        const authenticated = req.headers.cookie
          ?.split(";")
          .some((value) => value.trim() === cookie);
        if (!authenticated || !sameOrigin(req)) {
          refuse(res, 403, "open the private Browser dev UI link printed by this dev server");
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
