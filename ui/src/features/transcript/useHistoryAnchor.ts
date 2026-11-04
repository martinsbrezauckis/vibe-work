/** Keeps the row being read in place as older history arrives, including a simultaneous live append.
 * Depends on React and virtua geometry; Transcript supplies its rows and scroll notifications.
 * Bottom and prompt following remain owned by useFollow. No transcript entries are copied here.
 */
import { useLayoutEffect, useRef, type RefObject } from "react";
import type { VListHandle } from "virtua";
import type { Row } from "./model/build";

export function useHistoryAnchor(
  list: RefObject<VListHandle | null>,
  rows: readonly Row[],
  following: boolean,
) {
  const anchor = useRef<{ key: string; index: number; offset: number } | null>(null);
  const first = useRef<string | undefined>(undefined);
  const remember = (offset: number) => {
    const handle = list.current;
    if (handle === null) return;
    const index = handle.findItemIndex(offset);
    const row = rows[index];
    if (row !== undefined)
      anchor.current = { key: row.key, index, offset: offset - handle.getItemOffset(index) };
  };
  useLayoutEffect(() => {
    const previousFirst = first.current;
    first.current = rows[0]?.key;
    const saved = anchor.current;
    if (following) {
      anchor.current = null;
    } else if (saved !== null && previousFirst !== first.current) {
      const index = rows.findIndex((row) => row.key === saved.key);
      if (index < 0) {
        anchor.current = null;
      } else if (index !== saved.index) {
        anchor.current = { ...saved, index };
        list.current?.scrollToIndex(index, { align: "start", offset: saved.offset });
      }
    }
  }, [list, rows, following]);
  return remember;
}
