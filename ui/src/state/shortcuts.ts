/**
 * Keyboard shortcut registry: features register `{id, keys, run}`; one capture-phase listener
 * dispatches and skips shortcuts while typing. The registry pattern is adapted from ShellX.
 * Depends on: nothing (the listener lives in `features/shell/useShellWiring.ts`). Called by: the shell
 * and feature routes (`registerShortcut`), tests.
 * Invariants:
 * - Letters, digits and punctuation match by physical key (`event.code`), so Ctrl+K works on any
 *   keyboard layout, including Cyrillic and Greek layouts.
 * - Modifiers match exactly: Ctrl+Shift+P never fires the Ctrl+P binding, and AltGr (Ctrl+Alt on
 *   Windows) never fires a Ctrl binding.
 * - Two bindings with the same combo are refused at registration, so a clash is found
 *   when the feature is wired, not when a user presses the keys.
 * - A binding with a modifier works inside text fields; one without does not (typing).
 * - Inside an integrated terminal (`.vw-terminal`) no binding fires unless it says
 *   `allowInTerminal`: every key, Ctrl+K and Ctrl+C included, belongs to the shell there.
 */

/** A registered shortcut. */
export interface ShortcutDef {
  /** Stable id (tests, help listing). */
  id: string;
  /** Combo like `Mod+K`, `Mod+Shift+P`, `Mod+,`, `Mod+\``. `Mod` is Ctrl, or Cmd on macOS. */
  keys?: string;
  /** One line for a help listing. */
  description: string;
  /** What to do. */
  run: () => void;
  /** Override the default (modifier combos fire in text fields, bare keys do not). */
  allowInInput?: boolean;
  /** Fire even while a terminal has focus (default false: the terminal gets every key). */
  allowInTerminal?: boolean;
  /** Consume reserved window keys even during input composition. */
  allowWhileComposing?: boolean;
  /** Keep consumed window keys from reaching controls with their own key handlers. */
  stopPropagation?: boolean;
  /** In a terminal, only keep the window from acting on the key: the terminal still gets it and `run` is skipped. */
  terminalGetsKey?: boolean;
}

interface Combo {
  mod: boolean;
  shift: boolean;
  alt: boolean;
  code: string;
}

const NAMED: Readonly<Record<string, string>> = {
  "`": "Backquote",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
  "\\": "Backslash",
  "[": "BracketLeft",
  "]": "BracketRight",
  "-": "Minus",
  "=": "Equal",
  ";": "Semicolon",
  "'": "Quote",
  space: "Space",
  enter: "Enter",
  escape: "Escape",
  esc: "Escape",
  tab: "Tab",
};

/** Parses a combo string; throws on anything it does not understand. */
export function parseKeys(keys: string): Combo {
  const parts = keys.split("+");
  const key = parts.pop();
  if (key === undefined || key === "") throw new Error(`bad shortcut ${JSON.stringify(keys)}`);
  const mods = new Set(parts.map((p) => p.toLowerCase()));
  for (const m of mods)
    if (!["mod", "shift", "alt"].includes(m)) throw new Error(`bad modifier ${m} in ${keys}`);
  const lower = key.toLowerCase();
  let code = NAMED[lower];
  if (code === undefined && /^[a-z]$/.test(lower)) code = `Key${lower.toUpperCase()}`;
  if (code === undefined && /^[0-9]$/.test(lower)) code = `Digit${lower}`;
  if (code === undefined && /^f([1-9]|1[0-9]|2[0-4])$/.test(lower)) code = lower.toUpperCase();
  if (code === undefined) throw new Error(`unknown key ${key} in ${keys}`);
  return { mod: mods.has("mod"), shift: mods.has("shift"), alt: mods.has("alt"), code };
}

/** True on macOS and iOS-like platforms (Cmd instead of Ctrl). */
export function detectMac(): boolean {
  const nav = typeof navigator === "undefined" ? undefined : navigator;
  const platform =
    (nav as (Navigator & { userAgentData?: { platform?: string } }) | undefined)?.userAgentData
      ?.platform ??
    nav?.platform ??
    "";
  return /mac|iphone|ipad/i.test(platform);
}

/** The combo for display: `Ctrl K`, `Ctrl Shift P` (macOS: `Cmd`). */
export function formatKeys(keys: string, mac = detectMac()): string {
  return keys
    .split("+")
    .map((p) =>
      p.toLowerCase() === "mod" ? (mac ? "Cmd" : "Ctrl") : p.length === 1 ? p.toUpperCase() : p,
    )
    .join(" ");
}

function matches(combo: Combo, event: KeyboardEvent, mac: boolean): boolean {
  const modDown = mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
  const noMod = !event.ctrlKey && !event.metaKey;
  return (
    event.code === combo.code &&
    (combo.mod ? modDown : noMod) &&
    event.shiftKey === combo.shift &&
    event.altKey === combo.alt
  );
}

/** True when keyboard input would go into a text field. */
function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.tagName === "INPUT" ||
    target.tagName === "TEXTAREA" ||
    target.tagName === "SELECT" ||
    target.isContentEditable
  );
}

/** True when keyboard input goes into an integrated terminal. */
function isInTerminal(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(".vw-terminal") !== null;
}

/** Creates a registry. `mac` is injectable for tests. */
export function createShortcuts(mac = detectMac()) {
  const defs = new Map<string, { def: ShortcutDef; combo: Combo | null }>();
  const comboId = (c: Combo) => `${String(c.mod)}|${String(c.shift)}|${String(c.alt)}|${c.code}`;
  return {
    /** Registers a shortcut; returns the function that removes it. Throws on a duplicate id or combo. */
    register(def: ShortcutDef): () => void {
      const combo = def.keys === undefined ? null : parseKeys(def.keys);
      if (defs.has(def.id)) throw new Error(`shortcut id ${def.id} is already registered`);
      for (const other of defs.values()) {
        if (combo !== null && other.combo !== null && comboId(other.combo) === comboId(combo)) {
          throw new Error(`${String(def.keys)} is already bound to ${other.def.id}`);
        }
      }
      defs.set(def.id, { def, combo });
      return () => {
        defs.delete(def.id);
      };
    },
    /** All registrations, in registration order. */
    list(): readonly ShortcutDef[] {
      return [...defs.values()].map((d) => d.def);
    },
    /** Runs the matching shortcut for a keydown. Returns true when one ran (the caller prevents default). */
    handle(event: KeyboardEvent): boolean {
      const target = event.composedPath()[0] ?? event.target;
      const typing = isEditable(target);
      const terminal = isInTerminal(target);
      for (const { def, combo } of defs.values()) {
        if (combo === null || !matches(combo, event, mac)) continue;
        if (event.isComposing && def.allowWhileComposing !== true) continue;
        if (terminal && def.allowInTerminal !== true) continue;
        if (typing && !(def.allowInInput ?? combo.mod)) continue;
        if (terminal && def.terminalGetsKey === true) return true;
        if (def.stopPropagation === true) event.stopPropagation();
        def.run();
        return true;
      }
      return false;
    },
  };
}

/** The registry the app uses. */
export type Shortcuts = ReturnType<typeof createShortcuts>;
