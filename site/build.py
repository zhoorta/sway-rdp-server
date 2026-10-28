#!/usr/bin/env python3
"""Builds the documentation of waydesk.dev (site/docs/) from site/pages/*.md and *.html (standard library only): `python3 site/build.py`.

Each page is `key: value` lines, a line `---`, then Markdown of a small kind: `##` starts a section (`## Title {bar: label}` is accepted and the label ignored), `###` a sub-heading, paragraphs, `-` and `1.` lists, pipe tables, `>` notes, fenced code (```console puts the prompt and comments in
colour; ```html is passed through). A page that is a file `name.html` has the same front matter (and a `lede:` line) and a body of HTML sections written by hand: the charts and diagrams of the architecture, measurements, security and reference pages. The output goes to site/<slug>/index.html, with the header, footer and stylesheet of the home page, and sitemap.xml lists the home page and the
pages. The generated files are committed (the site is a folder that is copied as it is), so run this after editing a page and commit both. Deploy with
`rsync -a --exclude=.DS_Store --exclude=build.py --exclude=pages -e "ssh -i ~/.ssh/id_ed25519_metproof -o IdentitiesOnly=yes" site/ waydesk@2.28.140.203:app/` (no --delete: the apt repository lives in app/apt).
"""
import html
import re
import sys
from pathlib import Path

SITE = Path(__file__).resolve().parent
PAGES = SITE / "pages"
BASE = "https://waydesk.dev"
CAP = '<span class="cap" aria-hidden="true"><i>&#x2013;</i><i>&#x25a1;</i><i>&#x2715;</i></span>'
GH_ICON = (SITE / "index.html").read_text().split('<a class="gh" href="https://github.com/zhoorta/waydesk">')[1].split("</a>")[0]

# The documentation, in groups: the order of the sidebar, of the index and of "next page".
GROUPS = [
    ("Get started", ["install", "connect", "vpn"]),
    ("Run it", ["cli", "gpu", "reference"]),
    ("Understand it", ["architecture", "measurements", "compare", "limits"]),
    ("Help", ["help", "faq"]),
]
ORDER = [s for _, ss in GROUPS for s in ss]


def inline(text):
    """Escapes and marks up one line of text: `code`, **bold**, [link](url) (the text of a link may hold `code`)."""
    out = []
    for p in re.split(r"(\[(?:[^\]`]|`[^`]*`)+\]\([^)\s]+\)|`[^`]+`)", text):
        m = re.fullmatch(r"\[((?:[^\]`]|`[^`]*`)+)\]\(([^)\s]+)\)", p)
        if m:
            out.append(f'<a href="{m.group(2)}">{inline(m.group(1))}</a>')
        elif p.startswith("`") and p.endswith("`") and len(p) > 1:
            out.append("<code>" + html.escape(p[1:-1], quote=False) + "</code>")
        else:
            out.append(re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", html.escape(p, quote=False)))
    return "".join(out)


def console(code):
    """A terminal block: `$ command  # comment` gets the prompt and the comment in colour; other lines are output."""
    lines = []
    for line in code.split("\n"):
        if line.startswith("$ "):
            cmd, _, comment = line[2:].partition("  # ")
            s = '<span class="p">$</span> ' + html.escape(cmd, quote=False)
            if comment:
                s += '  <span class="c"># ' + html.escape(comment, quote=False) + "</span>"
            lines.append(s)
        else:
            lines.append(html.escape(line, quote=False))
    return '<pre><code>' + "\n".join(lines) + "</code></pre>"


def split_row(row):
    """The cells of one table row; `\\|` is a pipe inside a cell (the commands of waydesk-admin list their alternatives with it)."""
    return [c.replace("\\|", "|").strip() for c in re.split(r"(?<!\\)\|", row.strip().strip("|"))]


def table(rows):
    cells = [split_row(r) for r in rows]
    head, body = cells[0], cells[2:]
    th = "".join(f"<th>{inline(c)}</th>" for c in head)
    trs = "".join("<tr>" + "".join(f"<td>{inline(c)}</td>" for c in row) + "</tr>" for row in body)
    return f'<div class="tw"><table><tr>{th}</tr>{trs}</table></div>'


def blocks(md):
    """Markdown (without ## headings) to HTML."""
    out, para, lines, i = [], [], md.split("\n"), 0

    def flush():
        if para:
            out.append("<p>" + inline(" ".join(para)) + "</p>")
            para.clear()

    while i < len(lines):
        line = lines[i]
        if line.startswith("```"):
            flush()
            lang, buf = line[3:].strip(), []
            i += 1
            while i < len(lines) and not lines[i].startswith("```"):
                buf.append(lines[i])
                i += 1
            code = "\n".join(buf)
            out.append(console(code) if lang == "console" else code if lang == "html" else "<pre><code>" + html.escape(code, quote=False) + "</code></pre>")
        elif line.startswith("### "):
            flush()
            out.append("<h3>" + inline(line[4:]) + "</h3>")
        elif line.startswith("|"):
            flush()
            rows = []
            while i < len(lines) and lines[i].startswith("|"):
                rows.append(lines[i])
                i += 1
            i -= 1
            out.append(table(rows))
        elif line.startswith("> "):
            flush()
            buf = []
            while i < len(lines) and lines[i].startswith("> "):
                buf.append(lines[i][2:])
                i += 1
            i -= 1
            out.append("<blockquote>" + inline(" ".join(buf)) + "</blockquote>")
        elif re.match(r"(- |\d+\. )", line):
            flush()
            ordered = line[0].isdigit()
            items = []
            while i < len(lines) and re.match(r"(- |\d+\. |  )", lines[i]) and lines[i].strip():
                if re.match(r"(- |\d+\. )", lines[i]):
                    items.append(re.sub(r"^(- |\d+\. )", "", lines[i]))
                else:
                    items[-1] += " " + lines[i].strip()
                i += 1
            i -= 1
            tag = "ol" if ordered else "ul"
            out.append(f'<{tag} class="plain">' + "".join(f"<li>{inline(x)}</li>" for x in items) + f"</{tag}>")
        elif not line.strip():
            flush()
        else:
            para.append(line.strip())
        i += 1
    flush()
    return "\n".join(out)


def parse(path):
    head, _, body = path.read_text().partition("\n---\n")
    meta = dict(l.split(": ", 1) for l in head.strip().split("\n") if ": " in l)
    if path.suffix == ".html":
        return meta, meta.get("lede", ""), body
    intro, sections, cur = [], [], None
    for line in body.split("\n"):
        m = re.match(r"## (.+?)(?: \{bar: (.+)\})?$", line)
        if m:
            cur = {"title": m.group(1), "bar": m.group(2) or "notes", "lines": []}
            sections.append(cur)
        elif cur is None:
            intro.append(line)
        else:
            cur["lines"].append(line)
    return meta, "\n".join(intro), sections


def slugify(t):
    return re.sub(r"[^a-z0-9]+", "-", t.lower()).strip("-")


GH = f'<a class="gh" href="https://github.com/zhoorta/waydesk">{GH_ICON}</a>'
FOOT = f"""<footer>
  <span>\u00a9 2026 StackCare</span>
  <a href="mailto:jose@stackcare.pt">jose@stackcare.pt</a>
  {GH}
  <a href="https://github.com/zhoorta/waydesk/blob/main/CHANGELOG.md">changelog</a>
  <a href="https://stackcare.pt/en">commercial support: StackCare</a>
  <span>MIT or Apache-2.0</span>
</footer>"""


def shell(title, desc, url, body, kind="article"):
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(title)} \u00b7 Waydesk</title>
<meta name="description" content="{html.escape(desc, quote=True)}">
<link rel="canonical" href="{BASE}{url}">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="icon" href="/favicon-32.png" sizes="32x32" type="image/png">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<meta name="theme-color" content="#0a1c2a">
<meta property="og:title" content="{html.escape(title)} \u00b7 Waydesk">
<meta property="og:description" content="{html.escape(desc, quote=True)}">
<meta property="og:url" content="{BASE}{url}">
<meta property="og:type" content="{kind}">
<link rel="stylesheet" href="/style.css">
</head>
<body>
<div class="col">
<header>
  <a class="logo" href="/"><img src="/logo-dark.svg" alt="Waydesk" width="159" height="35"></a>
  <nav aria-label="Site">
    <a href="/">home</a>
    <a href="/docs/"{' aria-current="page"' if url.startswith("/docs/") else ""}>documentation</a>
    {GH}
  </nav>
</header>
{body}
{FOOT}
</div>
</body>
</html>
"""


def sidebar(pages, current, sections):
    out = ['<div class="side">\n<nav aria-label="Documentation">']
    for name, slugs in GROUPS:
        out.append(f"<h2>{name}</h2>\n<ul>")
        for s in slugs:
            if s not in pages:
                continue
            cur = ' aria-current="page"' if s == current else ""
            li = f'<li><a href="/docs/{s}/"{cur}>{html.escape(pages[s]["nav"])}</a>'
            if s == current and sections:
                li += "<ul>" + "".join(f'<li><a href="#{slugify(x["title"])}">{inline(x["title"])}</a></li>' for x in sections) + "</ul>"
            out.append(li + "</li>")
        out.append("</ul>")
    out.append("</nav>\n</div>")
    return "\n".join(out)



def page_html(meta, intro, sections, pages, source_name):
    slug = meta["slug"]
    if isinstance(sections, str):
        secs, anchors = sections, []
        anchors = [{"title": t} for t in re.findall(r'<section[^>]*id="[^"]*"[^>]*>\s*<h2>(.*?)</h2>', sections)]
    else:
        secs = "\n".join(
            f'<section id="{slugify(s["title"])}">\n<h2>{inline(s["title"])}</h2>\n{blocks(chr(10).join(s["lines"]))}\n</section>' for s in sections
        )
        anchors = sections
    i = ORDER.index(slug)
    prev = f'<a class="prev" href="/docs/{ORDER[i-1]}/"><span>previous</span>{html.escape(pages[ORDER[i-1]]["title"])}</a>' if i > 0 else "<span></span>"
    nxt = f'<a class="next" href="/docs/{ORDER[i+1]}/"><span>next</span>{html.escape(pages[ORDER[i+1]]["title"])}</a>' if i + 1 < len(ORDER) else "<span></span>"
    body = f"""<div class="docs">
{sidebar(pages, slug, anchors)}
<main class="doc">
<div class="pagehead">
<h1>{inline(meta["title"])}</h1>
<p class="lede">{inline(intro.strip())}</p>
</div>
{secs}
<nav class="prevnext" aria-label="Previous and next page">{prev}{nxt}</nav>
<p class="method">This page is <a href="https://github.com/zhoorta/waydesk/blob/main/site/pages/{source_name}">a file in the repository</a>: corrections are welcome.</p>
</main>
</div>"""
    return shell(meta["title"], meta["description"], f"/docs/{slug}/", body)


def index_html(pages):
    cols = []
    for name, slugs in GROUPS:
        rows = "\n".join(
            f'<li><a href="/docs/{s}/">{html.escape(pages[s]["title"])}</a><span>{html.escape(pages[s]["description"])}</span></li>' for s in slugs if s in pages
        )
        cols.append(f"<section>\n<h2>{name}</h2>\n<ul class=\"toc\">\n{rows}\n</ul>\n</section>")
    body = f"""<main class="doc docindex">
<div class="pagehead">
<h1>Documentation</h1>
<p class="lede">Read it in order, or jump to what you need. New to Waydesk: start with <a href="/docs/install/">Install and first login</a>, it takes about ten minutes from a fresh server. Every page is a file in the repository, written from what was run on real servers.</p>
</div>
{chr(10).join(cols)}
</main>"""
    return shell("Documentation", "Waydesk documentation: install, connect, the VPN, the command line, the GPU, how it works, what was measured, limits and troubleshooting.", "/docs/", body)


def main():
    pages = {}
    for path in sorted(PAGES.glob("*.md")) + sorted(PAGES.glob("*.html")):
        meta, intro, sections = parse(path)
        meta["slug"] = meta.get("slug", path.stem)
        pages[meta["slug"]] = (meta, intro, sections, path.name)
    metas = {k: v[0] for k, v in pages.items()}
    for slug in ORDER:
        meta, intro, sections, name = pages[slug]
        out = SITE / "docs" / slug / "index.html"
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(page_html(meta, intro, sections, metas, name))
        print("wrote", out.relative_to(SITE))
    (SITE / "docs" / "index.html").write_text(index_html(metas))
    urls = [f"{BASE}/", f"{BASE}/docs/"] + [f"{BASE}/docs/{s}/" for s in ORDER]
    (SITE / "sitemap.xml").write_text('<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">' + "".join(f"<url><loc>{u}</loc></url>" for u in urls) + "</urlset>\n")
    print("wrote docs/index.html and sitemap.xml:", len(urls), "urls")
    unknown = set(metas) - set(ORDER)
    if unknown:
        sys.exit(f"pages that are in no group: {sorted(unknown)}")


if __name__ == "__main__":
    sys.exit(main())
