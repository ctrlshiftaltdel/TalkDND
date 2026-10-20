"""Heading levels from PDF font sizes. Ported from ConvertPDF."""

import re

from . import repair

HEADING_FONT_EXCLUDE = ("angsana", "segoe", "symbol", "wingding")
HEADING_RE = re.compile(r"^(#{1,6})\s+(.*?)\s*$")
_NORM = re.compile(r"[^a-z0-9]+")


def norm_key(s):
    return _NORM.sub("", s.lower())


def build_heading_map(doc, pages=None):
    """{normalised text: level}. Level 1 = chapter, 2 = section, 3 = subsection."""
    raw = {}
    for i in pages if pages is not None else range(doc.page_count):
        d = doc[i].get_text("dict")
        for b in d["blocks"]:
            if b.get("type") != 0:
                continue
            for line in b["lines"]:
                spans = [s for s in line["spans"] if s["text"].strip()]
                if not spans:
                    continue
                if any(x in spans[0]["font"].lower() for x in HEADING_FONT_EXCLUDE):
                    continue
                size = max(s["size"] for s in spans)
                txt = "".join(s["text"] for s in spans).strip()
                if len(txt) < 2 or len(txt) > 80:
                    continue
                if size < 12.2 and not (8.7 <= size <= 9.4 and "bold" in spans[0]["font"].lower()):
                    continue
                key = norm_key(txt)
                if key and key not in raw:
                    raw[key] = size
    if not raw:
        return {}
    top = max(raw.values())
    return {k: (1 if s >= 0.85 * top else 2 if s >= 12.2 else 3) for k, s in raw.items()}


def normalize_heading(line, hmap):
    m = HEADING_RE.match(line)
    if not m:
        return line
    body = m.group(2).strip()
    if body.startswith("**") and body.endswith("**") and len(body) > 4:
        body = body[2:-2]
    body = re.sub(r"\*\*(.+?)\*\*", r"\1", body)
    body = re.sub(r"</?(?:u|mark|b|i|em|strong)>", "", body)
    key = norm_key(body)
    if repair.letterspaced_ratio(body) >= 0.6:
        body = repair.repair_letterspaced(body)
    else:
        body = repair.repair_text(body).strip()
    body = re.sub(r"[\s_]+$", "", body).strip()
    lvl = hmap.get(key) or hmap.get(norm_key(body)) or min(len(m.group(1)), 3)
    return "#" * (lvl + 1) + " " + body
