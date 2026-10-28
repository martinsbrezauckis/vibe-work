/**
 * Turns the text Vibe puts on a failed turn into a plain line or the provider's own details (controlled by the Settings switch "Show the provider's own error text").
 * Depends on: nothing. Called by: features/transcript/rows (SmallRows), errorDetail.
 * Invariants:
 * - Only text recognised as a model-provider failure is rewritten. Anything else (a tool failure, a
 *   limit, an unknown message) is shown as Vibe wrote it in both modes: hiding an unknown error would
 *   hide the problem.
 * - Legacy details never include the endpoint (it is this computer's own proxy address), the request
 *   payload summary or the body excerpt; they show the status code, Vibe's/the provider's message, the
 *   model and the request id, which is what support asks for. Vibe 3 SDK errors carry a status and body;
 *   their details preserve that body after the core view has scrubbed stored secrets.
 * - Vibe's texts are the ones in `vibe/core/llm/exceptions.py` (`BackendError._fmt`): a one-line
 *   invalid-key text, a one-line rate-limit text, an invalid-model text, and the generic block
 *   `LLM backend error [provider]` with `key: value` lines.
 */

/** What was recognised in a provider failure. */
export interface ProviderFailure {
  /** HTTP status, or `null` for a network failure or when none is given. */
  status: number | null;
  /** The status with its phrase ("401 Unauthorized"), or `null`. */
  statusLabel: string | null;
  /** Vibe's or the provider's message. */
  message: string;
  model: string | null;
  requestId: string | null;
}

/** The setting: a plain line, or the provider's status and message. */
export type ErrorDetailMode = "plain" | "details";

const GENERIC = /^LLM backend error \[([^\]]*)\]/;

function field(lines: string[], key: string): string | null {
  const prefix = `${key}:`;
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith(prefix)) {
      const value = trimmed.slice(prefix.length).trim();
      return value === "" || value === "N/A" ? null : value;
    }
  }
  return null;
}

/** Recognises a provider failure in Vibe's error text; `null` for anything else. */
export function parseProviderError(raw: string): ProviderFailure | null {
  const text = raw.trim();
  // mistralai/client/errors/sdkerror.py (Vibe 3): Content-Type is included for non-JSON responses.
  const sdk =
    /^(?:API error occurred|Unexpected response received): Status (\d{3})(?: Content-Type [^\r\n]*?)?\. Body: ([\s\S]*)$/.exec(
      text,
    );
  if (sdk !== null) {
    return {
      status: Number(sdk[1]),
      statusLabel: sdk[1] ?? null,
      message: sdk[2] ?? "",
      model: null,
      requestId: null,
    };
  }
  if (text.startsWith("Invalid API key")) {
    return {
      status: 401,
      statusLabel: "401 Unauthorized",
      message: text,
      model: null,
      requestId: null,
    };
  }
  if (text.startsWith("Rate limit exceeded")) {
    return {
      status: 429,
      statusLabel: "429 Too Many Requests",
      message: text,
      model: null,
      requestId: null,
    };
  }
  const model = /^Model '([^']+)' is not available on /.exec(text);
  if (model !== null) {
    const detail = /Provider message: (.*)$/m.exec(text);
    return {
      status: 400,
      statusLabel: "400 Bad Request",
      message: detail?.[1] ?? text.split("\n")[0] ?? text,
      model: model[1] ?? null,
      requestId: null,
    };
  }
  if (!GENERIC.test(text)) return null;
  const lines = text.split("\n");
  const statusLabel = field(lines, "status");
  const code = statusLabel === null ? null : Number.parseInt(statusLabel, 10);
  return {
    status: code !== null && Number.isFinite(code) ? code : null,
    statusLabel,
    message: field(lines, "provider_message") ?? field(lines, "reason") ?? "no message",
    model: field(lines, "model"),
    requestId: field(lines, "request_id"),
  };
}

/** The plain line for a failure: what happened and what to do, without codes. */
export function plainLine(failure: ProviderFailure): string {
  const status = failure.status;
  if (status === 401 || status === 403)
    return "Mistral did not accept the API key. Check the key in Settings.";
  if (status === 429) return "Mistral is limiting requests right now. Wait a moment and try again.";
  if (status !== null && status >= 500)
    return "Mistral had a problem on its side. Try again in a moment.";
  if (status === null)
    return "Vibe Work could not reach Mistral. Check the connection and try again.";
  if (status === 400 || status === 404 || status === 422)
    return "Mistral could not accept this request. The details switch in Settings shows its message.";
  return "The request to Mistral failed.";
}

/** The details line: the status code and the provider's message, then model and request id. */
export function detailLine(failure: ProviderFailure): string {
  const head = failure.statusLabel ?? "No status";
  const parts = [`${head}: ${failure.message}`];
  if (failure.model !== null) parts.push(`model ${failure.model}`);
  if (failure.requestId !== null) parts.push(`request ${failure.requestId}`);
  return parts.join(" · ");
}

/** What to show for Vibe's error `raw` in `mode`. */
export function errorText(raw: string, mode: ErrorDetailMode): string {
  const failure = parseProviderError(raw);
  if (failure === null) return raw;
  return mode === "details" ? detailLine(failure) : plainLine(failure);
}
