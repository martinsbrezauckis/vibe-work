/**
 * Generic category icons for the Connectors page and the rail's Tools button: no vendor logos (names and generic category icons). They come from the app's one icon set (`components/Icon`, Lucide), so a
 * connector tile and the rest of the app share one grid and one line weight.
 * Depends on: components/Icon, model/meta. Called by: ConnectorTile, ConnectorDetail, ServerList.
 * Invariant: decorative only (`aria-hidden`); the button or tile around it carries the label.
 */
import type { ReactElement } from "react";

import { Icon, type IconName, type IconSize } from "../../components/Icon";
import type { CategoryId } from "./model/meta";

/** A category, or `plug` (Tools, MCP servers in general) or `server` (one MCP server). */
export type GlyphName = CategoryId | "plug" | "server";

/** Category → icon. Each meaning has its own picture: chat apps a round bubble (sessions use the square one), file
 * storage a cloud (a folder means a project), "other" blocks (the plug means Tools). */
const ICON: Readonly<Record<GlyphName, IconName>> = {
  code: "code",
  issues: "list-todo",
  docs: "file-text",
  comms: "message-circle",
  mail: "mail",
  files: "cloud",
  data: "database",
  pay: "credit-card",
  search: "search",
  other: "blocks",
  plug: "plug",
  server: "server",
};

/** One glyph, 16 px by default (tiles), 20 px for detail headers. */
export function CategoryIcon({
  name,
  size = 16,
}: {
  name: GlyphName;
  size?: IconSize;
}): ReactElement {
  return <Icon name={ICON[name]} size={size} />;
}
