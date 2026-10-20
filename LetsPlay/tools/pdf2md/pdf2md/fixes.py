"""Page corrections that survive reconversion.

`.md/<stem>/fixes/pNNNN.md` replaces page N in the assembled book. The first line
is a header comment recording the PDF's sha256 and the pdf2md version; a fix made
for a different PDF (sha256 mismatch) is ignored. Fixes stay local (gitignored
with the rest of the converted book).
"""

import os
import re

from .audit import ANCHOR_RE, split_pages

HEADER_RE = re.compile(r"^<!-- pdf2md-fix (.*?) -->\n?")


def header(sha256, tool, engine):
    return f"<!-- pdf2md-fix sha256={sha256} tool={tool} engine={engine} -->\n"


def read_fix(path):
    with open(path, encoding="utf-8") as f:
        text = f.read()
    m = HEADER_RE.match(text)
    meta = dict(kv.split("=", 1) for kv in m.group(1).split()) if m else {}
    return meta, text[m.end():] if m else text


def write_fix(path, sha256, tool, engine, md):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(header(sha256, tool, engine) + md.rstrip() + "\n")
    os.replace(tmp, path)


def load_fixes(book_dir, sha256):
    d = os.path.join(book_dir, "fixes")
    out = {}
    if not os.path.isdir(d):
        return out
    for name in sorted(os.listdir(d)):
        m = re.fullmatch(r"p(\d{4})\.md", name)
        if not m:
            continue
        meta, md = read_fix(os.path.join(d, name))
        if meta.get("sha256") == sha256:
            out[int(m.group(1))] = md
    return out


def apply_fixes(md, fixes):
    """Replace fixed pages in assembled markdown (anchors are kept)."""
    if not fixes:
        return md
    first = ANCHOR_RE.search(md)
    if not first:
        return md
    head, pages = md[:first.start()], split_pages(md)
    parts = []
    for p, text in pages.items():
        if p in fixes:
            fix = fixes[p].strip()
            if not fix.startswith(f'<a id="p{p}"></a>'):
                fix = f'<a id="p{p}"></a>\n' + fix
            text = fix + "\n\n"
        parts.append(text)
    return head + "".join(parts)
