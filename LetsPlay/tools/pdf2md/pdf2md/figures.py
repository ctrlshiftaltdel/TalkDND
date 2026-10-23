"""Illustrations and maps cut out of scanned pages (ported from ConvertPDF
figures.py / export_art.py / insert_figures.py).

Scanned books have one page bitmap plus an invisible OCR text layer, so art is
not a separate image object: it is found in the page bitmap.

- Full-page art (maps, splash pages): ink cover >= 20% and long-line text cover
  <= 3% -> the whole page, longest side ~1600 px.
- Art inside a text page (monster art beside a stat block): ink grid minus text
  cells -> largest text-free rectangle per region, grown outwards until it meets
  text, then three gates (no text inside, aspect <= 3.6, no picture continuing
  outside the box). Crops that cannot be cut cleanly are dropped. Longest side
  ~1000 px.
- Placement: after the paragraph that ends just above the art (line anchor),
  else before the paragraph just below it, else at the end of the page.

All images are WebP (quality 80) under img/.
"""

import io
import re

import numpy as np
from PIL import Image, ImageFilter

from . import textutil

CELL = 4.0
TEXT_PAD = 2.0
DILATE_CELLS = 2
CROP_FILL_CELLS = 4
EDGE_BAND = 14.0
LONG_LINE = 0.70
MARK_DENSITY = 0.012
MIN_W, MIN_H = 80, 60
MIN_AREA = 8000
MAX_PER_PAGE = 3
MIN_REGION_INK = 0.10
FULL_MIN_INK = 0.20
FULL_MAX_TEXT = 0.03
ART_MIN_COVER = 0.06
ART_MAX_ASPECT = 3.6
ART_MAX_TEXT_OVERLAP = 0.02
MAP_SIDE = 1600
ART_SIDE = 1000
QUALITY = 80


def render_gray(page, dpi=72):
    import pymupdf

    pix = page.get_pixmap(dpi=dpi, colorspace=pymupdf.csGRAY, alpha=False)
    arr = np.frombuffer(pix.samples, dtype=np.uint8).reshape(pix.height, pix.width)
    return arr, pix.width / page.rect.width


def ink_mask(gray, dpi=72):
    """Adaptive ink: darker than the local (box-blurred) paper colour."""
    bg = np.asarray(Image.fromarray(gray).filter(ImageFilter.BoxBlur(max(6, int(12 * dpi / 72)))),
                    dtype=np.int16)
    g = gray.astype(np.int16)
    return (g < bg - 18) | (g <= 80)


def text_lines(page):
    out = []
    for blk in page.get_text("dict").get("blocks", []):
        if blk.get("type") != 0:
            continue
        for ln in blk.get("lines", []):
            t = textutil.line_text(ln).strip()
            if t:
                out.append((tuple(ln["bbox"]), t))
    return out


def grid_ink(ink, scale):
    cw = max(1, int(round(CELL * scale)))
    h, w = ink.shape
    nrow, ncol = h // cw, w // cw
    a = ink[:nrow * cw, :ncol * cw].astype(np.float32)
    return a.reshape(nrow, cw, ncol, cw).mean(axis=(1, 3))


def grid_text(lines, shape, pad=TEXT_PAD):
    nrow, ncol = shape
    tg = np.zeros((nrow, ncol), dtype=bool)
    for (x0, y0, x1, y1), _ in lines:
        x0, y0, x1, y1 = x0 - pad, y0 - pad, x1 + pad, y1 + pad
        tg[max(0, int(y0 / CELL)):min(nrow, int(np.ceil(y1 / CELL)) + 1),
           max(0, int(x0 / CELL)):min(ncol, int(np.ceil(x1 / CELL)) + 1)] = True
    return tg


def erase_thin_lines(mask, ratio=LONG_LINE, max_run=3):
    """Erase page frames / rules: runs of 1..max_run rows (cols) that are almost all ink."""
    for axis in (0, 1):
        hot = mask.mean(axis=1 - axis) > ratio
        i = 0
        while i < len(hot):
            if not hot[i]:
                i += 1
                continue
            j = i
            while j < len(hot) and hot[j]:
                j += 1
            if j - i <= max_run:
                if axis == 0:
                    mask[i:j, :] = False
                else:
                    mask[:, i:j] = False
            i = j
    return mask


def grid_dilate(mask, r):
    out = mask.copy()
    for _ in range(r):
        m = out.copy()
        out[1:, :] |= m[:-1, :]
        out[:-1, :] |= m[1:, :]
        m = out.copy()
        out[:, 1:] |= m[:, :-1]
        out[:, :-1] |= m[:, 1:]
    return out


def label_boxes(mask):
    nrow, ncol = mask.shape
    seen = np.zeros_like(mask, dtype=bool)
    boxes = []
    ys, xs = np.nonzero(mask)
    for y0, x0 in zip(ys.tolist(), xs.tolist()):
        if seen[y0, x0]:
            continue
        stack = [(y0, x0)]
        seen[y0, x0] = True
        r0 = r1 = y0
        c0 = c1 = x0
        while stack:
            y, x = stack.pop()
            r0, r1, c0, c1 = min(r0, y), max(r1, y), min(c0, x), max(c1, x)
            for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                ny, nx = y + dy, x + dx
                if 0 <= ny < nrow and 0 <= nx < ncol and mask[ny, nx] and not seen[ny, nx]:
                    seen[ny, nx] = True
                    stack.append((ny, nx))
        boxes.append((r0, r1 + 1, c0, c1 + 1))
    return boxes


def max_rect(mask):
    """Largest all-true sub-rectangle (histogram method): (area, r0, r1, c0, c1)."""
    nrow, ncol = mask.shape
    best = (0, 0, 0, 0, 0)
    heights = np.zeros(ncol, dtype=np.int32)
    for i in range(nrow):
        heights = np.where(mask[i], heights + 1, 0)
        stack = []
        for j in range(ncol + 1):
            cur = int(heights[j]) if j < ncol else 0
            start = j
            while stack and stack[-1][1] > cur:
                start, h = stack.pop()
                if h * (j - start) > best[0]:
                    best = (h * (j - start), i - h + 1, i + 1, start, j)
            stack.append((start, cur))
    return best


def _band(dens, tg, r):
    x0, y0, x1, y1 = r
    c0, c1 = int(x0 / CELL), int(np.ceil(x1 / CELL))
    r0, r1 = int(y0 / CELL), int(np.ceil(y1 / CELL))
    d = dens[max(0, r0):max(0, r1), max(0, c0):max(0, c1)]
    t = tg[max(0, r0):max(0, r1), max(0, c0):max(0, c1)]
    return (float(d.mean()) if d.size else 0.0, float(t.mean()) if t.size else 0.0)


def grow_rect(rect, dens, tg, pw, ph, max_pt=64):
    """Grow each side until text, the page edge, or 4 blank cells."""
    x0, y0, x1, y1 = rect
    for side in ("top", "bottom", "left", "right"):
        moved, blank = 0.0, 0
        while moved + CELL <= max_pt:
            if side == "top":
                if y0 - CELL < 0:
                    break
                nb = (x0, y0 - CELL, x1, y0)
            elif side == "bottom":
                if y1 + CELL > ph:
                    break
                nb = (x0, y1, x1, y1 + CELL)
            elif side == "left":
                if x0 - CELL < 0:
                    break
                nb = (x0 - CELL, y0, x0, y1)
            else:
                if x1 + CELL > pw:
                    break
                nb = (x1, y0, x1 + CELL, y1)
            d, t = _band(dens, tg, nb)
            if t > 0.15:
                break
            blank = blank + 1 if d < 0.02 else 0
            if blank > 3:
                break
            if side == "top":
                y0 -= CELL
            elif side == "bottom":
                y1 += CELL
            elif side == "left":
                x0 -= CELL
            else:
                x1 += CELL
            moved += CELL
    return (x0, y0, x1, y1)


def cut_sides(rect, dens, tg, pw, ph, th=0.22, tol=14):
    """Sides where the picture continues outside the box (the page edge does not count)."""
    x0, y0, x1, y1 = rect
    n = 0
    for at_edge, band in (
            (y0 <= tol, (x0, y0 - 2 * CELL, x1, y0 - CELL)),
            (y1 >= ph - tol, (x0, y1 + CELL, x1, y1 + 2 * CELL)),
            (x0 <= tol, (x0 - 2 * CELL, y0, x0 - CELL, y1)),
            (x1 >= pw - tol, (x1 + CELL, y0, x1 + 2 * CELL, y1))):
        if at_edge:
            continue
        d, t = _band(dens, tg, band)
        if d > th and t < 0.2:
            n += 1
    return n


def _overlap(rect, lines):
    x0, y0, x1, y1 = rect
    area = max(1.0, (x1 - x0) * (y1 - y0))
    hit = 0.0
    for (a, b, c, d), _ in lines:
        w, h = min(x1, c) - max(x0, a), min(y1, d) - max(y0, b)
        if w > 0 and h > 0:
            hit += w * h
    return min(1.0, hit / area)


def analyse(page):
    gray, scale = render_gray(page)
    ink = ink_mask(gray)
    lines = text_lines(page)
    dens = grid_ink(ink, scale)
    tg = grid_text(lines, dens.shape)
    return ink, lines, dens, tg


def full_page_art(page, ink=None, lines=None):
    """Mostly-picture page: ink cover >= 20% and long-line (>= 40 chars) text <= 3%."""
    if ink is None:
        ink, lines, _, _ = analyse(page)
    area = page.rect.width * page.rect.height
    long_area = sum(max(0, c - a) * max(0, d - b) for (a, b, c, d), t in lines if len(t) >= 40)
    return float(ink.mean()) >= FULL_MIN_INK and long_area / area <= FULL_MAX_TEXT


def find_art(page, analysed=None):
    """Clean art rectangles (points, top-left origin) inside a text page."""
    ink, lines, dens, tg = analysed or analyse(page)
    pw, ph = page.rect.width, page.rect.height
    mark = dens > MARK_DENSITY
    band = int(EDGE_BAND / CELL)
    mark[:band, :] = mark[-band:, :] = False
    mark[:, :band] = mark[:, -band:] = False
    erase_thin_lines(mark)
    mark_art = mark & ~tg
    cands = label_boxes(grid_dilate(mark_art, DILATE_CELLS))
    cands.sort(key=lambda b: -((b[1] - b[0]) * (b[3] - b[2])))
    out = []
    used = np.zeros_like(mark, dtype=bool)
    for r0, r1, c0, c1 in cands:
        if len(out) >= MAX_PER_PAGE:
            break
        w, h = (c1 - c0) * CELL, (r1 - r0) * CELL
        if w < MIN_W or h < MIN_H or w * h < MIN_AREA or used[r0:r1, c0:c1].mean() > 0.4:
            continue
        sub = mark_art[r0:r1, c0:c1]
        if float(sub.mean()) < MIN_REGION_INK or float(tg[r0:r1, c0:c1].mean()) > 0.5:
            continue
        area, a0, a1, b0, b1 = max_rect(grid_dilate(sub, CROP_FILL_CELLS) & ~tg[r0:r1, c0:c1])
        if area * CELL * CELL < MIN_AREA:
            continue
        s0, s1, t0, t1 = r0 + a0, r0 + a1, c0 + b0, c0 + b1
        if float(tg[s0:s1, t0:t1].mean()) > 0.03:
            continue
        rect = (t0 * CELL, s0 * CELL, min(pw, t1 * CELL), min(ph, s1 * CELL))
        rect = grow_rect(rect, dens, tg, pw, ph)
        rect = (max(0, rect[0]), max(0, rect[1]), min(pw, rect[2]), min(ph, rect[3]))
        rw, rh = rect[2] - rect[0], rect[3] - rect[1]
        if rw < 60 or rh < 60 or rw * rh / (pw * ph) < ART_MIN_COVER:
            continue
        if max(rw, rh) / max(1.0, min(rw, rh)) > ART_MAX_ASPECT:
            continue
        if _overlap(rect, lines) > ART_MAX_TEXT_OVERLAP or cut_sides(rect, dens, tg, pw, ph):
            continue  # text inside, or the picture goes on: drop rather than show a cut image
        if _band(dens, tg, rect)[0] < 0.05:
            continue
        out.append(rect)
        used[s0:s1, t0:t1] = True
    if out:
        big = max((r[2] - r[0]) * (r[3] - r[1]) for r in out)
        out = [r for r in out if (r[2] - r[0]) * (r[3] - r[1]) >= big * 0.35]
    out.sort(key=lambda r: (r[1], r[0]))
    return out


def anchors(lines, rect):
    """(nearest text line above, nearest text line below) overlapping the art horizontally."""
    x0, y0, x1, y1 = rect
    above = below = None
    for (a, b, c, d), t in lines:
        if len(t) < 15 or c < x0 - 6 or a > x1 + 6:
            continue
        if d <= y0 + 3 and (above is None or d > above[0]):
            above = (d, t)
        if b >= y1 - 3 and (below is None or b < below[0]):
            below = (b, t)
    return (above[1] if above else ""), (below[1] if below else "")


def render_webp(page, rect, side):
    """WebP bytes of a page region (None = whole page), longest side ~`side` px."""
    import pymupdf

    w = (rect[2] - rect[0]) if rect else page.rect.width
    h = (rect[3] - rect[1]) if rect else page.rect.height
    dpi = max(36, min(300, int(side * 72 / max(w, h))))
    kw = {"clip": pymupdf.Rect(*rect)} if rect else {}
    pix = page.get_pixmap(dpi=dpi, **kw)
    img = Image.frombytes("RGB" if pix.n < 4 else "RGBA", (pix.width, pix.height), pix.samples)
    buf = io.BytesIO()
    img.save(buf, "WEBP", quality=QUALITY, method=4)
    return buf.getvalue()


def _norm(s):
    s = s.lower().replace("\u2019", "'")
    return re.sub(r"\s+", " ", re.sub(r"[^a-z0-9 ]", " ", s)).strip()


def _locate(lines, text):
    key = _norm(text or "")
    if len(key) < 12:
        return None
    words = key.split()
    shingles = [" ".join(words[j:j + 3]) for j in range(max(1, len(words) - 2))]
    for i, ln in enumerate(lines):
        n = _norm(ln)
        if n and (key in n or n in key and len(n) >= 20):
            return i
    for sh in shingles[:6]:
        if len(sh.split()) < 3:
            continue
        for i, ln in enumerate(lines):
            if sh in _norm(ln):
                return i
    return None


def place(body, img_md, above, below):
    """Insert `img_md` after the paragraph containing `above`, else before the one
    containing `below`, else at the end. Returns (body, where)."""
    lines = body.split("\n")
    i = _locate(lines, above)
    if i is not None:
        while i + 1 < len(lines) and lines[i + 1].strip():
            i += 1
        lines[i + 1:i + 1] = ["", img_md]
        return "\n".join(lines), "after"
    i = _locate(lines, below)
    if i is not None:
        while i > 0 and lines[i - 1].strip():
            i -= 1
        lines[i:i] = [img_md, ""]
        return "\n".join(lines), "before"
    return (body.rstrip() + "\n\n" + img_md).strip(), "end"


def contact_sheet(paths, labels, cols=4, rows=4, thumb=280):
    """Grid images (WebP bytes) of `cols x rows` thumbnails for human review."""
    from PIL import ImageDraw

    sheets = []
    per = cols * rows
    for s in range(0, len(paths), per):
        canvas = Image.new("RGB", (cols * thumb, rows * (thumb + 18)), "white")
        draw = ImageDraw.Draw(canvas)
        for k, (p, lab) in enumerate(zip(paths[s:s + per], labels[s:s + per])):
            try:
                with Image.open(p) as im:
                    im = im.convert("RGB")
                    im.thumbnail((thumb - 8, thumb - 8))
                    x, y = (k % cols) * thumb, (k // cols) * (thumb + 18)
                    canvas.paste(im, (x + 4, y + 4))
                    draw.text((x + 4, y + thumb), lab, fill="black")
            except OSError:
                continue
        buf = io.BytesIO()
        canvas.save(buf, "WEBP", quality=75)
        sheets.append(buf.getvalue())
    return sheets
