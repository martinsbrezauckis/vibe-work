/** Names and counts for session-log scan notices. Depends on typed scan views; used by settings and the bell. */
import type { SecretLeakFinding, SecretLeakName, SecretScanView } from "../protocol/core";

const count = (n: number, noun: string) => `${String(n)} ${noun}${n === 1 ? "" : "s"}`;

/** Locale time for the last completed scan, with today's date kept short. */
export function scanStatus(view: SecretScanView): string | null {
  const status = statusText(view);
  if (view.status === "off" || view.skippedSecrets === 0) return status;
  return `${status === null ? "" : `${status} `}· skips ${String(view.skippedSecrets)} simple secrets`;
}

function statusText(view: SecretScanView): string | null {
  switch (view.status) {
    case "off":
      return null;
    case "idle":
      return null;
    case "checking":
      return "Checking your session logs…";
    case "locked":
      return "Waits for the Vault to be unlocked.";
    case "noSecrets":
      return "Nothing to look for: no Vault secrets strong enough yet.";
    case "error":
      return view.failedFiles > 0
        ? `Could not check ${count(view.failedFiles, "file")}. Checked the rest.`
        : "Could not check your session logs. Try Check now again.";
    case "checked": {
      if (view.failedFiles > 0)
        return `Could not check ${count(view.failedFiles, "file")}. Checked the rest.`;
      if (view.checkedMs === null) return null;
      const date = new Date(Number(view.checkedMs));
      const time = date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
      const when =
        date.toDateString() === new Date().toDateString()
          ? `today ${time}`
          : `${date.toLocaleDateString()} ${time}`;
      return `Last checked ${when} · ${view.places === 0 ? "nothing found" : `found ${count(view.places, "place")} in ${count(view.chats, "chat")}`}`;
    }
  }
}

function matches(finding: SecretLeakFinding, names: SecretLeakName[], removed: boolean): string {
  if (names.length === 0) return "";
  const places =
    names.length === finding.secrets.length
      ? finding.places
      : names.reduce((n, name) => n + name.places, 0);
  const verb = removed ? "Removed" : "Found";
  const where = `${removed ? "from" : "in"} the chat “${finding.chatTitle}” (${count(places, "place")}).`;
  const name = names[0];
  if (names.length === 1 && name) {
    if (name.websitePassword && !removed) return `Found your password for “${name.name}” ${where}`;
    return `${verb} your secret “${name.name}” ${where}`;
  }
  return `${verb} ${String(names.length)} of your secrets ${where}`;
}

/** One retained entry per chat, including keys removed while a password still needs a decision. */
export function findingText(finding: SecretLeakFinding): string {
  if (finding.state === "chatOpen") return "Close this chat first, then Redact.";
  if (finding.state === "partial") {
    const prefix = `Removed your secret from ${String(finding.redactedFiles)} of ${count(finding.files, "file")}.`;
    if (finding.formatFiles > 0)
      return `${prefix} Some files could not be changed without damaging the chat.`;
    if (finding.unreadableFiles > 0)
      return `${prefix} Some files could not be replaced. Try Redact again.`;
    return `${prefix} ${finding.changedFiles === 1 ? "The other file changed since the check; it will" : "The other files changed since the check; they will"} be checked again.`;
  }
  if (finding.state === "removed") return matches(finding, finding.secrets, true);
  return [
    matches(
      finding,
      finding.secrets.filter((name) => name.state !== "removed"),
      false,
    ),
    matches(
      finding,
      finding.secrets.filter((name) => name.state === "removed"),
      true,
    ),
  ]
    .filter(Boolean)
    .join(" ");
}
