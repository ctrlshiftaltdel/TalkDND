"""Adventure layout helpers: room-key headings the text layer missed, and
read-aloud boxes (tinted panels) marked as `>` quotes.

Both work from the page itself: room keys are re-read with OCR when the PDF text
layer dropped them, and read-aloud boxes are found by their tinted background in
a low-resolution render, so scanned books with an OCR text layer work too.
"""

import collections
import re

from . import headings

ROOM_OCR_RE = re.compile(r"^\W*([A-Z])([A-Z]?[0-9lIOS|]{1,3}[a-z]?)\s*[:.;]\s*(.{2,60}?)\s*$")
AREA_REF_RE = re.compile(r"\b(areas?|rooms?)\s+((?:[A-Z]{1,2}[0-9lIO|]{1,3}[a-z]?(?:,\s*|\s+(?:and|or|to|through)\s+)?)+)")
CODE_TOKEN_RE = re.compile(r"\b[A-Z]{1,2}[0-9lIO|]{1,3}[a-z]?\b")
BOX_DARKER = 12        # tint: summed RGB/8 below the page's paper colour by this much
BOX_MIN_LINES = 2


def norm(s):
    return re.sub(r"[^a-z]", "", s.lower())


def _code(letter, num):
    fixed = headings.fix_room_code(f"{letter}{num}: X")
    code = fixed.split(":")[0]
    return code if re.fullmatch(r"[A-Z]{1,2}\d{1,3}[a-z]?", code) else None


def ocr_rooms(lines, page_width):
    """[(heading text, text of the line that follows it in the same column)]."""
    out = []
    for ln in lines:
        m = ROOM_OCR_RE.match(ln.text)
        if not m:
            continue
        code = _code(m.group(1), m.group(2))
        title = m.group(3).strip()
        if not code or not re.search(r"[A-Z]{3}", title) or title != title.upper():
            continue
        below = [x for x in lines if x.y0 > ln.y1 - 2 and abs(x.x0 - ln.x0) < page_width * 0.08
                 and not ROOM_OCR_RE.match(x.text)]
        if not below:
            continue
        nxt = min(below, key=lambda x: x.y0)
        out.append((f"{code}: {title}", nxt.text))
    return out


def present_codes(body):
    codes = set()
    for ln in body.split("\n"):
        m = headings.HEADING_RE.match(ln)
        if m:
            c = re.match(r"([A-Z]{1,2}\d{1,3}[a-z]?)[:.]", headings.fix_room_code(m.group(2)))
            if c:
                codes.add(c.group(1))
    return codes


def insert_heading(body, heading, follow, level=4):
    """Put `heading` before the paragraph that starts with `follow` (fuzzy)."""
    key = norm(follow)[:20]
    if len(key) < 10:
        return body, False
    lines = body.split("\n")
    for i, ln in enumerate(lines):
        if headings.HEADING_RE.match(ln) or ln.startswith("|"):
            continue
        if key in norm(ln)[:80]:
            lines[i:i] = ["#" * level + " " + heading, ""]
            return "\n".join(lines), True
    return body, False


def split_inline_heading(body, code, title):
    """`> V7: Boxes BREAK of supplies ROOM AND STORAGE l corners` (a heading the
    text layer wove into the first paragraph) -> heading + clean paragraph."""
    tnorm = norm(title)
    lines = body.split("\n")
    pre = re.compile(r"^(>\s*)?" + re.escape(code) + r"\s*[:.]\s*")
    for i, ln in enumerate(lines):
        m = pre.match(ln.strip())
        if not m:
            continue
        words = ln.strip()[m.end():].split(" ")
        keep = [w for w in words if not (len(norm(w)) >= 2 and w.upper() == w and norm(w) in tnorm)]
        if len(keep) == len(words):
            continue
        lines[i:i + 1] = ["#### " + code + ": " + title, "", (m.group(1) or "") + " ".join(keep)]
        return "\n".join(lines), True
    return body, False


def add_missing_rooms(body, rooms):
    have = present_codes(body)
    n = 0
    for heading, follow in rooms:
        code = heading.split(":")[0]
        if code in have:
            continue
        body, ok = split_inline_heading(body, code, heading.split(":", 1)[1].strip())
        if not ok:
            body, ok = insert_heading(body, heading, follow)
        if ok:
            have.add(code)
            n += 1
    return body, n


def boxed_text(page, dpi=72):
    """Normalised text of lines sitting on a tinted read-aloud panel."""
    try:
        from PIL import Image
    except ImportError:  # pragma: no cover
        return ""
    pix = page.get_pixmap(dpi=dpi)
    img = Image.frombytes("RGB", (pix.width, pix.height), pix.samples)
    sx, sy = pix.width / page.rect.width, pix.height / page.rect.height
    rows = []
    for b in page.get_text("dict")["blocks"]:
        for ln in b.get("lines", []):
            t = "".join(s["text"] for s in ln["spans"]).strip()
            if len(t) < 12:
                continue
            x0, y0, x1, y1 = ln["bbox"]
            px = [img.getpixel((x, y))
                  for x in range(max(0, int(x0 * sx)), min(pix.width, int(x1 * sx)), 2)
                  for y in range(max(0, int(y0 * sy)), min(pix.height, int(y1 * sy)), 2)]
            px = [q for q in px if sum(q) > 450]          # background, not glyphs
            if len(px) < 8:
                continue
            c = collections.Counter((r // 8, g // 8, bl // 8) for r, g, bl in px).most_common(1)[0][0]
            rows.append((sum(c), t))
    if len(rows) < 6:
        return ""
    paper = sorted(s for s, _ in rows)[len(rows) // 2]
    boxed = [t for s, t in rows if paper - s >= BOX_DARKER]
    if len(boxed) < BOX_MIN_LINES:
        return ""
    return " ".join(norm(t) for t in boxed)


def mark_readaloud(body, boxed):
    """Prefix paragraphs whose text lies in a read-aloud panel with `> `."""
    if not boxed:
        return body, 0
    out, n = [], 0
    for ln in body.split("\n"):
        s = ln.strip()
        if (s and not s.startswith((">", "#", "|", "!", "<", "-", "*")) and len(norm(s)) >= 40):
            t = norm(s)
            hits = sum(1 for k in range(0, min(len(t) - 20, 200), 40) if t[k:k + 20] in boxed)
            tries = len(range(0, min(len(t) - 20, 200), 40))
            if tries and hits / tries >= 0.6:
                ln = "> " + s
                n += 1
        out.append(ln)
    return "\n".join(out), n


def fix_area_refs(md):
    """`area Vl` -> `area V1`, `areas VlO and V9` -> `areas V10 and V9`, only for
    codes that exist as room headings in the book."""
    codes = present_codes(md)
    if not codes:
        return md

    def tok(m):
        t = m.group(0)
        if t in codes:
            return t
        c = _code(t[0], t[1:])
        return c if c in codes else t

    return AREA_REF_RE.sub(lambda m: m.group(1) + " " + CODE_TOKEN_RE.sub(tok, m.group(2)), md)
