/**
 * Identity-based palette selection, shared by arrow keys, pointer hover and Enter.
 * Depends on: React, paletteModel. Called by: CommandPalette.
 * Invariant: an arriving row cannot change the highlighted result while its identity remains visible.
 */
import { useState } from "react";
import type { PaletteRow } from "./paletteModel";

/** Keeps the selected identity, or chooses the first visible row if it disappeared. Empty lists select nothing. */
export function usePaletteSelection(rows: readonly PaletteRow[]) {
  const [selection, setSelection] = useState({ rows, id: rows[0]?.result.id ?? null });
  const at = rows.findIndex((row) => row.result.id === selection.id);
  const active = Math.max(0, at);
  const selected = rows[active]?.result;
  if (selection.rows !== rows) setSelection({ rows, id: selected?.id ?? null });
  return {
    active,
    selected,
    select: (index: number) => {
      setSelection({ rows, id: rows[index]?.result.id ?? null });
    },
  };
}
