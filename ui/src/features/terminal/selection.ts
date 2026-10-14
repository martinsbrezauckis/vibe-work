/**
 * The selection API for "Point Vibe at this": the UI can read what the user selected in
 * the active terminal. Reading only; sending it to the agent is the job (and arrives as a quoted
 * message, never as a command).
 * Depends on: nothing. Called by: session.ts (registers the mounted terminal), terminalUiControl
 * (`selection` action), later.
 * Invariants: at most one terminal is registered (the one the panel shows); unregistering a
 * terminal that is no longer the registered one does nothing.
 */

/** What a mounted terminal offers. */
export interface TerminalHandle {
  /** The terminal id. */
  readonly terminalId: string;
  /** The selected text ("" when nothing is selected). */
  selection(): string;
  /** Selects the whole buffer. */
  selectAll(): void;
  /** Moves keyboard focus into the terminal. */
  focus(): void;
  /** True while the terminal has keyboard focus. */
  hasFocus(): boolean;
  /** The renderer xterm uses (`dom`), and the font size in px. */
  readonly info: () => { renderer: string; fontSize: number; cols: number; rows: number };
}

let current: TerminalHandle | null = null;

/** Registers the shown terminal; returns the function that unregisters it. */
export function registerTerminal(handle: TerminalHandle): () => void {
  current = handle;
  return () => {
    if (current === handle) current = null;
  };
}

/** The shown terminal, if any. */
export function shownTerminal(): TerminalHandle | null {
  return current;
}

/** The text selected in the shown terminal ("" when none). For "Point Vibe at this". */
export function readTerminalSelection(): string {
  return current?.selection() ?? "";
}
