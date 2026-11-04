/** Guards Linux app TLS against OpenSSL linkage. Depends on Cargo and, for a binary, readelf.
 * Called by check.sh and release builders: node scripts/public/check_linux_tls.mjs [linux-binary].
 */
import { execFileSync } from "node:child_process";
import { REPO_ROOT } from "./lib_files.mjs";

const options = { cwd: REPO_ROOT, encoding: "utf8", maxBuffer: 8 * 1024 * 1024 };
const tree = execFileSync("cargo", ["tree", "--color", "never", "--locked", "-p", "app", "--target",
  "x86_64-unknown-linux-gnu", "--edges", "normal", "--prefix", "none", "--format", "{p}"], options);
// openssl-probe only locates the OS certificate store; it does not link OpenSSL.
const forbidden = new Set(["openssl", "openssl-sys", "native-tls", "tokio-native-tls"]);
const linked = new Set(tree.split("\n").map((line) => line.split(" ")[0]));
const errors = [...forbidden].filter((name) => linked.has(name))
  .map((name) => `Linux app dependency tree contains ${name}`);
const [binary, ...extra] = process.argv.slice(2);
if (extra.length) throw new Error("Usage: check_linux_tls.mjs [linux-binary]");
if (binary) {
  const dynamic = execFileSync("readelf", ["-d", binary], options);
  if (!dynamic.includes("(NEEDED)")) throw new Error("Expected a dynamically linked Linux app binary");
  for (const line of dynamic.split("\n")) {
    if (/\(NEEDED\).*\[lib(?:ssl|crypto)\b/.test(line)) {
      errors.push(`Linux app binary links OpenSSL: ${line.trim()}`);
    }
  }
}
if (errors.length) {
  for (const error of errors) console.error(error);
  process.exitCode = 1;
} else {
  console.log(`Linux app TLS: no OpenSSL in dependency tree${binary ? " or binary NEEDED entries" : ""}`);
}
