/**
 * AST and model contracts for the own flowchart / graph engine.
 * Pure data types with zero DOM or external layout dependencies.
 */

/** Layout orientation of a flowchart or subgraph. */
export type FlowDirection = "TB" | "BT" | "LR" | "RL";

/** Every documented classic and expanded `@{ shape: ... }` node geometry. */
export type FlowShape =
  | "rect"
  | "rounded"
  | "stadium"
  | "subproc"
  | "cyl"
  | "circle"
  | "odd"
  | "diamond"
  | "hex"
  | "lean-r"
  | "lean-l"
  | "datastore"
  | "trap-b"
  | "trap-t"
  | "dbl-circ"
  | "text"
  | "notch-rect"
  | "lin-rect"
  | "sm-circ"
  | "framed-circle"
  | "fork"
  | "hourglass"
  | "comment"
  | "brace-r"
  | "braces"
  | "bolt"
  | "doc"
  | "delay"
  | "das"
  | "lin-cyl"
  | "curv-trap"
  | "div-rect"
  | "tri"
  | "win-pane"
  | "f-circ"
  | "lin-doc"
  | "notch-pent"
  | "flip-tri"
  | "sl-rect"
  | "docs"
  | "processes"
  | "flag"
  | "bow-rect"
  | "cross-circ"
  | "tag-doc"
  | "tag-rect";

/** Stroke style of a flowchart edge. */
export type FlowEdgeStyle = "solid" | "dotted" | "thick" | "invisible";

/** Decoration at the start or end of a flowchart edge. */
export type FlowEndpointMark = "none" | "arrow" | "circle" | "cross";

/** A flowchart vertex with its resolved display label, shape, and optional subgraph membership. */
export interface FlowNode {
  id: string;
  label: string;
  hasExplicitLabel: boolean;
  shape: FlowShape;
  subgraphId?: string;
  metadata: Record<string, string>;
}

/** A directed, undirected, or bidirectional edge between nodes or subgraphs. */
export interface FlowEdge {
  id?: string;
  from: string;
  to: string;
  label: string;
  style: FlowEdgeStyle;
  startMark: FlowEndpointMark;
  endMark: FlowEndpointMark;
  minlen: number;
}

/** A subgraph container with optional explicit title, orientation, nesting, and collapse state. */
export interface FlowSubgraph {
  id: string;
  title: string;
  parentId?: string;
  direction?: FlowDirection;
  collapsed: boolean;
  nodeIds: string[];
  childSubgraphIds: string[];
}

/** Complete parsed representation of a Mermaid flowchart / graph diagram. */
export interface FlowchartModel {
  direction: FlowDirection;
  nodes: FlowNode[];
  edges: FlowEdge[];
  subgraphs: FlowSubgraph[];
}
