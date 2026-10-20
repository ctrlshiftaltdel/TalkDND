"""Per-page quality audit of a converted book against the PDF text layer.

Scores (per page):
* recall      share of the PDF text-layer words found in the page markdown
              (None when the page has no usable text layer, e.g. scans)
* extra_1k    markdown words not in the text layer, per 1000 markdown words
* unknown_1k  lowercase words (3+ letters) missing from the word list, per 1000
* sus_heads   headings that look like body text
* inferred / check_pdf  agent review markers (`<!-- inferred: ... -->`, `<!-- check PDF -->`)
* table_issues, stutter, interleaved, empty, low (manifest quality flag)
`badness` combines them; audit.json lists pages worst first.
"""

import collections
import json
import os
import re

from . import fallback, repair, tables

REF_MAX_UNKNOWN = 60  # text layers worse than this (unknown words per 1k) are not used for recall
ANCHOR_RE = re.compile(r'<a id="p(\d+)"></a>')
_W = re.compile(r"[a-z]+(?:'[a-z]+)?")
_STRIP = re.compile(r"!\[[^\]]*\]\([^)]*\)|<a id=[^>]*></a>|<!--.*?-->|<[^>]+>", re.S)


def words(text):
    return _W.findall(text.lower())


def split_pages(md):
    """{page number: text from its anchor up to the next anchor} (anchor included)."""
    out, ms = {}, list(ANCHOR_RE.finditer(md))
    for i, m in enumerate(ms):
        end = ms[i + 1].start() if i + 1 < len(ms) else len(md)
        out[int(m.group(1))] = md[m.start():end]
    return out


def unknown_words(text):
    if not repair.WORDS:
        return 0, 0
    t = _STRIP.sub(" ", text)
    lw = [w.lower() for w in re.findall(r"[A-Za-z]{3,}", t) if w.islower() or not w[0].isupper()]
    return sum(1 for w in lw if w not in repair.WORDS), len(lw)


def normalize_ref(text):
    """Join words hyphenated across line ends in the PDF text layer ("materi-\\nalizing")."""
    text = text.replace("\u00ad\n", "").replace("\u00ad", "")
    return re.sub(r"(?<=[A-Za-z])[-\u2010]\s*\n\s*(?=[a-z])", "", text)


def score_page(pdf_text, md, info=None):
    info = info or {}
    pdf_text = normalize_ref(pdf_text)
    body = _STRIP.sub(" ", md)
    pw, mw = words(pdf_text), words(body)
    s = {}
    # a garbled text layer (bad OCR layer in the PDF) is no reference for recall
    rb, rn = unknown_words(pdf_text)
    s["ref_unknown_1k"] = round(rb / rn * 1000, 1) if rn else 0.0
    usable = len(pw) >= 30 and s["ref_unknown_1k"] <= REF_MAX_UNKNOWN
    if usable:
        P, M = collections.Counter(pw), collections.Counter(mw)
        s["recall"] = round(sum((P & M).values()) / len(pw), 4)
        s["extra_1k"] = round(sum((M - P).values()) / max(1, len(mw)) * 1000, 1)
    else:
        s["recall"], s["extra_1k"] = None, None
    bad, n = unknown_words(md)
    s["unknown_1k"] = round(bad / n * 1000, 1) if n else 0.0
    s["words"] = len(mw)
    s["sus_heads"] = sum(1 for ln in md.split("\n")
                         if re.match(r"^#{1,6}\s", ln) and fallback.body_like_heading(re.sub(r"^#+\s*", "", ln)))
    s["inferred"] = len(re.findall(r"<!--\s*inferred:", md))
    s["check_pdf"] = len(re.findall(r"<!--\s*check PDF", md))
    s["table_issues"] = len(tables.check_tables(md))
    s["stutter"] = fallback.has_stutter(md)
    s["interleaved"] = fallback.interleaved_lines(body, pdf_text) if usable else 0
    s["empty"] = len(mw) < 5 and len(pw) >= 30
    s["low"] = info.get("quality") == "low"
    s["flags"] = info.get("flags", [])
    s["badness"] = badness(s)
    return s


def badness(s):
    b = 0.0
    if s["recall"] is not None:
        b += (1 - s["recall"]) * 100 + min(s["extra_1k"], 300) / 10
    b += min(s["unknown_1k"], 300) / 3
    b += 5 * s["sus_heads"] + 5 * s["table_issues"] + 10 * s["interleaved"]
    b += 40 * bool(s["stutter"]) + 60 * bool(s["empty"]) + 15 * bool(s["low"])
    return round(b, 1)


def needs_review(s, threshold=25):
    # stat blocks are never rebuilt by the converter: always worth a review
    return s["badness"] >= threshold or "statblock" in s["flags"]


def audit_book(book, threshold=25):
    """book: convert.Book. Writes audit.json and returns the list of page rows."""
    md_path = os.path.join(book.dir, book.stem + ".md")
    with open(md_path, encoding="utf-8") as f:
        pages = split_pages(f.read())
    rows = []
    for p, md in pages.items():
        if p > book.doc.page_count:
            continue
        info = book.m["pages"].get(str(p), {})
        s = score_page(book.doc[p - 1].get_text(), md, info)
        s["page"] = p
        s["fixed"] = os.path.exists(fix_path(book, p))
        s["review"] = needs_review(s, threshold)
        rows.append(s)
    rows.sort(key=lambda r: -r["badness"])
    summary = {"pages": len(rows), "need_review": sum(r["review"] for r in rows),
               "fixed": sum(r["fixed"] for r in rows), "threshold": threshold}
    with open(os.path.join(book.dir, "audit.json"), "w", encoding="utf-8") as f:
        json.dump({"pdf": book.m["pdf"], "tool": book.m["tool"], "summary": summary, "pages": rows}, f, indent=1)
    return rows


class BookView:
    """Read-only view of a converted book (never writes the manifest, so it is
    safe while the background worker runs)."""

    def __init__(self, pdf):
        import pymupdf

        from .convert import out_dir

        self.pdf = os.path.abspath(pdf)
        self.dir, self.stem = out_dir(self.pdf)
        with open(os.path.join(self.dir, "manifest.json"), encoding="utf-8") as f:
            self.m = json.load(f)
        self.doc = pymupdf.open(self.pdf)


def fix_path(book, p):
    return os.path.join(book.dir, "fixes", "p%04d.md" % p)
