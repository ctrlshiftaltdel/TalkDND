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


IMG_RE = re.compile(r"!\[[^\]]*\]\(img/[^)]+\)")


def refresh_fix(fix, generated):
    """Carry newer converter output into a stored fix without losing the fix:
    image links and room-key headings the fix does not have yet."""
    from . import headings, rooms

    fix = "\n".join(headings.promote_room_lines(fix.split("\n")))
    gen_lines = generated.split("\n")
    pairs = []
    for i, ln in enumerate(gen_lines):
        m = headings.HEADING_RE.match(ln)
        if m and re.match(r"[A-Z]{1,2}\d{1,3}[a-z]?:", m.group(2)):
            follow = next((x for x in gen_lines[i + 1:] if x.strip() and not x.startswith("#")), "")
            pairs.append((m.group(2), follow))
    fix, _ = rooms.add_missing_rooms(fix, pairs)
    missing = [im for im in IMG_RE.findall(generated) if im.split("](")[1] not in fix]
    if missing:
        fix = fix.rstrip() + "\n\n" + "\n\n".join(missing)
    return fix


def apply_fixes(md, fixes, refresh=False):
    """Replace fixed pages in assembled markdown (anchors are kept). With
    `refresh`, image links and room headings from the new conversion are
    merged into the fix (the stored fix file itself is not changed)."""
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
            if refresh:
                fix = refresh_fix(fix, text)
            if not fix.startswith(f'<a id="p{p}"></a>'):
                fix = f'<a id="p{p}"></a>\n' + fix
            text = fix + "\n\n"
        parts.append(text)
    return head + "".join(parts)
