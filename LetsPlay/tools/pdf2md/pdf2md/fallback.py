"""Safety nets for pages where the layout extractor goes wrong.

* column_text(): plain text-layer extraction in reading order (full-width
  blocks split the page into bands; inside a band the left column is read
  before the right one).
* has_stutter(): the same phrase repeated many times on one line.
* order_score(): how well the extracted word order follows the column order;
  low values mean two columns were interleaved line by line.
* heading repair: decorative fonts with broken glyph maps produce garbled
  heading text; re-read those headings with OCR.
"""

import collections
import re

from . import repair, textutil

_W = re.compile(r"[a-z]+(?:'[a-z]+)?")


def words(text):
    return _W.findall(text.lower())


def body_size(sizes):
    """Font size carrying the most characters on the page (the body text)."""
    c = collections.Counter()
    for size, n in sizes:
        c[round(size * 2) / 2] += n
    return c.most_common(1)[0][0] if c else 0


def _lines(page):
    raw = []
    for blk in page.get_text("dict", sort=True)["blocks"]:
        if blk.get("type") != 0:
            continue
        for ln in blk["lines"]:
            t = textutil.line_text(ln).strip()
            if t:
                size = max(sp["size"] for sp in ln["spans"])
                bold = "bold" in ln["spans"][0]["font"].lower()
                raw.append((ln["bbox"], t, size, bold))
    # headings are judged against this page's body size, not absolute points:
    # page sizes (and scaled scans) differ from book to book
    body = body_size((s, len(t)) for _, t, s, _ in raw)
    return [(r, t, is_heading(t, s, b, body)) for r, t, s, b in raw]


def is_heading(text, size, bold, body):
    t = text.strip()
    letters = [c for c in t if c.isalpha()]
    # too short to be a word, or a body line that merely starts mid-sentence
    if not body or len(letters) < 3 or letters[0].islower() or len(t) > 80:
        return False
    if size >= body * 1.25:
        return True
    return bold and size >= body * 0.95 and len(text) <= 60 and not text.rstrip().endswith(("-", ","))


def _texts(page):
    return [(r, t) for r, t, _ in _lines(page)]


def _junk(t):
    """Stray glyphs from artwork: mostly 1-2 character tokens, few letters."""
    toks = t.split()
    if not toks:
        return True
    short = sum(1 for x in toks if len(re.sub(r"[^A-Za-z]", "", x)) <= 2)
    letters = sum(c.isalpha() for c in t)
    return len(toks) >= 4 and (short / len(toks) > 0.6 or letters < 0.5 * len(t.replace(" ", "")))


def column_text(page):
    """Text layer in reading order, joined into paragraphs."""
    width = page.rect.width or 1
    mid = page.rect.x0 + width / 2
    pad = width * 0.03
    lines = _lines(page)
    full = sorted(r[1] for r, _, _ in lines if r[0] < mid - pad and r[2] > mid + pad)

    def key(item):
        r = item[0]
        band = sum(1 for y in full if y <= r[1] + 0.5)
        col = -1 if (r[0] < mid - pad and r[2] > mid + pad) else (0 if r[2] <= mid + pad else 1)
        return (band, col, r[1], r[0])

    paras, cur, prev = [], [], None
    lines = [x for x in lines if not _junk(x[1])]
    for r, t, head in sorted(lines, key=key):
        k = key((r, t))[:2]
        h = r[3] - r[1]
        if head and len(t) <= 80:
            if cur:
                paras.append(" ".join(cur))
            paras.append("### " + t)
            cur, prev = [], None
            continue
        if prev and (k != prev[0] or r[1] - prev[1][3] > h * 0.8 or r[1] < prev[1][1]):
            paras.append(" ".join(cur))
            cur = []
        cur.append(t)
        prev = (k, r)
    if cur:
        paras.append(" ".join(cur))
    text = "\n\n".join(paras).replace("\u00ad", "")
    return re.sub(r"(?<=[A-Za-z])- (?=[a-z])", "", text)


def has_stutter(md, n=4, times=5):
    """True when one line repeats the same n-word phrase at least `times` times."""
    for line in md.split("\n"):
        w = line.lower().split()
        if len(w) < n * times:
            continue
        c = collections.Counter(tuple(w[i:i + n]) for i in range(len(w) - n + 1))
        if c.most_common(1)[0][1] >= times:
            return True
    return False


def order_score(md, ref):
    """Share of the reference's word bigrams that also appear in `md`."""
    a, b = words(md), words(ref)
    if len(b) < 40:
        return 1.0
    big = set(zip(a, a[1:]))
    ref_big = list(zip(b, b[1:]))
    return sum(1 for x in ref_big if x in big) / len(ref_big)


def interleaved_lines(md, ref, jump=3, min_jumps=4):
    """Lines whose word trigrams hop back and forth between distant parts of the
    reference text (two columns merged line by line)."""
    rw = words(ref)
    pos = {}
    for i in range(len(rw) - 2):
        pos.setdefault(tuple(rw[i:i + 3]), []).append(i)
    bad = 0
    for line in md.split("\n"):
        w = words(line)
        if len(w) < 12:
            continue
        seq = [pos[t][0] for t in (tuple(w[i:i + 3]) for i in range(len(w) - 2)) if len(pos.get(t, ())) == 1]
        big = [y - x for x, y in zip(seq, seq[1:]) if abs(y - x) > jump]
        flips = sum(1 for u, v in zip(big, big[1:]) if (u > 0) != (v > 0))
        if flips >= min_jumps:
            bad += 1
    return bad


def recall(md, ref):
    a, b = collections.Counter(words(md)), collections.Counter(words(ref))
    tot = sum(b.values())
    return sum((a & b).values()) / tot if tot else 1.0


def dict_ratio(text):
    w = [x for x in re.findall(r"[A-Za-z]{2,}", text)]
    if not w or not repair.WORDS:
        return 1.0
    return sum(1 for x in w if x.lower() in repair.WORDS) / len(w)


def repair_headings(md, page, ocr_engine, dpi=300, limit=12):
    """Re-read garbled headings with OCR. Returns (md, number of headings fixed)."""
    from . import ocr

    if not repair.WORDS:
        return md, 0
    fixed, out = 0, []
    for line in md.split("\n"):
        m = re.match(r"^(#{1,6})\s+(.*\S)\s*$", line)
        if m and fixed < limit:
            text = re.sub(r"<[^>]+>|\*", "", m.group(2)).strip()
            if len(re.findall(r"[A-Za-z]{3,}", text)) and dict_ratio(text) < 0.5:
                new = _ocr_heading(page, text, ocr_engine, dpi)
                ok = new and len(new) <= 2 * len(text) + 10
                if ok and dict_ratio(new) > dict_ratio(text) and dict_ratio(new) >= 0.5:
                    line = f"{m.group(1)} {new}"
                    fixed += 1
        out.append(line)
    return "\n".join(out), fixed


def _ocr_heading(page, text, ocr_engine, dpi):
    import pymupdf

    from . import ocr

    key = re.sub(r"[^a-z]", "", text.lower())
    if len(key) < 3:
        return None
    lines = _texts(page)
    r = None
    for bbox, t in lines:
        k = re.sub(r"[^a-z]", "", t.lower())
        if k and (k in key or key in k) and len(key) <= len(k) * 2 and len(k) <= len(key) * 2:
            r = pymupdf.Rect(bbox)
            break
    if r is None:
        hits = page.search_for(text)
        if not hits:
            return None
        r = hits[0]
    # same baseline, close by: other parts of the heading (drop caps, split spans)
    base, h = pymupdf.Rect(r), r.height
    for bbox, _ in lines:
        o = pymupdf.Rect(bbox)
        near = (o.x0 - base.x1) < h * 2 and (base.x0 - o.x1) < h * 2
        if near and o.height < h * 1.5 and o.width < base.width * 2 and abs(o.y1 - base.y1) < h * 0.6:
            r |= o
    pad = r.height * 0.6
    clip = pymupdf.Rect(r.x0 - pad, r.y0 - pad / 2, r.x1 + pad, r.y1 + pad / 2) & page.rect
    try:
        pix = page.get_pixmap(dpi=dpi, clip=clip)
        lines = ocr.get_engine(ocr_engine).read(pix.tobytes("png"))
    except Exception:  # OCR missing or failed: keep the original heading
        return None
    z = dpi / 72
    mid_y = (base.y0 + base.y1) / 2
    # keep the OCR line(s) on the heading's own row, left to right
    rows = [ln for ln in lines if ln.text.strip()]
    if rows:
        best = min(rows, key=lambda ln: abs(clip.y0 + (ln.y0 + ln.y1) / 2 / z - mid_y))
        cy = (best.y0 + best.y1) / 2
        rows = sorted((ln for ln in rows if abs((ln.y0 + ln.y1) / 2 - cy) < (best.y1 - best.y0) * 0.5),
                      key=lambda ln: ln.x0)
    got = " ".join(ln.text.strip() for ln in rows)
    return got.strip() or None


_HEAD = re.compile(r"^(#{1,6})\s+(.*?)\s*$")


def body_like_heading(text):
    """Heading text that is really a line of body text (or OCR junk)."""
    t = re.sub(r"<[^>]+>|\*", "", text).strip().lstrip("\u2018\u2019'\"")
    letters = [c for c in t if c.isalpha()]
    if len(letters) < 3 or letters[0].islower():
        return True
    if len(t) > 60:
        return True
    return len(t.split()) >= 5 and t.endswith((",", "-", ";"))


def demote_body_headings(md):
    """Turn sentence-like headings (OCR / text-layer fallback pages) back into text."""
    out = []
    for line in md.split("\n"):
        m = _HEAD.match(line)
        if m and body_like_heading(m.group(2)):
            line = m.group(2)
        out.append(line)
    return "\n".join(out)
