/** Pie chart parser: `pie [showData] [title T]`, optional `title T`, then `"label" : value` lines. */
import { linesOf } from "../../detect";
import { MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";

/** One labelled slice with its non-negative value. */
export interface PieSlice {
  label: string;
  value: number;
}
/** A parsed pie chart: title, whether values are shown, the slices in source order and their sum. */
export interface PieModel {
  title: string;
  showData: boolean;
  slices: PieSlice[];
  total: number;
}

const SLICE = /^"((?:[^"\\]|\\.)*)"\s*:\s*(\S+)\s*;?$/;
const NUMBER = /^\d{1,12}(?:\.\d{1,12})?$/;

/** Parse `pie` source (text or prepared lines); throws DiagramInputError on anything unreadable. */
export function parsePie(input: string | PreparedSource): PieModel {
  const [header = "", ...body] = linesOf(input, "pie");
  const head = /^pie(?:\s+(showData))?(?:\s+title\s+(.*))?$/i.exec(header.trim());
  if (!head) throw new DiagramInputError("Unreadable pie header");
  let showData = head[1] !== undefined;
  let title = (head[2] ?? "").trim();
  const slices: PieSlice[] = [];
  for (const raw of body) {
    const line = raw.trim();
    const titled = /^title\s+(.*)$/i.exec(line);
    if (titled) {
      title = (titled[1] ?? "").trim();
      continue;
    }
    if (/^showData$/i.test(line)) {
      showData = true;
      continue;
    }
    const slice = SLICE.exec(line);
    if (!slice || !NUMBER.test(slice[2] ?? ""))
      throw new DiagramInputError(`Unreadable pie line: ${line.slice(0, 40)}`);
    slices.push({ label: (slice[1] ?? "").replace(/\\(.)/g, "$1"), value: Number(slice[2]) });
    checkCount(slices.length, MAX_NODES, "slices");
  }
  const total = slices.reduce((sum, slice) => sum + slice.value, 0);
  if (slices.length === 0 || !(total > 0))
    throw new DiagramInputError("A pie chart needs slices with a positive total");
  return { title, showData, slices, total };
}
