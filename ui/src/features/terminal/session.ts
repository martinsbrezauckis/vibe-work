/**
 * One mounted terminal view: the xterm instance, its addons, the output stream from the core, the
 * input queue to it, resizing, clipboard keys and the trusted-surface rules. Plain TypeScript so the
 * React component stays a mount point.
 * The xterm, fit and links addons mount together, and fit-on-resize is debounced. The core owns the terminal, so unmounting only detaches the view.
 * Adapted from ShellX.
 * Depends on: @xterm/xterm and addons, store.ts, inputQueue.ts, theme.ts, links.ts, selection.ts,
 * xtermCsp.ts. Called by: TerminalView.tsx.
 * Trusted surface: output is text, never HTML. OSC 52 (clipboard write) is swallowed;
 * OSC 0/1/2 titles are ignored (nobody listens to `onTitleChange`); OSC 8 and plain URLs only become
 * `http(s)` links that go to the link dialog through `links.ts`.
 * Keyboard: every key goes to the shell (Ctrl+C included; the app's shortcuts skip the terminal,
 * `state/shortcuts.ts`), except Ctrl+Shift+C / Ctrl+Shift+V (Cmd+C / Cmd+V on macOS) for copy and
 * paste, and Ctrl+` (Cmd+`) which moves focus out of the terminal (no keyboard trap).
 */
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

import type { TerminalStream } from "../../protocol/client/types";
import { detectMac } from "../../state/shortcuts";
import { createInputQueue } from "./inputQueue";
import { activateLink } from "./links";
import { registerTerminal } from "./selection";
import type { Terminals } from "./store";
import { terminalLook } from "./theme";
import { installXtermCspShim } from "./xtermCsp";

/** Debounce for fit + PTY resize while the panel is dragged (ShellX used 50 ms). */
const RESIZE_DEBOUNCE_MS = 50;
/** Lines of scrollback xterm keeps in the view (the core keeps its own 2 MiB). */
const VIEW_SCROLLBACK = 10_000;

/** What the view needs from its surroundings. */
export interface MountOptions {
  terminals: Terminals;
  sessionId: string;
  terminalId: string;
  /** Shows a notice to the user (clipboard failures). */
  notice: (text: string) => void;
  /** Called with Ctrl+` (Cmd+`) inside the terminal: move focus elsewhere. */
  leave: () => void;
}

/** A mounted terminal. */
export interface MountedTerminal {
  /** Re-reads the theme tokens (theme switch). */
  restyle(): void;
  /** Clears the view's scrollback (the core keeps its own). */
  clear(): void;
  /** Copies the selection to the clipboard. */
  copy(): Promise<void>;
  /** Pastes the clipboard into the terminal (as a bracketed paste when the program asked). */
  paste(): Promise<void>;
  /** Focuses the terminal. */
  focus(): void;
  /** Detaches and disposes everything (the terminal keeps running in the core). */
  dispose(): void;
}

/** Mounts terminal `terminalId` into `host`. */
export function mountTerminal(host: HTMLElement, options: MountOptions): MountedTerminal {
  installXtermCspShim();
  const { terminals, sessionId, terminalId } = options;
  const look = terminalLook();
  const term = new Terminal({
    theme: look.theme,
    fontFamily: look.fontFamily,
    fontSize: look.fontSize,
    lineHeight: 1.15,
    cursorBlink: true,
    scrollback: VIEW_SCROLLBACK,
    // The unicode11 addon uses `term.unicode`, a proposed API in xterm 6. The flag only gates what
    // our own code may call; program output cannot reach any API through it.
    allowProposedApi: true,
    linkHandler: {
      allowNonHttpProtocols: false,
      activate: (_event, text) => {
        activateLink(text);
      },
    },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.loadAddon(
    new WebLinksAddon((_event, uri) => {
      activateLink(uri);
    }),
  );
  term.loadAddon(new Unicode11Addon());
  term.unicode.activeVersion = "11";
  // OSC 52 asks the terminal to write the clipboard: swallowed, the program cannot reach it.
  const osc52 = term.parser.registerOscHandler(52, () => true);
  term.open(host);
  term.textarea?.setAttribute("data-testid", "terminal.input");
  term.textarea?.setAttribute("data-item-id", terminalId);

  const mac = detectMac();
  const queue = createInputQueue(
    (data, binary) => terminals.write(terminalId, data, binary),
    (error) => {
      terminals.report(sessionId, error);
    },
  );
  const copy = async (): Promise<void> => {
    const text = term.getSelection();
    if (text === "") return;
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      options.notice("Copy failed: the clipboard is not available here.");
    }
  };
  const paste = async (): Promise<void> => {
    try {
      const text = await navigator.clipboard.readText();
      if (text !== "") term.paste(text);
    } catch {
      options.notice("Paste failed: the clipboard could not be read.");
    }
  };
  term.attachCustomKeyEventHandler((event) => {
    if (event.type !== "keydown") return true;
    const mod = mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
    const clipboardMod = mac ? mod && !event.shiftKey : mod && event.shiftKey;
    if (clipboardMod && event.code === "KeyC" && !event.altKey) {
      void copy();
      return false;
    }
    if (clipboardMod && event.code === "KeyV" && !event.altKey) {
      event.preventDefault();
      void paste();
      return false;
    }
    if (mod && !event.shiftKey && !event.altKey && event.code === "Backquote") {
      event.preventDefault();
      options.leave();
      return false;
    }
    return true;
  });
  const typed = term.onData((data) => {
    queue.text(data);
  });
  const typedBinary = term.onBinary((data) => {
    queue.binary(data);
  });

  let stream: TerminalStream | null = null;
  let disposed = false;
  void terminals.client
    .attachTerminal(terminalId, (chunk, done) => {
      if (!disposed) term.write(chunk, done);
    })
    .then(
      (attached) => {
        if (disposed) attached.detach();
        else stream = attached;
      },
      (error: unknown) => {
        terminals.report(sessionId, error);
      },
    );

  let timer: ReturnType<typeof setTimeout> | null = null;
  const refit = (): void => {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      if (disposed || host.clientWidth === 0) return;
      fit.fit();
      void terminals.resize(terminalId, term.cols, term.rows);
    }, RESIZE_DEBOUNCE_MS);
  };
  const observer = new ResizeObserver(refit);
  observer.observe(host);
  refit();

  const unregister = registerTerminal({
    terminalId,
    selection: () => term.getSelection(),
    selectAll: () => {
      term.selectAll();
    },
    focus: () => {
      term.focus();
    },
    hasFocus: () => host.contains(document.activeElement),
    info: () => ({
      renderer: "dom",
      fontSize: term.options.fontSize ?? 0,
      cols: term.cols,
      rows: term.rows,
    }),
  });

  return {
    restyle() {
      const next = terminalLook();
      term.options.theme = next.theme;
      term.options.fontFamily = next.fontFamily;
      term.options.fontSize = next.fontSize;
      refit();
    },
    clear() {
      term.clear();
    },
    copy,
    paste,
    focus() {
      term.focus();
    },
    dispose() {
      disposed = true;
      if (timer !== null) clearTimeout(timer);
      observer.disconnect();
      unregister();
      queue.close();
      stream?.detach();
      typed.dispose();
      typedBinary.dispose();
      osc52.dispose();
      term.dispose();
    },
  };
}
