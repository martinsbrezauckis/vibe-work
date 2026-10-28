/**
 * The right-click menu of code: the editor (selection or the caret's line), the files viewer's text and the
 * read-only saved side of a compare. Same grammar as every menu (RowMenu): Point Vibe at these lines first,
 * then edit and copy, find and go to, versions; the header line names `path:line` or `path:first-last`.
 * Depends on: fileMenu (chips, copy), model/context (file chips), RowMenu (items), AppContext.
 * Called by: FileTab (editor and viewer) and SavedComparison (both sides of a compare).
 */
import { writeClipboard } from "../../state/clipboard";
import { type MenuItem, menuGroups } from "../../components/RowMenu";
import type { App } from "../shell/AppContext";
import { copyText, pointChip } from "./fileMenu";
import { absoluteFilePath, type CodeSelection, fileChip } from "./model/context";

/** What the menu needs from one piece of shown code. */
export interface CodeSurface {
  /** The selection, or null when nothing is selected. */
  selection(): CodeSelection | null;
  /** The caret's line (1-based); the viewer: the line under the pointer, else 1. */
  line(): number;
  lineText(line: number): string;
  /** Replaces the selection (Cut, Paste); false when it cannot be changed. */
  replace(text: string): boolean;
  find(): void;
  /** Opens Go to line; absent where there is none (the viewer). */
  goToLine?: () => void;
  readOnly: boolean;
}

/** Where the code is. */
export interface CodePlace {
  owner: string;
  folder: string;
  /** The path as shown (relative inside the folder). */
  path: string;
  /** The editor holds unsaved edits (the chip says so). */
  unsaved: boolean;
  /** The read-only side of a compare: its words and how to point at it. */
  saved?: { words: string; tab: string; text: string; close: () => void } | null;
  /** The editable side of a compare: Back to one editor; else Compare with saved version. */
  compare?: { back: () => void } | { open: () => void } | null;
  /** The editor (Cut, Paste, Go to line) or the files viewer (Copy, Find). */
  editor: boolean;
}

/** The lines a point or a link names: the selection, else the caret's line. */
function lines(surface: CodeSurface): CodeSelection {
  const selected = surface.selection();
  if (selected !== null) return selected;
  const line = surface.line();
  return { start: line, end: line, text: surface.lineText(line) };
}

const at = (path: string, range: CodeSelection) =>
  `${path}:${range.start === range.end ? String(range.start) : `${String(range.start)}-${String(range.end)}`}`;

/** The header line, label and items of the code menu. */
export function codeMenu(
  app: App,
  place: CodePlace,
  surface: CodeSurface,
): { heading: string; label: string; items: MenuItem[] } {
  const range = lines(surface);
  const selected = surface.selection();
  const link = at(place.path, range);
  const run = (action: () => unknown) => () => {
    void action();
  };
  const copySelection = () => copyText(app, selected?.text ?? "", "the selected text");
  if (place.saved) {
    const saved = place.saved;
    return {
      heading: saved.words,
      label: `Actions for ${saved.words}`,
      items: menuGroups(
        [
          {
            id: "point",
            label: "Point Vibe at the saved version",
            run: run(() =>
              pointChip(app, place.owner, place.folder, {
                label: `${place.path} · ${saved.tab}`,
                source: "git",
                text: saved.text,
              }),
            ),
          },
        ],
        [
          selected !== null && {
            id: "copy",
            label: "Copy",
            keys: "Mod+C",
            run: run(copySelection),
          },
        ],
        [{ id: "close-saved", label: "Close the saved version", run: saved.close }],
      ),
    };
  }
  const point = () =>
    pointChip(
      app,
      place.owner,
      place.folder,
      fileChip({
        path: absoluteFilePath(place.path, place.folder),
        project: place.folder,
        name: place.path.split("/").at(-1) ?? place.path,
        unsaved: place.unsaved,
        selection: range,
      }),
    );
  const compare = place.compare;
  return {
    heading: link,
    label: `Actions for ${link}`,
    items: menuGroups(
      [
        {
          id: "point",
          label:
            range.start === range.end ? "Point Vibe at this line" : "Point Vibe at these lines",
          keys: "Mod+L",
          run: run(point),
        },
      ],
      [
        place.editor && {
          id: "cut",
          label: "Cut",
          keys: "Mod+X",
          disabled: surface.readOnly || selected === null,
          run: run(async () => {
            if (selected === null || surface.readOnly) return;
            await writeClipboard(selected.text);
            surface.replace("");
          }),
        },
        (place.editor || selected !== null) && {
          id: "copy",
          label: "Copy",
          keys: "Mod+C",
          disabled: selected === null,
          run: run(copySelection),
        },
        place.editor && {
          id: "paste",
          label: "Paste",
          keys: "Mod+V",
          disabled: surface.readOnly,
          run: run(async () => {
            const text = await navigator.clipboard.readText();
            surface.replace(text);
          }),
        },
        { id: "copy-link", label: "Copy line link", run: run(() => copyText(app, link)) },
      ],
      [
        {
          id: "find",
          label: "Find in this file",
          keys: "Mod+F",
          run: () => {
            surface.find();
          },
        },
        surface.goToLine !== undefined && {
          id: "go-to-line",
          label: "Go to line…",
          keys: "Mod+Alt+G",
          run: () => {
            surface.goToLine?.();
          },
        },
      ],
      [
        compare != null &&
          ("back" in compare
            ? { id: "back", label: "Back to one editor", run: compare.back }
            : { id: "compare", label: "Compare with saved version", run: compare.open }),
      ],
    ),
  };
}
