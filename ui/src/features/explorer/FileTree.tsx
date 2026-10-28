/** Virtualized folder tree. Uses the shared row, path and badge components; FilesPanel supplies opening behavior. */
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { VList, type VListHandle } from "virtua";
import { Badge, Button, RowDisclosure } from "../../components/Controls";
import { Icon } from "../../components/Icon";
import { PathText } from "../../components/PathText";
import { RowMenuButton, type RowMenuState } from "../../components/RowMenu";
import type { ExplorerTarget } from "./explorerMenus";
import type { Explorer, ExplorerState } from "./store";
import { fileWords } from "../git-browser/model";

type Row = { path: string; directory: boolean; depth: number; parent: string; more?: boolean };
/** Filters changed paths without reading unopened folders. */
export function visibleRows(state: ExplorerState): Row[] {
  const rows: Row[] = [];
  const visit = (path: string, depth: number) => {
    let entries = state.folders[path]?.entries ?? [];
    if (state.changedOnly) {
      const prefix = path ? `${path}/` : "";
      const children = new Map<string, boolean>();
      for (const file of state.status?.files ?? []) {
        if (!file.path.startsWith(prefix)) continue;
        const tail = file.path.slice(prefix.length),
          slash = tail.indexOf("/");
        children.set(prefix + (slash < 0 ? tail : tail.slice(0, slash)), slash >= 0);
      }
      entries = [...children]
        .map(([path, directory]) => ({ path, directory }))
        .sort((a, b) => Number(b.directory) - Number(a.directory) || a.path.localeCompare(b.path));
    }
    for (const entry of entries) {
      rows.push({ ...entry, depth, parent: path });
      if (entry.directory && state.expanded.includes(entry.path)) visit(entry.path, depth + 1);
    }
    if (!state.changedOnly && state.folders[path]?.next != null)
      rows.push({ path, depth, parent: path, directory: false, more: true });
  };
  visit("", 0);
  return rows;
}
const rowId = (row: Row) => JSON.stringify([row.path, row.more === true]);
const targetOf = (row: Row): ExplorerTarget =>
  row.directory
    ? { kind: "folder", path: row.path }
    : { kind: "file", path: row.path, place: "files" };
export function FileTree({
  state,
  explorer,
  onOpen,
  menu = null,
}: {
  state: ExplorerState;
  explorer: Explorer;
  onOpen: (path: string, line?: number) => void;
  /** The tab's one menu (right click, menu key, ⋯); null: no menu. */
  menu?: RowMenuState<ExplorerTarget> | null;
}) {
  const rows = useMemo(() => visibleRows(state), [state]);
  const [selected, setSelected] = useState("");
  const list = useRef<VListHandle>(null);
  const elements = useRef(new Map<string, HTMLDivElement>());
  const pending = useRef<string | null>(null);
  const revealedRow =
    state.revealed === null ? undefined : rows.find((r) => r.path === state.revealed && !r.more);
  const active = revealedRow
    ? rowId(revealedRow)
    : rows.some((r) => rowId(r) === selected)
      ? selected
      : rows[0]
        ? rowId(rows[0])
        : undefined;
  const positions = useMemo(() => {
    const counts = new Map<string, number>(),
      result = new Map<string, { position: number; size: number }>();
    for (const row of rows) {
      const position = (counts.get(row.parent) ?? 0) + 1;
      counts.set(row.parent, position);
      result.set(rowId(row), { position, size: 0 });
    }
    for (const row of rows) {
      const item = result.get(rowId(row));
      if (item)
        item.size = state.folders[row.parent]?.next != null ? -1 : (counts.get(row.parent) ?? 0);
    }
    return result;
  }, [rows, state.folders]);
  function focus(row: Row | undefined) {
    if (!row) return;
    setSelected(rowId(row));
    pending.current = rowId(row);
    list.current?.scrollToIndex(rows.indexOf(row), { align: "nearest" });
    const el = elements.current.get(rowId(row));
    if (el) {
      el.focus();
      pending.current = null;
    }
  }
  // Reveal in Files: once the revealed row is in the list, it is scrolled to and focused (focus selects it).
  useEffect(() => {
    if (revealedRow === undefined) return;
    const id = rowId(revealedRow);
    pending.current = id;
    list.current?.scrollToIndex(rows.indexOf(revealedRow), { align: "nearest" });
    const el = elements.current.get(id);
    if (el) {
      el.focus();
      pending.current = null;
    }
    explorer.revealDone();
  }, [revealedRow, rows, explorer]);
  const open = (row: Row) => {
    if (row.more) void explorer.loadFolder(row.path, true);
    else if (row.directory) explorer.toggle(row.path);
    else onOpen(row.path);
  };
  function keyboard(event: KeyboardEvent<HTMLDivElement>, row: Row) {
    if ((event.target as HTMLElement).closest("button") && event.key === "Enter") return;
    const at = rows.indexOf(row);
    switch (event.key) {
      case "ArrowDown":
        focus(rows[Math.min(rows.length - 1, at + 1)]);
        break;
      case "ArrowUp":
        focus(rows[Math.max(0, at - 1)]);
        break;
      case "Home":
        focus(rows[0]);
        break;
      case "End":
        focus(rows.at(-1));
        break;
      case "ArrowRight":
        if (row.directory && !state.expanded.includes(row.path)) explorer.toggle(row.path);
        else if (rows[at + 1]?.parent === row.path) focus(rows[at + 1]);
        break;
      case "ArrowLeft":
        if (row.directory && state.expanded.includes(row.path)) explorer.toggle(row.path);
        else focus(rows.find((r) => r.path === row.parent));
        break;
      case "Enter":
        open(row);
        break;
      default:
        return;
    }
    event.preventDefault();
  }
  if (!rows.length)
    return (
      <p className="key-line">
        {state.loadingFolders.includes("")
          ? "Reading files…"
          : state.changedOnly
            ? "No changed files in this folder."
            : "This folder is empty."}
      </p>
    );
  return (
    <VList ref={list} className="fx-tree" role="tree" aria-label="Files in this folder" data={rows}>
      {(row: Row) => {
        const status = state.status?.files.find((f) => f.path === row.path)?.status;
        const count = row.directory
          ? (state.status?.files.filter((f) => f.path.startsWith(`${row.path}/`)).length ?? 0)
          : 0;
        return (
          <div
            key={rowId(row)}
            className="fx-tree-row"
            role="treeitem"
            aria-level={row.depth + 1}
            aria-posinset={positions.get(rowId(row))?.position}
            aria-setsize={positions.get(rowId(row))?.size}
            aria-selected={active === rowId(row)}
            aria-expanded={row.directory ? state.expanded.includes(row.path) : undefined}
            tabIndex={active === rowId(row) ? 0 : -1}
            style={{ paddingInlineStart: 8 + row.depth * 16 }}
            data-testid="explorer.tree-row"
            data-item-id={row.path}
            ref={(el) => {
              if (el) {
                elements.current.set(rowId(row), el);
                if (pending.current === rowId(row)) {
                  el.focus();
                  pending.current = null;
                }
              } else elements.current.delete(rowId(row));
            }}
            onFocus={() => {
              setSelected(rowId(row));
            }}
            onContextMenu={(e) => {
              if (!row.more) menu?.triggers(targetOf(row)).onContextMenu(e);
            }}
            onKeyDown={(e) => {
              if (!row.more) menu?.triggers(targetOf(row)).onKeyDown(e);
              if (!e.defaultPrevented) keyboard(e, row);
            }}
          >
            {row.more ? (
              <Button
                data-testid="explorer.folder-more"
                data-item-id={row.path}
                onClick={() => {
                  open(row);
                }}
              >
                Show 5,000 more
              </Button>
            ) : (
              <>
                <RowDisclosure
                  data-testid="explorer.open-entry"
                  data-item-id={row.path}
                  tabIndex={-1}
                  aria-expanded={row.directory ? state.expanded.includes(row.path) : undefined}
                  onClick={() => {
                    open(row);
                  }}
                >
                  {row.directory ? (
                    <>
                      <Icon
                        name={state.expanded.includes(row.path) ? "chevron-down" : "chevron-right"}
                        size={14}
                      />
                      <Icon
                        name={state.expanded.includes(row.path) ? "folder-open" : "folder"}
                        size={14}
                      />
                    </>
                  ) : (
                    <>
                      <span className="fx-twist-gap" />
                      <Icon name="file" size={14} />
                    </>
                  )}
                  <PathText path={row.path.split("/").at(-1) ?? row.path} title={row.path} />
                </RowDisclosure>
                {status && <Badge>{fileWords(status)}</Badge>}
                {count > 0 && (
                  <span className="ui-caption">{count.toLocaleString("en-US")} changed</span>
                )}
                {state.loadingFolders.includes(row.path) && (
                  <span className="ui-caption" role="status">
                    Reading…
                  </span>
                )}
                {menu && (
                  <RowMenuButton
                    testId="explorer.row-more"
                    itemId={row.path}
                    label={`Actions for ${row.path}`}
                    expanded={
                      menu.at !== null &&
                      JSON.stringify(menu.at.target) === JSON.stringify(targetOf(row))
                    }
                    onOpen={(button) => {
                      menu.openBelow(button, targetOf(row));
                    }}
                  />
                )}
              </>
            )}
          </div>
        );
      }}
    </VList>
  );
}
