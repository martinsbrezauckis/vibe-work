/** Check the app's resolved video features against the built sidecar's real MCP catalog and worker payload.
 * Depends on Cargo and native sidecars; called by release builders before accepting their binaries.
 */
import { spawnSync } from "node:child_process";
import { statSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../..", import.meta.url));
const { values } = parseArgs({
  options: { sidecar: { type: "string" }, target: { type: "string" } },
});
if (!values.sidecar || !values.target)
  throw new Error(
    "usage: check_video_features.mjs --sidecar PATH --target TRIPLE",
  );
const sidecar = resolve(values.sidecar);
const target = values.target;
const linux = target.includes("linux");
const supported =
  linux || target.includes("windows") || target.includes("apple-darwin");
if (!supported) throw new Error(`unsupported release target: ${target}`);
const graph = spawnSync(
  "cargo",
  [
    "tree",
    "--locked",
    "-p",
    "app",
    "--target",
    target,
    "--edges",
    "normal",
    "--depth",
    "1",
    "--prefix",
    "none",
    "--format",
    "{p}|{f}",
  ],
  { cwd: root, encoding: "utf8", windowsHide: true, timeout: 60_000 },
);
if (graph.error) throw graph.error;
if (graph.status !== 0)
  throw new Error(`app feature graph failed: ${graph.stderr}`);
const core = graph.stdout
  .split("\n")
  .find((line) => line.startsWith("vw-core-api "));
if (!core) throw new Error("app feature graph has no core");
const features = new Set(core.split("|")[1].split(","));
const enabled =
  features.has("video") && (!linux || features.has("linux-video"));
const input =
  [
    {
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-11-25",
        capabilities: {},
        clientInfo: { name: "release-check", version: "1" },
      },
    },
    { jsonrpc: "2.0", id: 2, method: "tools/list", params: {} },
  ]
    .map((value) => JSON.stringify(value))
    .join("\n") + "\n";
const reply = spawnSync(sidecar, ["mcp"], {
  input,
  encoding: "utf8",
  windowsHide: true,
  timeout: 10_000,
  maxBuffer: 4 * 1024 * 1024,
});
if (reply.error) throw reply.error;
if (reply.status !== 0)
  throw new Error(`sidecar catalog failed: ${reply.stderr}`);
const tools = reply.stdout
  .trim()
  .split("\n")
  .map((line) => JSON.parse(line))
  .find((value) => value.id === 2)?.result?.tools;
if (!Array.isArray(tools))
  throw new Error("sidecar did not return its tool catalog");
const listed = tools.some((tool) => tool.name === "video_frames");
if (listed !== enabled)
  throw new Error(
    `release video mismatch: app ${enabled ? "on" : "off"}, sidecar ${listed ? "on" : "off"}`,
  );
if (enabled) {
  for (const name of [
    "vw-video-helper",
    ...(linux ? ["vw-video-sandbox"] : []),
  ]) {
    const file = join(
      dirname(sidecar),
      basename(sidecar).replace("vw-agent-tools", name),
    );
    const info = statSync(file);
    if (
      !info.isFile() ||
      (process.platform !== "win32" && !(info.mode & 0o111))
    )
      throw new Error(`video worker is not executable: ${file}`);
  }
}
console.log(
  `Release video: app ${enabled ? "on" : "off"}, sidecar ${listed ? "on" : "off"}; worker payload checked`,
);
