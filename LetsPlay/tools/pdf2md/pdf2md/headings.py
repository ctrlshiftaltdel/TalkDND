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
    body = fix_room_code(body)
    lvl = hmap.get(key) or hmap.get(norm_key(body)) or min(len(m.group(1)), 3)
    return "#" * (lvl + 1) + " " + body


# Adventure room keys: "V1: GRAND ENTRANCE", "B12. Vault". OCR reads 1 as l/I.
ROOM_CODE_RE = re.compile(r"^([A-Z])([A-Z]?[0-9lIOS|]{1,3}[a-z]?)([:.])\s*(?=[A-Z])")
ROOM_LINE_RE = re.compile(
    r"^(?:\*\*)?([A-Z]{1,2}[0-9]{1,3}[a-z]?)[:.]\s+([A-Z][A-Za-z'\u2019 ,&\-]{2,60}?)(?:\*\*)?\s*$")
MAP_LINE_RE = re.compile(r"^(?:\*\*)?(MAP\s+\d+\.\d+\s*:\s*.{2,60}?)(?:\*\*)?\s*$", re.I)


def fix_room_code(text):
    """`Vl: GRAND ENTRANCE` -> `V1: GRAND ENTRANCE`; `Vl4`/`VI4` -> `V14`, `VlO` -> `V10`,
    `V5:TITLE` -> `V5: TITLE`. Needs a digit-like run starting with a digit, l, I or |."""
    m = ROOM_CODE_RE.match(text)
    if not m:
        return text
    num = m.group(2)
    run = re.match(r"[A-Z]?[0-9lI|][0-9lIOS|]*", num)
    if not run:
        return text
    digits = run.group(0)
    rest = num[len(digits):]
    if rest and not re.fullmatch(r"[a-z]", rest):
        return text
    mapped = digits.translate(str.maketrans("lI|OS", "11105"))
    if not mapped.isdigit():  # a real two-letter code such as `AB3`
        if not (digits[0].isupper() and mapped[1:].isdigit()):
            return text
        return m.group(1) + digits[0] + mapped[1:] + rest + m.group(3) + " " + text[m.end():]
    if mapped.startswith("0") or len(mapped) > 3:
        return text
    return m.group(1) + mapped + rest + m.group(3) + " " + text[m.end():]


def promote_room_lines(lines, level=4):
    """Room keys and map captions on their own line become headings so the index lists them."""
    out = []
    for ln in lines:
        s = ln.strip()
        if s and not HEADING_RE.match(ln) and not s.startswith(("|", "!", "<", ">")):
            s2 = fix_room_code(s.strip("*").strip())
            if ROOM_LINE_RE.match(s2) or MAP_LINE_RE.match(s2):
                ln = "#" * level + " " + s2.strip("*").strip()
        out.append(ln)
    return out
