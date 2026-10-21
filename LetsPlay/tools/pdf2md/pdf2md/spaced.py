"""Letter-spaced small-caps headings (``B l e s s i n g  o f ...``).

Some books set feature headings in tracked small caps. The text layer keeps
them as one character per word, and the markdown converter drops them, so the
2014 Player's Handbook lost most subclass feature headings. We rebuild the
words from character gaps and put each heading back before the paragraph that
follows it on the page.
"""

import re
import statistics

from . import repair, rooms

MIN_CHARS = 5


def _join_fragments(text):
    """`D omain` -> `Domain`, `A ncients` -> `Ancients` (keeps `A Rule`)."""
    def sub(m):
        a, b = m.group(1), m.group(2)
        if (a + b).lower() in repair.WORDS or b.lower() not in repair.WORDS:
            return a + b
        return m.group(0)

    text = re.sub(r"\b([A-Z]) ([a-z]{2,})\b", sub, text)
    return re.sub(r"\s*:\s*", ": ", text).strip()


def _lines(page):
    out = []
    for b in page.get_text("rawdict")["blocks"]:
        for ln in b.get("lines", []):
            chars = [c for s in ln["spans"] for c in s["chars"]]
            out.append((ln["bbox"], chars, "".join(c["c"] for c in chars)))
    return out


def _is_spaced(t):
    t = t.rstrip()
    if len(t) < MIN_CHARS * 2 - 1:
        return False
    gaps = t[1::2]
    return gaps.count(" ") >= 0.8 * len(gaps)


def _words(chars):
    vis = [c for c in chars if c["c"].strip()]
    if len(vis) < MIN_CHARS:
        return ""
    gaps = [vis[i + 1]["bbox"][0] - vis[i]["bbox"][2] for i in range(len(vis) - 1)]
    med = statistics.median(gaps)
    w = vis[0]["c"]
    for i, g in enumerate(gaps):
        w += (" " if g > med + 1.2 else "") + vis[i + 1]["c"]
    return w


def spaced_headings(page):
    """[(heading, text of the next line in the same column)]."""
    lines = _lines(page)
    found = []
    for bbox, chars, t in lines:
        if not _is_spaced(t):
            continue
        h = _join_fragments(_words(chars))
        if not re.search(r"[A-Za-z]{3}", h) or len(h.split()) > 8:
            continue
        below = [(bb, tt) for bb, _, tt in lines
                 if bb[1] >= bbox[3] - 1 and abs(bb[0] - bbox[0]) < 15 and not _is_spaced(tt)
                 and len(tt.strip()) >= 12]
        follow = min(below, key=lambda x: x[0][1])[1] if below else ""
        found.append((h, follow))
    return found


def restore(body, page):
    """Insert letter-spaced headings missing from `body`. Returns (body, n)."""
    have = {rooms.norm(m.group(2)) for m in
            (re.match(r"^(#+)\s+(.*)$", ln) for ln in body.split("\n")) if m}
    n = 0
    for h, follow in spaced_headings(page):
        if rooms.norm(h) in have or not follow:
            continue
        body, ok = rooms.insert_heading(body, h, follow)
        if ok:
            have.add(rooms.norm(h))
            n += 1
    return body, n
