/**
 * Read-only views over Vibe's history entries (`PublicHistoryEntry`, `vibe/app_server/models.py:959-1082`)
 * as the UI holds them: untyped JSON from the core's session store. Every accessor narrows at the
 * boundary and falls back to a neutral value, so an entry of a newer Vibe (unknown `type`, `kind` or
 * `status`) is shown as a generic row instead of throwing ("unknown kinds must render as a
 * generic step, never crash").
 * Depends on: protocol/core (`JsonValue`). Called by: the transcript model (`build.ts`, `callbacks.ts`,
 * `diff.ts`, `todos.ts`) and the row views.
 * Invariants: pure; never mutates its input; labels come from Vibe (`detail.display`, `state.display`),
 * never invented here.
 */
import { connectorToolLabel } from "../../connectors/model/meta";
import type { JsonValue } from "../../../protocol/core";

/** A JSON object as the store holds it. */
export type Obj = Readonly<Record<string, JsonValue | undefined>>;

/** `value` when it is a JSON object, else `null`. */
export function obj(value: JsonValue | undefined): Obj | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : null;
}

/** `value` when it is a string, else `null` (an empty string stays a string). */
export function str(value: JsonValue | undefined): string | null {
  return typeof value === "string" ? value : null;
}

/** `value` when it is a finite number, else `null`. */
export function num(value: JsonValue | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/** `value` when it is an array, else an empty array. */
export function arr(value: JsonValue | undefined): readonly JsonValue[] {
  return Array.isArray(value) ? value : [];
}

/** The entry's `id` ("" for a malformed entry, which the model skips). */
export function entryId(entry: Obj): string {
  return str(entry["id"]) ?? "";
}

/** The entry's `type` tag (`message`, `effect`, ...; anything else is "unknown" to the model). */
export function entryType(entry: Obj): string {
  return str(entry["type"]) ?? "";
}

/** The turn the entry belongs to, or `null` (shell runs, checkpoints). */
export function turnIdOf(entry: Obj): string | null {
  const id = str(entry["turnId"]);
  return id === null || id === "" ? null : id;
}

/** `createdAt` in epoch milliseconds (0 when absent). */
export function createdAt(entry: Obj): number {
  return num(entry["createdAt"]) ?? 0;
}

/** True while Vibe is still generating the entry (`generationStatus: in_progress`). */
export function inProgress(entry: Obj): boolean {
  return entry["generationStatus"] === "in_progress";
}

/** A message entry's role (`user`, `assistant`, `system`), or `null` for any other entry. */
export function roleOf(entry: Obj): string | null {
  return entryType(entry) === "message" ? str(entry["role"]) : null;
}

/** The plain text of a message: its text content blocks joined (Vibe sends one block per message). */
export function messageText(entry: Obj): string {
  return arr(entry["content"])
    .map((block) => {
      const b = obj(block);
      return b !== null && b["type"] === "text" ? (str(b["text"]) ?? "") : "";
    })
    .join("");
}

/** The effect's `detail` object (tool call), or an empty object. */
export function detailOf(entry: Obj): Obj {
  return obj(entry["detail"]) ?? {};
}

/** The entry's `state` object, or an empty object. */
export function stateOf(entry: Obj): Obj {
  return obj(entry["state"]) ?? {};
}

/** The effect/callback `detail.kind` (`shell`, `file_edit`, `approval`, ...), "" when absent. */
export function kindOf(entry: Obj): string {
  return str(detailOf(entry)["kind"]) ?? "";
}

/** The `state.status` tag (`running`, `completed`, `open`, `answered`, ...), "" when absent. */
export function statusOf(entry: Obj): string {
  return str(stateOf(entry)["status"]) ?? "";
}

/** Effect statuses that mean "still happening". */
const LIVE_STATUSES = new Set(["pending", "running", "blocked"]);

/** True while an effect has not settled (`pending`, `running`, `blocked`, or the entry in progress). */
export function effectLive(entry: Obj): boolean {
  return LIVE_STATUSES.has(statusOf(entry)) || (statusOf(entry) === "" && inProgress(entry));
}

/** What one effect step shows, all taken from Vibe's own display fields. */
export interface EffectView {
  kind: string;
  toolName: string;
  status: string;
  live: boolean;
  /** Verb to show: the running verb while live, the settled one after (`Reading` / `Read`). */
  verb: string;
  /** Object of the verb (`hello.txt`, the command), from Vibe. */
  target: string;
  /** Vibe's short status line (`Running command`), shown in the group header while live. */
  statusText: string;
  /** `state.display.success`; `null` while live or when Vibe gave none. */
  success: boolean | null;
  warnings: readonly string[];
  approvalNote: string | null;
  /** Who approved it (`user`, `config`, `smart`, `bypass`, `never`), when Vibe says. */
  approvalSource: string | null;
  decision: string | null;
  durationMs: number | null;
  /** Error text of a failed or cancelled effect. */
  error: string | null;
}

interface Label {
  verb: string;
  target: string;
}

function liveLabel(call: Obj): Label {
  return {
    verb: str(call["verb"]) ?? "",
    target: str(call["message"]) || str(call["summary"]) || "",
  };
}

/**
 * The settled label. Vibe pairs `state.display.verb` with `state.display.message` when it has a
 * result-specific line ("Read" + "1 line from hello.txt"); when the settled verb is empty (shell runs,
 * failures: "" + "Ran echo b") the call's `settledVerb` / `settledMessage` pair is used ("Ran" + "echo
 * b"). A target that repeats the verb ("Started" + "Started explore-files") loses the repeat.
 */
function settledLabel(call: Obj, settled: Obj | null): Label {
  const settledVerb = str(settled?.["verb"]) ?? "";
  let verb: string;
  let target: string;
  if (settledVerb !== "") {
    verb = settledVerb;
    target = str(settled?.["message"]) || str(call["settledMessage"]) || str(call["message"]) || "";
  } else {
    verb = str(call["settledVerb"]) || str(call["verb"]) || "";
    target = str(call["settledMessage"]) || str(call["message"]) || str(call["summary"]) || "";
  }
  if (verb !== "" && target.startsWith(`${verb} `)) target = target.slice(verb.length + 1);
  return { verb, target };
}

/** Builds the step view of an effect entry. */
export function effectView(entry: Obj): EffectView {
  const detail = detailOf(entry);
  const state = stateOf(entry);
  const call = obj(detail["display"]) ?? {};
  const settled = obj(state["display"]);
  const live = effectLive(entry);
  const status = statusOf(entry);
  const label = live ? liveLabel(call) : settledLabel(call, settled);
  const toolName = str(detail["toolName"]) ?? str(entry["title"]) ?? "";
  // unified Vibe labels connector tool steps with the raw name (`connector_linear.list_issues`);
  // show "Linear: list issues" instead.
  const { verb } = label;
  const target = connectorToolLabel(toolName) ?? label.target;
  const success = settled === null ? null : settled["success"] === true;
  const errorObj = obj(state["error"]);
  const error =
    status === "failed" || status === "cancelled"
      ? (str(errorObj?.["message"]) ?? str(state["reason"]) ?? str(settled?.["message"]) ?? status)
      : null;
  return {
    kind: str(detail["kind"]) ?? "",
    toolName,
    status,
    live,
    verb: verb === "" ? (str(entry["title"]) ?? "Step") : verb,
    target,
    statusText: str(call["statusText"]) ?? "",
    success,
    warnings: arr(settled?.["warnings"]).filter((w): w is string => typeof w === "string"),
    approvalNote: str(settled?.["approvalNote"]),
    approvalSource: str(state["approvalSource"]),
    decision: str(state["decision"]),
    durationMs: num(state["durationMs"]),
    error,
  };
}

/** The text output Vibe reports for an effect (`state.outputText`), "" when none. */
export function outputText(entry: Obj): string {
  return str(stateOf(entry)["outputText"]) ?? "";
}

/** The structured `state.output` object of an effect, or `null`. */
export function outputOf(entry: Obj): Obj | null {
  return obj(stateOf(entry)["output"]);
}

/** The structured `detail.input` object of an effect, or `null`. */
export function inputOf(entry: Obj): Obj | null {
  return obj(detailOf(entry)["input"]);
}

/** One image of a user message, ready for an `<img>`: only `data:` URLs, never files or remote URLs. */
export interface MessageImage {
  src: string;
  alt: string;
}

/** Image types the transcript shows inline (SVG excluded: it is a document, not a picture). */
const IMAGE_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);
/** Base64 alphabet only, so a crafted value cannot break out of the data URL. */
const BASE64 = /^[A-Za-z0-9+/]+={0,2}$/;
/** Largest inline image shown (base64 characters, about 15 MB of image). */
const MAX_IMAGE_CHARS = 20_000_000;

function dataUrl(mime: string | null, base64: string | null): string | null {
  if (mime === null || base64 === null || !IMAGE_TYPES.has(mime.toLowerCase())) return null;
  if (base64.length > MAX_IMAGE_CHARS || !BASE64.test(base64)) return null;
  return `data:${mime.toLowerCase()};base64,${base64}`;
}

/**
 * The images of a message: `image` blocks with an inline source and `resource` blocks holding a
 * `UserBlobResource` of an image type. A file-path source is never turned into a URL (no `file://`,
 * no asset protocol); it is listed by name in `fileImages` instead.
 */
export function messageImages(entry: Obj): { images: MessageImage[]; fileImages: string[] } {
  const images: MessageImage[] = [];
  const fileImages: string[] = [];
  for (const block of arr(entry["content"])) {
    const b = obj(block);
    if (b === null) continue;
    if (b["type"] === "image") {
      const attachment = obj(b["attachment"]) ?? {};
      const source = obj(attachment["source"]) ?? {};
      const alt = str(attachment["alias"]) ?? "image";
      if (source["kind"] === "inline") {
        const src = dataUrl(str(attachment["mimeType"]), str(source["data"]));
        if (src !== null) images.push({ src, alt });
      } else fileImages.push(alt);
    } else if (b["type"] === "resource") {
      const resource = obj(b["resource"]) ?? {};
      if (resource["kind"] !== "blob") continue;
      const src = dataUrl(str(resource["mediaType"]), str(resource["blob"]));
      if (src !== null) images.push({ src, alt: str(resource["uri"]) ?? "image" });
    }
  }
  return { images, fileImages };
}

/**
 * The text the user saw when sending (`userDisplayContent`; the model gets `content`,
 * the display keeps what was typed), or `null` when Vibe sent no display text.
 */
export function displayText(entry: Obj): string | null {
  const display = obj(entry["userDisplayContent"]);
  const parts = arr(display?.["content"])
    .map((item) => {
      const part = obj(item);
      return part?.["type"] === "text" ? str(part["text"]) : null;
    })
    .filter((t): t is string => t !== null);
  return parts.length > 0 ? parts.join("") : null;
}

/**
 * Names of context resources sent with a message (text and link resources, not images). The composer's
 * own chips carry their label in `userDisplayContent` as `{type: "vw.context", label}` items; those
 * win over the resource URIs (`vw://context/<n>`).
 */
export function contextNames(entry: Obj): string[] {
  const labels = arr(obj(entry["userDisplayContent"])?.["content"]).flatMap((item) => {
    const part = obj(item);
    const label = part?.["type"] === "vw.context" ? str(part["label"]) : null;
    return label === null || label === "" ? [] : [label];
  });
  if (labels.length > 0) return labels;
  return arr(entry["content"]).flatMap((block) => {
    const b = obj(block);
    const resource = obj(b?.["resource"]);
    if (b?.["type"] !== "resource" || resource === null || resource["kind"] === "blob") return [];
    return [str(resource["title"]) ?? str(resource["name"]) ?? str(resource["uri"]) ?? "context"];
  });
}

/**
 * True when the user said this message in voice chat: its display content carries a `vw.voice` item
 * (the composer's voice turns); the transcript marks it "voice".
 */
export function spokenMessage(entry: Obj): boolean {
  return arr(obj(entry["userDisplayContent"])?.["content"]).some(
    (item) => obj(item)?.["type"] === "vw.voice",
  );
}
