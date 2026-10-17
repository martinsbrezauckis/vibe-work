/**
 * The command palette's result types (block the grouped search, extended by with messages and files).
 * Depends on: components/Icon (names). Called by: CommandPalette, PalettePreview, coreRows, the `ui/search` route.
 */
import { term } from "../../vocabulary";
import type { IconName } from "../../components/Icon";

/** Result types; paletteModel chooses the fixed order for content searches and explicit action searches. */
export type Kind = "session" | "message" | "file" | "setting" | "action";

/** Group headings, icons and the preview's type name. */
export const GROUPS: readonly { kind: Kind; label: string; icon: IconName; one: string }[] = [
  {
    kind: "session",
    label: "Sessions",
    icon: "message-square",
    one: term("workNames", "session").name,
  },
  { kind: "message", label: "Messages", icon: "message-square-text", one: "Message" },
  { kind: "file", label: "Files", icon: "file-text", one: "File" },
  { kind: "setting", label: "Settings", icon: "settings", one: "Setting" },
  { kind: "action", label: "Actions", icon: "zap", one: "Action" },
];

/** One row of the result list. */
export interface Result {
  id: string;
  kind: Kind;
  title: string;
  path: readonly string[];
  /** Shortcut or project shown on the right. */
  hint?: string | undefined;
  /** Text for the preview pane (and why it matched when the title does not show it). */
  detail: string;
  /** Where Enter goes, in words, for the preview. */
  opens: string;
  run: () => void;
}
