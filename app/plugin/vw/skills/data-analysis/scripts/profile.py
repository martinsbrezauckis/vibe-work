"""Vibe Work data-analysis helper: a short profile of a data file, so the agent reads a summary instead of the data.

Usage: python3 profile.py DATA [--sheet NAME_OR_NUMBER] [--csv OUT.csv] [--rows N]

DATA is a .csv, .tsv, .json (a list of objects, or {"key": [objects]}) or .xlsx file. Prints, in at most 6,000
characters: the row and column counts, duplicate and empty rows, and per column its type (number, date, text), how many
cells are filled, distinct values, then min / max / mean / sum for numbers, the range for dates, the most common values
for text, and the first rows (--rows, default 5, at most 20). Odd values (text in a number column) are listed with
their row number. For .xlsx: the sheet names, then the chosen sheet (default the first); --csv also writes that sheet
as CSV (the input for your own scripts). Python standard library only; no network.
"""
import csv
import io
import json
import os
import re
import sys
import zipfile
from collections import Counter

MAX_OUT = 6000          # total characters printed (the cap the agent relies on)
MAX_COLUMNS = 40        # columns described in detail
CELL = 40               # characters shown of one cell value
LINE = 220              # characters of one printed line
ROWS_BUDGET = 1500      # characters for the first rows
ODD_BUDGET = 800        # characters for the odd values
DATE = re.compile(r"^\d{4}-\d{2}(-\d{2})?([ T]\d{2}:\d{2}(:\d{2})?)?$|^\d{1,2}[./]\d{1,2}[./]\d{2,4}$")


def number(text):
    """A cell as a float, or None. Accepts 1,234.5 / 12% / currency signs; '' is None (missing, not odd)."""
    t = (text or "").strip().replace(",", "").replace("%", "").replace("€", "").replace("$", "").replace(
        "£", "").replace(" ", "")
    if not t:
        return None
    try:
        return float(t)
    except ValueError:
        return None


def short(value):
    value = str(value).replace("\n", " ")
    return value if len(value) <= CELL else value[: CELL - 1] + "…"


def fmt(v):
    if v is None:
        return "-"
    if abs(v) >= 1000 or float(v).is_integer():
        return f"{v:,.0f}" if float(v).is_integer() else f"{v:,.2f}"
    return f"{v:,.4g}"


def load(path, sheet):
    """Returns (header, rows, notes); rows are lists of strings."""
    ext = os.path.splitext(path)[1].lower()
    notes = []
    if ext == ".xlsx":
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        import xlsx_to_csv  # noqa: E402  (the sibling helper)
        with zipfile.ZipFile(path) as book:
            found = xlsx_to_csv.sheets(book)
            notes.append("sheets: " + ", ".join(f"{i + 1}. {n}" for i, (n, _) in enumerate(found)))
            pick = sheet or "1"
            chosen = next(((n, p) for n, p in found if n == pick), None)
            if chosen is None and pick.isdigit() and 0 < int(pick) <= len(found):
                chosen = found[int(pick) - 1]
            if chosen is None:
                sys.exit(f"no sheet {pick!r}; sheets: {', '.join(n for n, _ in found)}")
            notes.append(f"sheet: {chosen[0]}")
            table = list(xlsx_to_csv.rows(book, chosen[1], xlsx_to_csv.shared_strings(book)))
    elif ext == ".json":
        with open(path, encoding="utf-8-sig") as f:
            data = json.load(f)
        if isinstance(data, dict):
            key = next((k for k, v in data.items() if isinstance(v, list)), None)
            if key is None:
                sys.exit("JSON: expected a list of objects (or an object holding one)")
            notes.append(f"records under key {key!r}")
            data = data[key]
        if not isinstance(data, list):
            sys.exit("JSON: expected a list of objects")
        names = []
        for item in data:
            if isinstance(item, dict):
                names.extend(k for k in item if k not in names)
        table = [names] + [["" if not isinstance(item, dict) or item.get(k) is None else
                            (json.dumps(item[k]) if isinstance(item[k], (dict, list)) else str(item[k]))
                            for k in names] for item in data]
    else:
        with open(path, newline="", encoding="utf-8-sig") as f:
            text = f.read()
        delimiter = "\t" if ext == ".tsv" else None
        if delimiter is None:
            try:
                delimiter = csv.Sniffer().sniff(text[:20000], delimiters=",;\t|").delimiter
            except csv.Error:
                delimiter = ","
        if delimiter != ",":
            notes.append(f"delimiter: {delimiter!r}")
        table = list(csv.reader(io.StringIO(text), delimiter=delimiter))
    if not table:
        sys.exit("the file has no rows")
    header, rows = table[0], table[1:]
    width = max([len(header)] + [len(r) for r in rows])
    header = header + [f"column_{i + 1}" for i in range(len(header), width)]
    rows = [r + [""] * (width - len(r)) for r in rows]
    return header, rows, notes


def describe(name, values):
    """One line about a column."""
    filled = [v for v in values if v.strip()]
    missing = len(values) - len(filled)
    distinct = len(set(filled))
    nums = [(i, number(v)) for i, v in enumerate(values) if v.strip()]
    numeric = [n for _, n in nums if n is not None]
    odd = [(i, values[i]) for i, n in nums if n is None]
    head = f"{short(name)}: "
    if filled and len(numeric) >= 0.8 * len(filled) and numeric:
        total = sum(numeric)
        line = (f"{head}number, {len(filled)} filled, {missing} missing, {distinct} distinct; "
                f"min {fmt(min(numeric))}, max {fmt(max(numeric))}, mean {fmt(total / len(numeric))}, "
                f"sum {fmt(total)}")
        return line, odd[:3]
    if filled and sum(1 for v in filled if DATE.match(v.strip())) >= 0.8 * len(filled):
        dates = sorted(v.strip() for v in filled if DATE.match(v.strip()))
        return (f"{head}date, {len(filled)} filled, {missing} missing, {distinct} distinct; "
                f"from {short(dates[0])} to {short(dates[-1])}"), []
    counts = Counter(v.strip() for v in filled).most_common(4)
    if counts and counts[0][1] > 1:
        top = "top: " + ", ".join(f"{short(v)} ({n})" for v, n in counts)
    else:
        top = f"all different, e.g. {short(filled[0])!r}" if filled else "all empty"
    return f"{head}text, {len(filled)} filled, {missing} missing, {distinct} distinct; {top}", []


def clip_line(line, limit=LINE):
    return line if len(line) <= limit else line[: limit - 1] + "\u2026"


def main(argv):
    args = argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return
    path, sheet, out_csv, show = args[0], None, None, 5
    i = 1
    while i < len(args):
        if args[i] in ("--sheet", "--csv", "--rows") and i + 1 < len(args):
            if args[i] == "--sheet":
                sheet = args[i + 1]
            elif args[i] == "--csv":
                out_csv = args[i + 1]
            else:
                show = max(0, min(20, int(args[i + 1])))
            i += 2
        else:
            sys.exit(f"unknown argument {args[i]!r} (see --help)")
    if not os.path.isfile(path):
        sys.exit(f"no file {path!r} in {os.getcwd()} (run from the session's working folder, or give the full path)")
    try:
        header, rows, notes = load(path, sheet)
    except (OSError, UnicodeDecodeError, ValueError, KeyError, zipfile.BadZipFile, json.JSONDecodeError) as e:
        sys.exit(f"cannot read {path!r}: {type(e).__name__}: {str(e)[:200]}")
    lines = [f"file: {path} ({os.path.getsize(path):,} bytes)"] + notes
    if out_csv:
        with open(out_csv, "w", newline="", encoding="utf-8") as f:
            writer = csv.writer(f, lineterminator="\n")
            writer.writerow(header)
            writer.writerows(rows)
        lines.append(f"wrote {out_csv} ({len(rows):,} rows)")
    keys = [tuple(r) for r in rows]
    empty = sum(1 for r in rows if not any(c.strip() for c in r))
    dupes = len(keys) - len(set(keys))
    lines.append(f"rows: {len(rows):,} (plus the header), columns: {len(header)}, "
                 f"duplicate rows: {dupes}, empty rows: {empty}")
    # Each part has its own budget, so many or wide columns never push out the rows or the "more columns" note.
    tail = []
    odd_lines = []
    column_lines = []
    for c, name in enumerate(header[:MAX_COLUMNS]):
        name = name.strip() or f"column_{c + 1}"
        line, odd = describe(name, [r[c] for r in rows])
        column_lines.append("  " + clip_line(line))
        odd_lines += [f"  {short(name)} row {r + 2}: {short(v)!r} (text in a number column)" for r, v in odd]
    if odd_lines:
        tail.append("odd values (first 3 per column; row numbers count the header as row 1):")
        used = 0
        for line in odd_lines:
            if used + len(line) > ODD_BUDGET:
                tail.append(f"  ... and {len(odd_lines) - odd_lines.index(line)} more")
                break
            tail.append(line)
            used += len(line) + 1
    if show:
        tail.append(f"first {min(show, len(rows))} rows:")
        buf = io.StringIO()
        writer = csv.writer(buf, lineterminator="\n")
        writer.writerow([short(h) for h in header[:MAX_COLUMNS]])
        for r in rows[:show]:
            writer.writerow([short(v) for v in r[:MAX_COLUMNS]])
        used = 0
        for line in buf.getvalue().splitlines():
            line = "  " + clip_line(line)
            if used + len(line) > ROWS_BUDGET:
                break
            tail.append(line)
            used += len(line) + 1
    budget = MAX_OUT - len("\n".join(lines + tail)) - 400
    lines.append("columns:")
    shown = 0
    for line in column_lines:
        if len(line) + 1 > budget:
            break
        lines.append(line)
        budget -= len(line) + 1
        shown += 1
    if shown < len(header):
        rest = [h.strip() or f"column_{i + 1}" for i, h in enumerate(header)][shown:]
        lines.append(clip_line(f"  ... and {len(rest)} more columns: " + ", ".join(short(h) for h in rest), 380))
    lines += tail
    text = "\n".join(lines)
    if len(text) > MAX_OUT:
        text = text[: MAX_OUT - 80].rsplit("\n", 1)[0] + f"\n... (cut at {MAX_OUT:,} characters)"
    print(text)


if __name__ == "__main__":
    main(sys.argv)
