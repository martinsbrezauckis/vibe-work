"""Vibe Work data-analysis helper: turns a short report body into a complete handbook report page (Project Handbook
Format 1.2), with the charts filled in, and lists it in the handbook index. Python standard library only.

Usage: python3 report_page.py BODY.html --title TEXT --description TEXT [--slug SLUG] [--date YYYY-MM-DD]
                              [--status final|draft] [--outcome pass|fail|mixed|info] [--handbook docs/handbook]

BODY.html holds only the content: an optional first paragraph, then <section id="scope">, <section id="method"> and
<section id="results">, each starting with an <h2>. Put a chart where it belongs with a marker line
    <!-- chart: analysis/revenue.svg.html -->
(a file made by svg_chart.py --out; the path is relative to the current folder). The helper writes
<handbook>/reports/<date>-<slug>.html with the head, header and links the format needs, creates the handbook
(index.html) when the project has none, and adds or updates the page's line
in the index's "reports" section. Run it again after changing BODY.html: it rewrites the page and keeps its created
date. The handbook tool installs the bundled stylesheet. It prints one line per file it wrote, never the page itself.
"""
import argparse
import datetime
import html
import os
import re
import sys
from html.parser import HTMLParser

REQUIRED = ("scope", "method", "results")
MARKER = re.compile(r"<!--\s*chart:\s*([^\s>]+)\s*-->")
SLUG = re.compile(r"[^a-z0-9]+")
# Tags that run code, embed another page or redirect the reader, and the tags a browser reads as raw text (where the
# check below would see markup the browser does not, or the reverse); a page made from data needs none of them.
ACTIVE_TAGS = {"script", "iframe", "frame", "frameset", "object", "embed", "applet", "meta", "base", "style",
               "noscript", "xmp", "noembed", "noframes", "plaintext", "textarea"}
PAGE_TAGS = {"html", "head", "body", "h1"}
CODE_SCHEMES = ("javascript:", "vbscript:")


def die(message):
    sys.exit(f"report_page.py: {message}")


def inside(root, path):
    """True when `path` is `root` or below it (both already real paths)."""
    try:
        return os.path.commonpath([root, path]) == root
    except ValueError:  # another drive on Windows
        return False


class _Scan(HTMLParser):
    """Collects what a browser would treat as active content: the tags above, on* attributes, script links."""

    def __init__(self):
        super().__init__(convert_charrefs=False)  # raw text reaches handle_data; attributes are still decoded
        self.found = []

    def handle_starttag(self, tag, attrs):
        if tag in ACTIVE_TAGS:
            self.found.append(f"<{tag}>")
        if tag in PAGE_TAGS:
            self.found.append(f"page:<{tag}>")
        if "<" in (self.get_starttag_text() or "")[1:]:
            self.found.append("markup inside a tag")
        for name, value in attrs:
            if name.startswith("on"):
                self.found.append("on* attributes")
            # Browsers drop spaces and control characters inside a URL scheme ("jav\tascript:"); an SVG animation
            # value list ("0;javascript:...") sets a link too, so the scheme counts anywhere in the value.
            squeezed = "".join(ch for ch in (value or "") if ord(ch) > 32).lower()
            if any(scheme in squeezed for scheme in CODE_SCHEMES):
                self.found.append("javascript: links")

    handle_startendtag = handle_starttag

    def handle_data(self, data):
        # Text the parser reads as raw (a <title> on newer Pythons) is markup to a browser inside SVG.
        if getattr(self, "cdata_elem", None) and "<" in data:
            self.found.append("markup inside a tag")

    def handle_comment(self, data):
        # Only chart markers: a browser may end a comment earlier ("<!-->") and read the rest as markup.
        if not re.fullmatch(r"\s*chart:\s*[^\s>]+\s*", data):
            self.found.append("comments other than chart markers")

    def handle_decl(self, data):
        self.found.append("declarations")

    handle_pi = unknown_decl = handle_decl


def problems(text):
    """The active-content findings in an HTML fragment, in document order, each once."""
    scan = _Scan()
    scan.feed(text)
    # An unfinished tag at the end would close against the page's own markup after it.
    if "<" in scan.rawdata:
        scan.found.append("an unfinished tag")
    scan.close()
    return list(dict.fromkeys(scan.found))


def fill_charts(body):
    """Replaces every chart marker with the chart file's figure; refuses files that are not a chart."""
    root = os.path.realpath(os.getcwd())

    def one(match):
        rel = match.group(1)
        path = os.path.realpath(os.path.join(root, rel))
        if os.path.isabs(rel) or not inside(root, path):
            die(f"chart {rel!r}: give a path inside the current folder")
        try:
            with open(path, encoding="utf-8") as f:
                figure = f.read().strip()
        except OSError:
            die(f"chart {rel!r} not found (make it with svg_chart.py --out {rel})")
        if not figure.startswith(("<figure", "<svg")) or problems(figure):
            die(f"chart {rel!r} is not a chart made by svg_chart.py")
        return figure

    return MARKER.sub(one, body)


def check_body(body):
    found = problems(body)
    if any(f.startswith("page:") for f in found):
        die("BODY.html holds only the content (sections), not <html>, <head>, <body> or <h1>")
    if found:
        die(f"BODY.html has {', '.join(found)}: a report page holds no <script>, no on* attributes, "
            "no embedded pages or redirects and no javascript: links")
    missing = [s for s in REQUIRED if not re.search(rf'<section\s[^>]*id="{s}"', body)]
    if missing:
        die("missing " + ", ".join(f'<section id="{s}">' for s in missing))


def page_html(title, description, status, created, modified, outcome, body):
    t, d = html.escape(title), html.escape(description, quote=True)
    extra = f'\n<meta name="handbook.outcome" content="{outcome}">' if outcome else ""
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{t}</title>
<meta name="handbook.format" content="1.2">
<meta name="handbook.kind" content="report">
<meta name="handbook.status" content="{status}">{extra}
<meta name="dcterms.created" content="{created}">
<meta name="dcterms.modified" content="{modified}">
<meta name="description" content="{d}">
<link rel="stylesheet" href="../handbook.css">
</head>
<body>
<header>
<p class="hb-kind">Report</p>
<h1>{t}</h1>
<p class="hb-meta"><span data-field="status">{status.capitalize()}</span> · updated <time data-field="modified" datetime="{modified}">{modified}</time></p>
<nav class="hb-related" aria-label="Related pages"><a rel="related" href="../index.html">Handbook</a></nav>
</header>
<main>
{body.strip()}
</main>
</body>
</html>
"""


def new_index(project, today):
    p = html.escape(project)
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{p} handbook</title>
<meta name="handbook.format" content="1.2">
<meta name="handbook.kind" content="index">
<meta name="handbook.project" content="{p}">
<meta name="handbook.status" content="active">
<meta name="dcterms.created" content="{today}">
<meta name="dcterms.modified" content="{today}">
<meta name="description" content="Plans, decisions and reports of {p}.">
<link rel="stylesheet" href="handbook.css">
</head>
<body>
<header>
<p class="hb-kind">Handbook</p>
<h1>{p} handbook</h1>
<p class="hb-meta"><span data-field="status">Active</span> · updated <time data-field="modified" datetime="{today}">{today}</time></p>
</header>
<main>
<p>Plans, decisions and reports of {p}.</p>
<section id="reports"><h2>Reports</h2><ul></ul></section>
<section id="archive"><h2>Archive</h2><details><summary>Closed pages</summary><ul></ul></details></section>
</main>
</body>
</html>
"""


def update_index(index, href, line, today):
    """Adds or replaces the page's line in the "reports" section; returns the new text or None when unsure."""
    existing = re.search(rf'<li>\s*<a href="{re.escape(href)}"[^\n]*?</li>', index)
    if existing:
        index = index[: existing.start()] + line + index[existing.end():]
    else:
        section = re.search(r'<section\s[^>]*id="reports"[^>]*>', index)
        if section:
            close = index.find("</ul>", section.end())
            next_section = index.find("<section", section.end())
            if close == -1 or (next_section != -1 and next_section < close):
                end = index.find("</section>", section.end())
                if end == -1:
                    return None
                index = index[:end] + f"<ul>{line}</ul>" + index[end:]
            else:
                index = index[:close] + line + index[close:]
        else:
            block = f'<section id="reports"><h2>Reports</h2><ul>{line}</ul></section>\n'
            anchor = re.search(r'<section\s[^>]*id="archive"', index)
            pos = anchor.start() if anchor else index.find("</main>")
            if pos == -1:
                return None
            index = index[:pos] + block + index[pos:]
    index = re.sub(r'(<meta name="dcterms\.modified" content=")[^"]*(")', rf"\g<1>{today}\g<2>", index, count=1)
    index = re.sub(r'(<time data-field="modified" datetime=")[^"]*(">)[^<]*(</time>)',
                   rf"\g<1>{today}\g<2>{today}\g<3>", index, count=1)
    return index


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("body")
    ap.add_argument("--title", required=True)
    ap.add_argument("--description", required=True)
    ap.add_argument("--slug")
    ap.add_argument("--date", default=datetime.date.today().isoformat())
    ap.add_argument("--status", default="final", choices=["final", "draft"])
    ap.add_argument("--outcome", choices=["pass", "fail", "mixed", "info"])
    ap.add_argument("--handbook", default="docs/handbook")
    a = ap.parse_args()
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", a.date):
        die("--date is YYYY-MM-DD")
    slug = SLUG.sub("-", (a.slug or a.title).lower()).strip("-")[:60].strip("-") or "report"
    with open(a.body, encoding="utf-8") as f:
        body = f.read()
    check_body(body)
    charts = len(MARKER.findall(body))
    body = fill_charts(body)
    check_body(body)
    title, description = " ".join(a.title.split()), " ".join(a.description.split())
    hb = a.handbook
    root = os.path.realpath(os.getcwd())
    if not inside(root, os.path.realpath(hb)):
        die(f"--handbook {hb!r}: give a folder inside the current folder")
    rel = f"reports/{a.date}-{slug}.html"
    path = os.path.join(hb, rel)
    for target in (path, os.path.join(hb, "index.html")):
        if not inside(root, os.path.realpath(target)):
            die(f"{target!r} leads outside the current folder (a link); give a handbook folder inside it")
    os.makedirs(os.path.join(hb, "reports"), exist_ok=True)
    created = a.date
    if os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            old = re.search(r'<meta name="dcterms\.created" content="(\d{4}-\d{2}-\d{2})"', f.read())
        created = old.group(1) if old else created
    today = datetime.date.today().isoformat()
    modified = max(today, created)
    wrote = []
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(page_html(title, description, a.status, created, modified, a.outcome, body))
    wrote.append(f"{path} ({os.path.getsize(path):,} bytes, {charts} charts)")
    index_path = os.path.join(hb, "index.html")
    line = (f'<li><a href="{rel}">{html.escape(title)}</a> — {a.status} · {html.escape(description)}</li>')
    if os.path.exists(index_path):
        with open(index_path, encoding="utf-8") as f:
            index = update_index(f.read(), rel, line, today)
        if index is None:
            print("\n".join(f"wrote {w}" for w in wrote))
            print(f"index.html: add this line to its reports section yourself: {line}")
            return
    else:
        project = os.path.basename(os.path.realpath(os.getcwd())) or "Project"
        index = update_index(new_index(project, today), rel, line, today)
    with open(index_path, "w", encoding="utf-8", newline="\n") as f:
        f.write(index)
    wrote.append(f"{index_path} (lists the report)")
    print("\n".join(f"wrote {w}" for w in wrote))
    print("next: handbook_check({repair: \"stylesheet\"}), then handbook_visual_check on " + rel)


if __name__ == "__main__":
    main()
