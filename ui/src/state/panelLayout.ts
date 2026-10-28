/**
 * Layout decisions for the shell, as one pure function: is the list visible, how wide is the side
 * panel, is it a drawer. Up to three columns show (list, chat, one panel): a wide tab takes the room the chat can
 * spare and folds the list only when the three do not fit; a dragged width is remembered per tab kind and clamped;
 * narrow windows turn the panel into an overlay drawer.
 * Depends on: shellState. Called by: the Shell component (sets CSS variables and classes), the
 * UI-control `state` route, tests.
 * Invariants: pure; the chat column never gets narrower than `CHAT_MIN` in grid mode; the panel is
 * never narrower than `PANEL_MIN`.
 */
import { editorLayoutOf } from "./editorLayout";
import { type ShellState, TAB_META, panelOf } from "./shellState";

/** Panel width for a side tab. */
const SIDE_W = 520;
/** Chat width a wide tab leaves. */
const CHAT_COMPACT = 440;
/** Narrowest chat column the panel may leave. */
export const CHAT_MIN = 380;
/** Icon rail width. */
export const RAIL_W = 56;
/** Session list width. */
const COL_W = 284;
/** Narrowest side panel. */
export const PANEL_MIN = 360;
/** At or below this width the panel is an overlay drawer and the list is hidden. */
export const DRAWER_MAX = 899;
/** At or below this width the rail is hidden too (phone layout). */
const PHONE_MAX = 640;

/** The computed layout. */
export interface Layout {
  listVisible: boolean;
  editorSplit: boolean;
  slotWidth: number;
  slot: "chat" | "list" | "folded";
  chatFolded: boolean;
  panelOpen: boolean;
  panelWidth: number;
  /** The active tab wants room. */
  wide: boolean;
  drawer: boolean;
  phone: boolean;
  /** The panel owns the whole window. */
  maximized: boolean;
  /** The list is open as an overlay drawer (narrow windows only). */
  listDrawerOpen: boolean;
}

/** Computes the layout for `state` in a window `viewport` pixels wide. */
export function computeLayout(state: ShellState, viewport: number): Layout {
  const panel = panelOf(state);
  const editor = editorLayoutOf(state);
  // The Settings page has the list column for its categories and no side panel.
  const settings = state.settings.open;
  const active = panel.open && !settings ? panel.active : null;
  const panelOpen = active !== null;
  const wide = active !== null && TAB_META[active].size === "wide";
  const drawer = viewport <= DRAWER_MAX;
  const phone = viewport <= PHONE_MAX;
  const editorSplit = editor.split && panelOpen && !settings;
  const slotWidth = Math.max(CHAT_MIN, Math.min(520, viewport * 0.3));
  const actualSlot = Math.max(
    CHAT_MIN,
    Math.min(editor.width ?? slotWidth, viewport - RAIL_W - 48 - 400),
  );
  if (editorSplit)
    return {
      listVisible: !drawer && editor.slot === "list",
      panelOpen,
      panelWidth: drawer
        ? viewport
        : Math.round(viewport - RAIL_W - 48 - (editor.slot === "folded" ? 48 : actualSlot)),
      wide: true,
      drawer,
      phone,
      maximized: false,
      listDrawerOpen: false,
      editorSplit: true,
      slotWidth: Math.round(actualSlot),
      slot: editor.slot,
      chatFolded: editor.slot !== "chat",
    };
  // Up to three columns: the list, the chat and one panel, wide tabs included. The list folds
  // only when the three do not fit; a list the person opened (pinned) stays then, and the panel folds instead until a
  // tab is opened again (`openTab` ends the pin).
  const fits = viewport >= RAIL_W + COL_W + CHAT_MIN + PANEL_MIN;
  const listWanted = settings || state.list !== "hidden";
  const panelShown = panelOpen && (drawer || fits || state.list !== "shown");
  const listVisible = !drawer && listWanted && (!panelShown || fits);
  const avail = viewport - RAIL_W - (listVisible ? COL_W : drawer ? 0 : 48);
  let width = SIDE_W;
  if (active !== null && panelShown) {
    const remembered = state.widths[active];
    if (remembered !== undefined) width = remembered;
    else if (drawer) width = wide ? viewport * 0.92 : SIDE_W;
    else width = wide ? avail - CHAT_COMPACT : SIDE_W;
  }
  if (!drawer) width = Math.max(PANEL_MIN, Math.min(width, avail - CHAT_MIN));
  return {
    editorSplit: false,
    slotWidth: Math.round(slotWidth),
    slot: "chat",
    chatFolded: editor.chatFolded,
    listVisible,
    panelOpen: panelShown,
    panelWidth: Math.round(width),
    wide: wide && panelShown,
    drawer,
    phone,
    maximized: panelShown && state.maximized,
    listDrawerOpen: drawer && state.drawer,
  };
}
