"""Intake audit for one PDF: is its text layer good enough for play?

Per book we record the text-layer character count per page, 0-character
pages, the U+FFFD / control-character rate, a scanned-vs-digital guess and the
PDF bookmarks (PyMuPDF TOC). The verdict is ``text layer usable`` or
``OCR needed``. Books without bookmarks can opt into a recurring opening
phrase (``--chapter-open``) to locate chapter starts.

The audit only reads the PDF; it never rewrites book text. It is idempotent:
the same PDF gives the same result, and an existing result is reused until the
PDF or the configured opening phrase changes.
"""

import re

TEXT_PAGE_MIN = 40           # fewer text-layer characters: little/no text on the page
GARBAGE_RATE_MAX = 0.02
MIN_USABLE_CHARS = 100
_GARBAGE_RE = re.compile("[\ufffd\u0000-\u0008\u000b\u000c\u000e-\u001f\ue000-\uf8ff]")
_NORM = re.compile(r"[^a-z0-9]+")


def _norm(text):
    return _NORM.sub("", text.lower())


def _page_texts(doc):
    out = []
    for i in range(doc.page_count):
        try:
            out.append(doc[i].get_text("text") or "")
        except Exception:  # a broken page is recorded, not fatal
            out.append("")
    return out


def _toc(doc):
    try:
        rows = doc.get_toc(simple=True) or []
    except Exception:
        return []
    out = []
    for row in rows:
        try:
            level, title, page = row
            out.append({"level": int(level), "title": str(title), "page": int(page)})
        except (TypeError, ValueError):
            continue
    return out


def chapter_starts(texts, toc=None, phrase=None):
    """Chapter starts from a recurring opening phrase, else from the TOC.

    A page matches when its first text line starts with the configured phrase
    (case/punctuation/whitespace are ignored). This is how books without
    bookmarks get usable chapter starts.
    """
    phrase = (phrase or "").strip()
    if phrase:
        q = _norm(phrase)
        out = []
        if q:
            for i, text in enumerate(texts):
                lines = [ln.strip() for ln in text.splitlines() if ln.strip()]
                if not lines:
                    continue
                first = _norm(lines[0])
                if first.startswith(q) or q in first[: max(len(q) + 20, 80)]:
                    out.append({"page": i + 1, "title": lines[0]})
        return out
    toc = toc or []
    if not toc:
        return []
    top = min({t["level"] for t in toc} or {1})
    return [{"page": t["page"], "title": t["title"]} for t in toc
            if t["level"] == top and t["page"] > 0]


def audit_doc(doc, chapter_open=None):
    """Return the book's intake audit. `doc` is an open PyMuPDF document."""
    texts = _page_texts(doc)
    pages = len(texts)
    chars_per_page = [len(t.strip()) for t in texts]
    total = sum(chars_per_page)
    zero = sum(1 for n in chars_per_page if n == 0)
    low = sum(1 for n in chars_per_page if n < TEXT_PAGE_MIN)
    garbage = sum(len(_GARBAGE_RE.findall(t)) for t in texts)
    rate = round(garbage / total, 4) if total else 0.0
    toc = _toc(doc)
    scanned = bool(pages) and (zero >= pages * 0.5 or low >= pages * 0.6)
    guess = "unknown" if not pages else ("scanned" if scanned else "digital")

    reasons = []
    if not pages:
        reasons.append("PDF has no pages")
    elif total < MIN_USABLE_CHARS:
        reasons.append(f"only {total} text-layer characters")
    if pages and zero > max(1, pages // 4):
        reasons.append(f"{zero}/{pages} pages have no text layer")
    if rate >= GARBAGE_RATE_MAX:
        reasons.append(f"{rate:.2%} replacement/control characters")
    if scanned:
        reasons.append("most pages look scanned (no usable text layer)")
    verdict = "OCR needed" if reasons else "text layer usable"
    phrase = (chapter_open or "").strip()
    return {
        "pages": pages,
        "chars_total": total,
        "chars_per_page": chars_per_page,
        "zero_pages": zero,
        "low_text_pages": low,
        "garbage_chars": garbage,
        "garbage_rate": rate,
        "scanned": scanned,
        "scanned_guess": guess,
        "bookmarks": len(toc),
        "toc": toc,
        "chapter_open": phrase,
        "chapters": chapter_starts(texts, toc, phrase or None),
        "verdict": verdict,
        "reasons": reasons,
    }


def ensure(manifest, doc, chapter_open=None):
    """Compute the audit once and keep it in the manifest (idempotent)."""
    old = manifest.get("intake")
    phrase = (chapter_open or "").strip()
    if isinstance(old, dict):
        # A new/changed --chapter-open re-detects chapter starts, but the
        # cheap text-layer fields do not change without a new PDF. An omitted
        # option keeps the phrase/chapters already stored.
        if not phrase or phrase == old.get("chapter_open", ""):
            return old
    new = audit_doc(doc, phrase or None)
    manifest["intake"] = new
    return new


def brief(audit):
    """One readable line for status/audit/lookup output."""
    if not audit:
        return "intake: not audited yet (run pdf2md audit)"
    bits = [audit.get("verdict", "?")]
    if audit.get("garbage_rate"):
        bits.append("garbage %.2f%%" % (audit["garbage_rate"] * 100))
    if audit.get("zero_pages"):
        bits.append("0-char %s/%s" % (audit["zero_pages"], audit.get("pages", "?")))
    if audit.get("bookmarks"):
        bits.append("bookmarks %s" % audit["bookmarks"])
    if audit.get("chapters"):
        bits.append("chapter starts %s" % len(audit["chapters"]))
    return "intake: " + "; ".join(bits)


def from_manifest(manifest):
    a = (manifest or {}).get("intake")
    return a if isinstance(a, dict) else None
