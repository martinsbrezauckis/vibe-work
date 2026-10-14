/**
 * Layout decisions for the shell, as one pure function: is the list visible, how wide is the side
 * panel, is it a drawer. Adapted from the design mockup's panel sizing: a wide tab collapses
 * the list and compacts the chat; a dragged width is remembered per tab kind and clamped; narrow
 * windows turn the panel into an overlay drawer.
 * Depends on: shellState. Called by: the Shell component (sets CSS variables and classes), the
 * UI-control `state` route, tests.
 * Invariants: pure; the chat column never gets narrower than `CHAT_MIN` in grid mode; the panel is
 * never narrower than `PANEL_MIN`.
 */
import { type ShellState, TAB_META, panelOf } from "./shellState";

/** Panel width for a side tab. */
export const SIDE_W = 520;
/** Chat width a wide tab leaves. */
export const CHAT_COMPACT = 440;
/** Narrowest chat column the panel may leave. */
export const CHAT_MIN = 380;
/** Icon rail width. */
export const RAIL_W = 56;
/** Session list width. */
export const COL_W = 284;
/** Narrowest side panel. */
export const PANEL_MIN = 360;
/** At or below this width the panel is an overlay drawer and the list is hidden. */
export const DRAWER_MAX = 1100;
/** At or below this width the rail is hidden too (phone layout). */
export const PHONE_MAX = 640;

/** The computed layout. */
export interface Layout {
  listVisible: boolean;
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
  // The Settings page has the list column for its categories and no side panel.
  const settings = state.settings.open;
  const active = panel.open && !settings ? panel.active : null;
  const panelOpen = active !== null;
  const wide = active !== null && TAB_META[active].size === "wide";
  const drawer = viewport <= DRAWER_MAX;
  const phone = viewport <= PHONE_MAX;
  const listVisible =
    !drawer &&
    (settings || (state.list !== "hidden" && !(state.list === "auto" && panelOpen && wide)));
  const avail = viewport - RAIL_W - (listVisible ? COL_W : 0);
  let width = SIDE_W;
  if (active !== null) {
    const remembered = state.widths[active];
    if (remembered !== undefined) width = remembered;
    else if (drawer) width = wide ? viewport * 0.92 : SIDE_W;
    else width = wide ? avail - CHAT_COMPACT : SIDE_W;
  }
  if (!drawer) width = Math.max(PANEL_MIN, Math.min(width, avail - CHAT_MIN));
  return {
    listVisible,
    panelOpen,
    panelWidth: Math.round(width),
    wide,
    drawer,
    phone,
    maximized: panelOpen && state.maximized,
    listDrawerOpen: drawer && state.drawer,
  };
}
