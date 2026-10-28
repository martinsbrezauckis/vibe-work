/**
 * Quote in reply: a right click (or the menu key) on text selected inside one of Vibe's answers opens the answer's
 * menu (Quote in reply, Copy). Quote in reply puts the selection into this session's composer as a quote chip;
 * nothing is sent. Without such a selection the window's own menu is left alone.
 * Depends on: RowMenu, files/fileMenu (chips into the composer, copy), AppContext. Called by: Transcript.
 */
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactElement,
} from "react";
import { createPortal } from "react-dom";
import { RowMenu, isMenuKey, menuGroups, useRowMenu } from "../../components/RowMenu";
import { copyText, pointChip } from "../files/fileMenu";
import type { App } from "../shell/AppContext";

const MAX_QUOTE = 4000;

/** The text selected inside one answer of Vibe (`.a-msg`), or null. */
function answerSelection(): string | null {
  const selection = window.getSelection();
  if (selection === null || selection.isCollapsed || selection.rangeCount === 0) return null;
  const start = selection.anchorNode;
  const end = selection.focusNode;
  const answer = (node: Node | null) =>
    (node instanceof Element ? node : (node?.parentElement ?? null))?.closest(".a-msg") ?? null;
  const first = answer(start);
  if (first === null || first !== answer(end)) return null;
  const text = selection.toString().trim();
  return text === "" ? null : text;
}

/** The quote chip for `text`. */
function quoteChip(text: string): { label: string; source: string; text: string } {
  const flat = text.replace(/\s+/g, " ");
  const cut = text.length > MAX_QUOTE;
  return {
    label: `Quote · “${flat.length > 40 ? `${flat.slice(0, 40)}…` : flat}”`,
    source: "quote",
    text:
      "Quoted from your earlier answer in this chat:\n" +
      text
        .slice(0, MAX_QUOTE)
        .split("\n")
        .map((line) => `> ${line}`)
        .join("\n") +
      (cut ? "\n(cut: the quote is longer)" : ""),
  };
}

/** The menu's openers and element for the transcript of `sessionId`. */
export function useQuoteMenu(
  app: App,
  sessionId: string,
): {
  onContextMenu: (event: ReactMouseEvent) => void;
  onKeyDown: (event: ReactKeyboardEvent) => void;
  element: ReactElement | null;
} {
  const menu = useRowMenu<string>();
  const open = (event: ReactMouseEvent | ReactKeyboardEvent, x: number, y: number) => {
    const text = answerSelection();
    if (text === null) return;
    event.preventDefault();
    event.stopPropagation();
    menu.open({ x, y, target: text });
  };
  return {
    onContextMenu: (event) => {
      open(event, event.clientX, event.clientY);
    },
    onKeyDown: (event) => {
      if (!isMenuKey(event)) return;
      const box = window.getSelection()?.getRangeAt(0).getBoundingClientRect();
      open(event, box?.left ?? 24, (box?.bottom ?? 24) + 2);
    },
    element:
      menu.at === null
        ? null
        : createPortal(
            <RowMenu
              testId="transcript.quote-menu"
              at={menu.at}
              label="Actions for the selected text"
              heading={`Selected in Vibe's answer · ${menu.at.target.length.toLocaleString("en-US")} characters`}
              items={menuGroups(
                [
                  {
                    id: "quote",
                    label: "Quote in reply",
                    run: () => {
                      pointChip(app, sessionId, "", quoteChip(menu.at?.target ?? ""));
                    },
                  },
                ],
                [
                  {
                    id: "copy",
                    label: "Copy",
                    keys: "Mod+C",
                    run: () => void copyText(app, menu.at?.target ?? "", "the selected text"),
                  },
                ],
              )}
              onClose={menu.close}
            />,
            document.body,
          ),
  };
}
