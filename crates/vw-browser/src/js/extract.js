// Bounded rendered table/list/link extraction for browser_read exports; never reads form values.
// Included only by the observation assembler; common.js supplies visibility and secret redaction.
/* global REQ, visible, hideKeys */
/* exported extractData */
const extractData = (elements, scanTruncated, select) => {
  const spec = REQ.extract, datasets = [];
  if (!spec) return {datasets};
  let remaining = 240000;
  const text = el => el && visible(el) ? select.text(el) : "";
  const link = el => {
    if (!el || !visible(el) || !el.hasAttribute("href")) return "";
    try { return hideKeys(new URL(el.getAttribute("href"), el.ownerDocument.baseURI).href); }
    catch { return ""; }
  };
  const dataset = (columns, raw, source, incomplete = false) => {
    let truncated = scanTruncated || incomplete || raw.length > 500;
    const rows = [];
    const clean = value => {
      const valueText = String(value);
      const limit = Math.min(2000, Math.max(0, remaining));
      if (valueText.length > limit) truncated = true;
      const valueOut = valueText.slice(0, limit);
      remaining -= valueOut.length;
      return valueOut;
    };
    for (const row of raw.slice(0, 500)) {
      if (remaining <= 0) { truncated = true; break; }
      if (row.length > 32) truncated = true;
      rows.push(row.slice(0, 32).map(clean));
    }
    datasets.push({columns:columns.slice(0, 32).map(clean),rows,source:hideKeys(source).slice(0, 2000),truncated});
  };
  try {
    const matches = select("css", spec.selector || {table:"table",list:"li,[role=listitem]",links:"a[href]"}[spec.kind]);
    const selected = elements.filter(el => matches.has(el) && visible(el));
    if (spec.kind === "table") {
      for (const table of selected.slice(0, 12)) {
        const trs = Array.from(table.querySelectorAll("tr")).filter(row => row.closest("table") === table && visible(row));
        const grid = [];
        let tableTruncated = false;
        for (const [y, row] of trs.slice(0, 502).entries()) {
          const cells = Array.from(row.children).filter(cell => cell.matches("td,th") && visible(cell));
          grid[y] ||= [];
          let x = 0;
          for (const cell of cells) {
            while (grid[y][x] !== undefined) x++;
            if (x >= 32) { tableTruncated = true; break; }
            const value = text(cell), height = Math.min(cell.rowSpan || 1, 502 - y), width = Math.min(cell.colSpan || 1, 32 - x);
            if (cell.colSpan > width || cell.rowSpan > height) tableTruncated = true;
            for (let dy = 0; dy < height; dy++) {
              grid[y + dy] ||= [];
              for (let dx = 0; dx < width; dx++) grid[y + dy][x + dx] = value;
            }
            x += width;
          }
        }
        const headed = trs[0] && Array.from(trs[0].children).every(cell => cell.localName === "th");
        const columns = headed ? grid.shift() || [] : Array.from({length:Math.max(0,...grid.map(row => row.length))},(_,i)=>`Column ${i+1}`);
        const rows = grid.filter((row, index) => {
          const element = trs[index + Number(Boolean(headed))];
          const repeatedHeader = headed && element?.parentElement.localName === "tfoot" &&
            Array.from(element.children).every(cell => cell.localName === "th") &&
            row.length === columns.length && row.every((value, i) => value === columns[i]);
          return !repeatedHeader;
        });
        dataset(spec.columns?.length ? spec.columns.map(column=>column.name) : columns,rows.map(row=>Array.from({length:columns.length},(_,i)=>row[i]??"")),table.ownerDocument.URL,tableTruncated || selected.length>12 || trs.length>502);
      }
    } else {
      const columns = spec.columns?.length ? spec.columns.map(column=>column.name) : spec.kind === "links" ? ["Text","URL"] : ["Text"];
      const documents = new Set(selected.map(el=>el.ownerDocument));
      for (const doc of documents) {
        const items = selected.filter(el=>el.ownerDocument===doc);
        const rows = items.slice(0,501).map(el => spec.columns?.length ? spec.columns.map(column => {
          const target = column.selector ? select("css", column.selector, el) : el;
          return column.attribute === "href" ? link(target) : text(target);
        }) : spec.kind === "links" ? [text(el),link(el)] : [text(el)]);
        dataset(columns,rows,doc.URL,items.length>500);
      }
    }
    return {datasets};
  } catch { return {datasets:[],extractionError:"invalid selector"}; }
};
