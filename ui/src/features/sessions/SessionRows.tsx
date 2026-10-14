/**
 * Stable sibling rows for a list section, including hidden matches and folded descendants.
 * Depends on: listModel, shell expansion state, SessionRow. Called by: the list and its groups.
 * Filtering changes visibility/depth in one render; promoting a child keeps its React parent.
 */
import { useShellState } from "../shell/hooks";
import type { RowNode } from "./listModel";
import { SessionRow } from "./SessionRow";

function flatten(rows: readonly RowNode[]): RowNode[] {
  return rows.flatMap((node) => [node, ...flatten(node.children)]);
}

/** Retains loaded rows; `shown` supplies the filtered tree and visible order. */
export function SessionRows({
  rows,
  shown = rows,
}: {
  rows: readonly RowNode[];
  shown?: readonly RowNode[];
}) {
  const expanded = useShellState((s) => s.expanded);
  const visible: { node: RowNode; depth: number }[] = [];
  const visit = (nodes: readonly RowNode[], depth: number) => {
    for (const node of nodes) {
      visible.push({ node, depth });
      if (expanded[node.session.id] === true) visit(node.children, depth + 1);
    }
  };
  visit(shown, 0);
  const ids = new Set(visible.map(({ node }) => node.session.id));
  const retained = [
    ...visible.map((row) => ({ ...row, hidden: false })),
    ...flatten(rows)
      .filter((node) => !ids.has(node.session.id))
      .map((node) => ({ node, depth: 0, hidden: true })),
  ];
  return (
    <>
      {retained.map(({ node, depth, hidden }) => (
        <SessionRow key={node.session.id} node={node} depth={depth} hidden={hidden} />
      ))}
    </>
  );
}
