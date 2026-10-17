/**
 * What the composer sends: the intent of the typed text (`!cmd`, a GUI `/command`, or a turn) and the
 * protocol params of a turn, a steer and a queued turn, built from the generated Vibe types.
 * Depends on: protocol/generated (types only), model/slashNames. Called by: state/composerSend,
 * the `ui/composer` route, tests.
 * Invariants:
 * - **Sent context is never a command**: only the text the user typed in
 *   the composer is classified. App commands with attachments are held until the attachments are removed. A
 *   context item is always a `resource` block (`vw://context/<n>`, Vibe's `UserTextResource`);
 *   the model sees it and history keeps it, but it is never part of a text block, so Vibe never parses it as a
 *   skill or command either.
 * - Inserted context and images keep what the user saw in `userDisplayContent`, with our own
 *   `vw.context` items next to Vibe's `text` items.
 * - The UI sends image bytes inline (`source.kind: "inline"`). Core saves validated
 *   bytes under the session's project and adds their absolute path as a text resource.
 * - A voice turn carries the voice instruction as a `role: "context"` entry before the user entry
 *   (Vibe injects it into the model's context without a history entry), never inside the user's text; its display
 *   content adds a `vw.voice` item, the transcript's "voice" marker.
 * - Pure: no I/O, no clock, no randomness (ids come in as arguments).
 */
import type { JsonValue } from "../../../protocol/core";
import type {
  ContentBlock,
  SessionContentBlock,
  TurnContextInputEntry,
  TurnUserInputEntry,
  UserDisplayContent,
} from "../../../protocol/generated";
import { VOICE_INSTRUCTION } from "../../voice/voiceChatText";
import { parseLoopArgs } from "./loopArgs";
import { isClientCommand } from "./slashNames";

/** Text the user inserted with "Point Vibe at this", a quote or a page comment: data, never a command. */
export interface ContextItem {
  id: string;
  /** Short label shown on the chip ("Terminal line", "Page section"). */
  label: string;
  /** Where it came from (`terminal`, `browser`, `page`, `quote`, ...). */
  source: string;
  /** The quoted text. */
  text: string;
  /** Optional model-facing text when the preview text is longer or differs from the safe prompt. */
  modelText?: string;
  /** Inserted by a Debug API client (`ui/composer`): sends go out with the Debug API origin. */
  by?: "debug-api";
  /** The resource name to send it under (a read document: `vw://doc/<id>`); default `vw://context/<n>`. */
  uri?: string;
  /** Facts of a read document, shown on its chip and in the preview. */
  doc?: DocFacts;
}

/** What a document chip shows about the document its text came from. */
export interface DocFacts {
  /** "PDF", "Word document", ... */
  kindLabel: string;
  /** Pages read (OCR) or counted; `null` when not paged. */
  pages: number | null;
  /** Characters of the text. */
  chars: number;
  /** What was kept of a long document (`null`: all of it). */
  kept: { unit: "pages" | "lines"; kept: number; of: number } | null;
  /** The OCR model that read it (`null`: read on this computer). */
  model: string | null;
}

/** An image the user pasted, dropped or picked, as bytes (base64), never a path. */
export interface ImageItem {
  id: string;
  /** File name shown on the chip and sent as the attachment alias. */
  name: string;
  mimeType: string;
  /** Base64 of the file's bytes. */
  data: string;
  /** Decoded size in bytes. */
  bytes: number;
  /** Attached by a Debug API client (`ui/composer`): sends go out with the Debug API origin. */
  by?: "debug-api";
}

/** What pressing Enter means for the current composer content. */
export type Intent =
  | { kind: "empty" }
  | { kind: "shell"; command: string }
  | { kind: "command"; name: string; args: string }
  | { kind: "turn" };

/**
 * Classifies the composer content. `!cmd` runs a shell command; `/name args` is a GUI command when
 * `name` is one we handle (`slashNames.ts`), else a turn (skills and unknown commands go to Vibe as
 * text, like the CLI; so does a `/loop` line that is not one of its strict forms, `loopArgs.ts`). App commands
 * stay reserved with attachments or recovered text; the send path asks the user to remove attachments or
 * explicitly use recovered text as a command.
 */
export function classify(text: string, context: number, images: number, literal = false): Intent {
  const trimmed = text.trim();
  if (trimmed === "" && context === 0 && images === 0) return { kind: "empty" };
  // App commands are reserved on every path, including recovered text and attachments.
  const command = /^\/([A-Za-z][\w:.-]*)(?:\s+([\s\S]*))?$/.exec(trimmed);
  if (command?.[1] !== undefined && isClientCommand(command[1])) {
    const args = (command[2] ?? "").trim();
    // `/loop` is ours only in its strict forms; "/loop every weekday at 9 ..." goes to Vibe's model like the CLI's.
    if (command[1] === "loop" && parseLoopArgs(args) === null) return { kind: "turn" };
    return { kind: "command", name: command[1], args };
  }
  if (context > 0 || images > 0 || literal) return { kind: "turn" };
  if (trimmed.startsWith("!")) {
    const command = trimmed.slice(1).trim();
    return command === "" ? { kind: "turn" } : { kind: "shell", command };
  }
  return { kind: "turn" };
}

/** The `uri` of the n-th context item of a message. */
export function contextUri(index: number): string {
  return `vw://context/${String(index + 1)}`;
}

/** Content blocks for `turn/start` and `turn/steer`: typed text, then context, then images. */
export function turnContent(
  text: string,
  context: readonly ContextItem[],
  images: readonly ImageItem[],
): ContentBlock[] {
  const blocks: ContentBlock[] = [];
  if (text.trim() !== "") blocks.push({ type: "text", text });
  context.forEach((item, i) => {
    blocks.push({
      type: "resource",
      resource: { uri: item.uri ?? contextUri(i), kind: "text", text: item.modelText ?? item.text },
    });
  });
  for (const image of images) {
    blocks.push({
      type: "image",
      attachment: {
        source: { kind: "inline", data: image.data },
        alias: image.name,
        mimeType: image.mimeType,
      },
    });
  }
  return blocks;
}

/**
 * The display payload stored with the message when it carries context or images (`null` otherwise:
 * plain text needs none). Vibe keeps it and returns it in history.
 */
export function displayContent(
  text: string,
  context: readonly ContextItem[],
  images: readonly ImageItem[],
  voice = false,
): UserDisplayContent | null {
  if (context.length === 0 && images.length === 0 && !voice) return null;
  return {
    version: "1",
    host: "vibe-work",
    content: [
      ...(text.trim() === "" ? [] : [{ type: "text", text }]),
      ...context.map((item) => ({ type: "vw.context", label: item.label, source: item.source })),
      ...images.map((image) => ({ type: "vw.image", name: image.name })),
      ...(voice ? [{ type: "vw.voice" }] : []),
    ],
  };
}

/** Session content blocks for one queued user entry (`session/turn/enqueue`, `queue/replace`). */
function queuedContent(
  text: string,
  context: readonly ContextItem[],
  images: readonly ImageItem[],
): SessionContentBlock[] {
  const blocks: SessionContentBlock[] = [];
  if (text.trim() !== "") blocks.push({ type: "text", text });
  context.forEach((item, i) => {
    blocks.push({
      type: "embedded_resource",
      uri: item.uri ?? contextUri(i),
      text: item.modelText ?? item.text,
    });
  });
  for (const image of images) {
    blocks.push({
      type: "image",
      uri: `data:${image.mimeType};base64,${image.data}`,
      mediaType: image.mimeType,
      altText: image.name,
    });
  }
  return blocks;
}

/**
 * The entries of a queued turn: one user entry with our anchor id `entryId` (the gate keeps a clean id
 * and makes one when absent; a resend must send the same id, or Vibe sees "different input" for the
 * same idempotency key. A voice turn has the voice instruction as a context entry
 * first (Vibe 2.26.1 `models.py` `validate_turn_input_entries`: the user entry must be the last).
 */
export function queueEntries(
  text: string,
  context: readonly ContextItem[],
  images: readonly ImageItem[],
  entryId: string,
  voice = false,
): (TurnContextInputEntry | TurnUserInputEntry)[] {
  const display = displayContent(text, context, images, voice);
  return [
    ...(voice
      ? [
          {
            role: "context" as const,
            content: [{ type: "text" as const, text: VOICE_INSTRUCTION }],
          },
        ]
      : []),
    {
      role: "user",
      entryId,
      content: queuedContent(text, context, images),
      ...(display === null ? {} : { annotations: { "vibe.userDisplayContent": display } }),
    },
  ];
}

/**
 * A generated protocol value as the `JsonValue` the core command carries. Sound by construction: the
 * generated types describe JSON (strings, numbers, booleans, null, arrays, objects) and are built from
 * JSON-only values here; TypeScript cannot see that an interface without an index signature is JSON.
 */
export function asJson(value: object): JsonValue {
  return value;
}

/**
 * Vibe's own retry prompt (`vibe/utils/retry_prompt.py:6-17` @ Vibe 2.26.0, Apache-2.0, see NOTICE),
 * sent with `turn/start {injected: true}` exactly like the CLI's `/retry` (`app.py:3492-3510`). The
 * protocol has no retry method; `session/continue` resumes the most recent session instead.
 */
export function retryPrompt(instructions: string): string {
  let message =
    "The previous model stream ended before reaching its end. Continue the " +
    "response exactly where it stopped without repeating text already produced. " +
    "If no response text was produced, answer the pending user request normally.";
  const extra = instructions.trim();
  if (extra !== "") {
    message +=
      "\n\nFollow these additional instructions from the user while " + `continuing:\n${extra}`;
  }
  return `<vibe_warning>${message}</vibe_warning>`;
}
