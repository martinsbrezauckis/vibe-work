"""Vibe Work data-analysis helper: a bar or line chart as inline SVG for a handbook page, Python standard library only.

Usage: python3 svg_chart.py DATA.csv LABEL_COLUMN VALUE_COLUMN [--kind bar|hbar|line] [--title TEXT] [--unit TEXT]
                            [--agg sum|mean|count] [--series COLUMN] [--out FILE]

--out FILE writes the chart there and prints one line (what was written); give the file to report_page.py with a
`<!-- chart: FILE -->` marker, so the SVG never has to pass through the conversation. Without --out the chart goes to
stdout (old behaviour).
--agg groups the rows by LABEL_COLUMN (first-seen order) and sums, averages or counts VALUE_COLUMN, so a chart can come
straight from the raw data. --series COLUMN (with --agg) makes one series per distinct value of COLUMN (at most 4),
e.g. revenue by month with one line per region.
Several value columns without --series: VALUE_COLUMN may be "a,b" (grouped bars or one line each, up to 4).
The SVG uses the handbook's colour tokens (var(--blue), var(--text-2), ...) so it reads in light and dark themes, has a
<title> and an aria-label, and carries no script. At most 60 labels (aggregate first).
"""
import argparse
import csv
import html
import os
import sys

COLOURS = ["var(--blue)", "var(--accent)", "var(--green)", "var(--red)"]
MAX_LABELS = 60
MAX_SERIES = 4


def number(text):
    t = (text or "").strip().replace(",", "").replace("%", "").replace("€", "").replace("$", "")
    try:
        return float(t)
    except ValueError:
        return None


def nice(v):
    return f"{v:,.0f}" if abs(v) >= 100 else f"{v:,.2f}".rstrip("0").rstrip(".")


def aggregate(data, label, value, how, series_col):
    """Groups `data` by `label` (and `series_col`): returns (labels, series names, values per series)."""
    order, names, cells = [], [], {}
    for row in data:
        key = row[label]
        name = row[series_col] if series_col else value
        if key not in order:
            order.append(key)
        if name not in names:
            names.append(name)
        v = number(row[value]) if how != "count" else 1.0
        if v is None:
            continue
        cells.setdefault((key, name), []).append(v)
    if len(names) > MAX_SERIES:
        sys.exit(f"{len(names)} series in {series_col!r}: at most {MAX_SERIES} (filter or aggregate first)")

    def reduce(vals):
        if not vals:
            return None
        return sum(vals) / len(vals) if how == "mean" else sum(vals)

    return order, names, [[reduce(cells.get((k, n), [])) for k in order] for n in names]


def render(labels, series, values, kind, title, unit):
    flat = [v for col in values for v in col if v is not None]
    if not flat:
        sys.exit("no numbers in the value columns")
    top = max(max(flat), 0.0) or 1.0
    low = min(min(flat), 0.0)
    span = top - low or 1.0
    w, h, left, right, up, down = 720, 360, 64, 16, 36, 70
    pw, ph = w - left - right, h - up - down
    title = html.escape(title or f"{', '.join(series)} by label")
    out = [f'<figure class="hb-chart"><svg viewBox="0 0 {w} {h}" width="100%" role="img" aria-label="{title}" '
           f'xmlns="http://www.w3.org/2000/svg" style="max-width:{w}px;font:12px system-ui,sans-serif">',
           f"<title>{title}</title>"]
    y = lambda v: up + ph - (v - low) / span * ph  # noqa: E731
    for i in range(5):
        tick = low + span * i / 4
        out.append(f'<line x1="{left}" x2="{w - right}" y1="{y(tick):.1f}" y2="{y(tick):.1f}" style="stroke:var(--line)"/>')
        out.append(f'<text x="{left - 6}" y="{y(tick) + 4:.1f}" text-anchor="end" style="fill:var(--text-3)">{nice(tick)}</text>')
    n = len(labels)
    slot = pw / max(n, 1)
    if kind == "line":
        for s, col in enumerate(values):
            pts = [f"{left + slot * (i + 0.5):.1f},{y(v):.1f}" for i, v in enumerate(col) if v is not None]
            out.append(f'<polyline points="{" ".join(pts)}" fill="none" style="stroke:{COLOURS[s]};stroke-width:2"/>')
    else:
        bw = slot * 0.8 / len(values)
        for s, col in enumerate(values):
            for i, v in enumerate(col):
                if v is None:
                    continue
                x = left + slot * i + slot * 0.1 + bw * s
                y0, y1 = sorted([y(v), y(0)])
                out.append(f'<rect x="{x:.1f}" y="{y0:.1f}" width="{bw:.1f}" height="{max(y1 - y0, 0.5):.1f}" '
                           f'style="fill:{COLOURS[s]}"><title>{html.escape(labels[i])}: {nice(v)}{html.escape(unit)}</title></rect>')
    every = max(1, n // 12)
    for i, label in enumerate(labels):
        if i % every == 0:
            x = left + slot * (i + 0.5)
            text = html.escape(label if len(label) <= 14 else label[:13] + "…")
            out.append(f'<text x="{x:.1f}" y="{h - down + 16}" text-anchor="end" transform="rotate(-35 {x:.1f} {h - down + 16})" '
                       f'style="fill:var(--text-2)">{text}</text>')
    if len(series) > 1:
        for s, name in enumerate(series):
            out.append(f'<rect x="{left + s * 140}" y="8" width="10" height="10" style="fill:{COLOURS[s]}"/>'
                       f'<text x="{left + s * 140 + 14}" y="17" style="fill:var(--text-2)">{html.escape(name)}</text>')
    out.append("</svg>")
    out.append(f"<figcaption>{title}{' (' + html.escape(unit) + ')' if unit else ''}</figcaption></figure>")
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("csv")
    ap.add_argument("label")
    ap.add_argument("values")
    ap.add_argument("--kind", default="bar", choices=["bar", "hbar", "line"])
    ap.add_argument("--title", default="")
    ap.add_argument("--unit", default="")
    ap.add_argument("--agg", choices=["sum", "mean", "count"])
    ap.add_argument("--series")
    ap.add_argument("--out")
    a = ap.parse_args()
    with open(a.csv, newline="", encoding="utf-8-sig") as f:
        data = list(csv.DictReader(f))
    series = [s.strip() for s in a.values.split(",") if s.strip()][:MAX_SERIES]
    for name in [a.label, *series, *([a.series] if a.series else [])]:
        if data and name not in data[0]:
            sys.exit(f"no column {name!r}; columns: {', '.join(data[0])}")
    if a.series and not a.agg:
        sys.exit("--series needs --agg (sum, mean or count)")
    if a.agg:
        if len(series) != 1:
            sys.exit("--agg takes one value column")
        labels, series, values = aggregate(data, a.label, series[0], a.agg, a.series)
    else:
        labels = [r[a.label] for r in data]
        values = [[number(r[s]) for r in data] for s in series]
    if len(labels) > MAX_LABELS:
        sys.exit(f"{len(labels)} labels: aggregate to at most {MAX_LABELS} first (--agg, or a grouped CSV)")
    title = a.title or f"{', '.join(series)} by {a.label}"
    figure = render(labels, series, values, a.kind, title, a.unit)
    if not a.out:
        print(figure)
        return
    folder = os.path.dirname(a.out)
    if folder:
        os.makedirs(folder, exist_ok=True)
    with open(a.out, "w", encoding="utf-8", newline="\n") as f:
        f.write(figure + "\n")
    print(f"wrote {a.out}: {a.kind} chart, {len(labels)} labels x {len(series)} series "
          f"({', '.join(series)}), {len(figure):,} bytes")


if __name__ == "__main__":
    main()
