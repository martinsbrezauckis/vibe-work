"""Vibe Work data-analysis helper: one sheet of an .xlsx workbook to CSV, with the Python standard library only.

Usage: python3 xlsx_to_csv.py BOOK.xlsx [SHEET_NAME_OR_NUMBER] > out.csv
       python3 xlsx_to_csv.py BOOK.xlsx --list          (sheet names, one per line)
       python3 xlsx_to_csv.py BOOK.xlsx [--sheet NAME] [--csv OUT.csv]   (the flags profile.py takes; writes OUT.csv)

Reads cell values as stored (numbers, shared and inline strings, booleans); formulas give their cached value; dates
stay Excel serial numbers (convert with: datetime(1899, 12, 30) + timedelta(days=value)). No macros run; no network.
Limits: at most 200 MB of XML per sheet and 1,000,000 rows.
"""
import csv
import re
import sys
import zipfile
import xml.etree.ElementTree as ET

NS = {"m": "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
      "r": "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
      "p": "http://schemas.openxmlformats.org/package/2006/relationships"}
MAX_XML = 200 * 1024 * 1024
MAX_ROWS = 1_000_000


def read(book, name):
    info = book.getinfo(name)
    if info.file_size > MAX_XML:
        sys.exit(f"{name} is too large ({info.file_size} bytes)")
    return book.read(name)


def sheets(book):
    wb = ET.fromstring(read(book, "xl/workbook.xml"))
    rels = ET.fromstring(read(book, "xl/_rels/workbook.xml.rels"))
    target = {r.get("Id"): r.get("Target") for r in rels.findall("p:Relationship", NS)}
    out = []
    for s in wb.findall("m:sheets/m:sheet", NS):
        rid = s.get("{%s}id" % NS["r"])
        path = target.get(rid, "").lstrip("/")
        out.append((s.get("name"), path if path.startswith("xl/") else "xl/" + path))
    return out


def shared_strings(book):
    if "xl/sharedStrings.xml" not in book.namelist():
        return []
    root = ET.fromstring(read(book, "xl/sharedStrings.xml"))
    return ["".join(t.text or "" for t in si.iter("{%s}t" % NS["m"])) for si in root.findall("m:si", NS)]


def column(ref):
    letters = re.match(r"[A-Z]+", ref or "")
    n = 0
    for ch in letters.group(0) if letters else "A":
        n = n * 26 + (ord(ch) - 64)
    return n - 1


def rows(book, path, strings):
    root = ET.fromstring(read(book, path))
    for count, row in enumerate(root.iter("{%s}row" % NS["m"])):
        if count >= MAX_ROWS:
            sys.exit(f"more than {MAX_ROWS} rows")
        values = {}
        for c in row.findall("m:c", NS):
            kind = c.get("t")
            v = c.find("m:v", NS)
            if kind == "inlineStr":
                text = "".join(t.text or "" for t in c.iter("{%s}t" % NS["m"]))
            elif v is None:
                text = ""
            elif kind == "s":
                text = strings[int(v.text)] if v.text and int(v.text) < len(strings) else ""
            elif kind == "b":
                text = "TRUE" if v.text == "1" else "FALSE"
            else:
                text = v.text or ""
            values[column(c.get("r"))] = text
        width = max(values) + 1 if values else 0
        yield [values.get(i, "") for i in range(width)]


def main(argv):
    if len(argv) < 2:
        sys.exit(__doc__)
    # The flags profile.py takes (models mix the two helpers up): --sheet NAME, --csv OUT.
    out_path = None
    rest = [argv[0], argv[1]]
    i = 2
    while i < len(argv):
        if argv[i] in ("--sheet", "--csv") and i + 1 < len(argv):
            if argv[i] == "--sheet":
                rest.append(argv[i + 1])
            else:
                out_path = argv[i + 1]
            i += 2
        else:
            rest.append(argv[i])
            i += 1
    argv = rest
    if out_path:
        with open(out_path, "w", newline="", encoding="utf-8") as out:
            stdout, sys.stdout = sys.stdout, out
            try:
                convert(argv)
            finally:
                sys.stdout = stdout
        print(f"wrote {out_path}")
        return
    convert(argv)


def convert(argv):
    with zipfile.ZipFile(argv[1]) as book:
        found = sheets(book)
        if len(argv) > 2 and argv[2] == "--list":
            print("\n".join(name for name, _ in found))
            return
        pick = argv[2] if len(argv) > 2 else "1"
        chosen = next((p for n, p in found if n == pick), None)
        if chosen is None and pick.isdigit() and 0 < int(pick) <= len(found):
            chosen = found[int(pick) - 1][1]
        if chosen is None:
            sys.exit(f"no sheet {pick!r}; sheets: {', '.join(n for n, _ in found)}")
        writer = csv.writer(sys.stdout, lineterminator="\n")
        for row in rows(book, chosen, shared_strings(book)):
            writer.writerow(row)


if __name__ == "__main__":
    main(sys.argv)
