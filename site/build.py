#!/usr/bin/env python3
"""Builds the subpages of waydesk.dev from site/pages/*.md (standard library only): `python3 site/build.py`.

Each page is `key: value` lines, a line `---`, then Markdown of a small kind: `##` starts a section (shown in one of the site's windows, `## Title {bar: label}`
names the title bar), `###` a sub-heading, paragraphs, `-` and `1.` lists, pipe tables, `>` notes, fenced code (```console puts the prompt and comments in
colour). The output goes to site/<slug>/index.html, with the header, footer and stylesheet of the home page, and sitemap.xml lists the home page and the
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

# The order of the pages in the navigation and in the index.
NAV = ["start", "connect", "vpn", "cli", "gpu", "help"]


def inline(text):
    """Escapes and marks up one line of text: `code`, **bold**, [link](url)."""
    parts = re.split(r"(`[^`]+`)", text)
    out = []
    for p in parts:
        if p.startswith("`") and p.endswith("`") and len(p) > 1:
            out.append("<code>" + html.escape(p[1:-1], quote=False) + "</code>")
            continue
        p = html.escape(p, quote=False)
        p = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", p)
        p = re.sub(r"\[([^\]]+)\]\(([^)\s]+)\)", lambda m: f'<a href="{m.group(2)}">{m.group(1)}</a>', p)
        out.append(p)
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


def table(rows):
    cells = [[c.strip() for c in r.strip().strip("|").split("|")] for r in rows]
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
            out.append(console(code) if lang == "console" else "<pre><code>" + html.escape(code, quote=False) + "</code></pre>")
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


def page_html(meta, intro, sections, pages, source_name):
    def link(s):
        current = ' aria-current="page"' if s == meta["slug"] else ""
        return f'<a href="/{s}/"{current}>{pages[s][0]["nav"]}</a>'

    nav = "\n    ".join(link(s) for s in NAV if s in pages)
    secs = []
    for s in sections:
        secs.append(
            f'<section class="sect" id="{slugify(s["title"])}">\n<h2>{inline(s["title"])}</h2>\n<div class="win">\n'
            f'<div class="bar"><span class="t">{html.escape(s["bar"])}</span>{CAP}</div>\n<div class="body">\n{blocks(chr(10).join(s["lines"]))}\n</div>\n</div>\n</section>'
        )
    title, desc = meta["title"], meta["description"]
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(title)} · Waydesk</title>
<meta name="description" content="{html.escape(desc, quote=True)}">
<link rel="canonical" href="{BASE}/{meta['slug']}/">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="icon" href="/favicon-32.png" sizes="32x32" type="image/png">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
<meta name="theme-color" content="#0a1c2a">
<meta property="og:title" content="{html.escape(title)} · Waydesk">
<meta property="og:description" content="{html.escape(desc, quote=True)}">
<meta property="og:url" content="{BASE}/{meta['slug']}/">
<meta property="og:type" content="article">
<link rel="stylesheet" href="/style.css">
</head>
<body>
<div class="col">
<header>
  <a class="logo" href="/"><img src="/logo-dark.svg" alt="Waydesk" width="159" height="35"></a>
  <nav aria-label="Pages">
    <a href="/">home</a>
    {nav}
    <a href="/docs/">all pages</a>
    <a class="gh" href="https://github.com/zhoorta/waydesk">{GH_ICON}</a>
  </nav>
</header>
<main>
<section class="pagehead">
<h1>{inline(title)}</h1>
<p class="lede">{inline(intro.strip())}</p>
</section>
{chr(10).join(secs)}
<p class="method">This page is <a href="https://github.com/zhoorta/waydesk/blob/main/site/pages/{source_name}">a file in the repository</a>: corrections are welcome.</p>
</main>
<footer>
  <span>© 2026 StackCare</span>
  <a href="mailto:jose@stackcare.pt">jose@stackcare.pt</a>
  <a class="gh" href="https://github.com/zhoorta/waydesk">{GH_ICON}</a>
  <a href="https://github.com/zhoorta/waydesk/blob/main/CHANGELOG.md">changelog</a>
  <a href="https://stackcare.pt/en">commercial support: StackCare</a>
  <span>MIT or Apache-2.0</span>
</footer>
</div>
</body>
</html>
"""


def index_html(pages):
    cards = "\n".join(
        f'  <div class="win card">\n    <div class="bar"><span class="t">/{s}/</span>{CAP}</div>\n'
        f'    <div class="body"><h3><a href="/{s}/">{html.escape(pages[s][0]["title"])}</a></h3><p>{html.escape(pages[s][0]["description"])}</p></div>\n  </div>'
        for s in NAV if s in pages
    )
    meta = {"slug": "docs", "title": "All pages", "description": "Every page of waydesk.dev: getting started, connecting, the VPN, the command line, the GPU, and what to do when something is wrong."}
    nav = "\n    ".join(f'<a href="/{s}/">{pages[s][0]["nav"]}</a>' for s in NAV if s in pages)
    page = page_html(meta, "The home page is the overview; these pages go deeper. Each is a file in the repository, written from what was run on real servers.", [], pages, "")
    # the index has cards, not sections
    page = page.replace('<p class="method">This page is <a href="https://github.com/zhoorta/waydesk/blob/main/site/pages/">a file in the repository</a>: corrections are welcome.</p>\n', f'<div class="grid">\n{cards}\n</div>\n')
    return page


def main():
    pages = {}
    for path in sorted(PAGES.glob("*.md")):
        meta, intro, sections = parse(path)
        meta["slug"] = meta.get("slug", path.stem)
        pages[meta["slug"]] = (meta, intro, sections, path.name)
    for slug, (meta, intro, sections, name) in pages.items():
        out = SITE / slug / "index.html"
        out.parent.mkdir(exist_ok=True)
        out.write_text(page_html(meta, intro, sections, {k: (v[0],) for k, v in pages.items()}, name))
        print("wrote", out.relative_to(SITE))
    (SITE / "docs").mkdir(exist_ok=True)
    (SITE / "docs" / "index.html").write_text(index_html({k: (v[0],) for k, v in pages.items()}))
    urls = [f"{BASE}/"] + [f"{BASE}/docs/"] + [f"{BASE}/{s}/" for s in NAV if s in pages]
    (SITE / "sitemap.xml").write_text('<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">' + "".join(f"<url><loc>{u}</loc></url>" for u in urls) + "</urlset>\n")
    print("wrote docs/index.html and sitemap.xml:", len(urls), "urls")


if __name__ == "__main__":
    sys.exit(main())
