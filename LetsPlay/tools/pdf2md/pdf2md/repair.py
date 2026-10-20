"""Text repair passes for OCR / layout-extracted Markdown.

Ported from the owner's ConvertPDF project (scripts/pdf_to_markdown.py,
scripts/ocr_repair.py, scripts/fix_glitches.py), general rules only. Book-
specific corrections were dropped. See NOTICE.
"""

import os
import re

WORDS = set()
FUNC = set(
    """
a the and or but if of to in on at by up as is it be no so do go we he me my us are was
were been being has have had can could will would shall should may might must did does done
not any some all both each few more most other own same than too very just also into onto upon
out off over under again further then once here there when where why how what which who whom
this that these those because while during before after above below from down between through
against among within without toward towards per via ever never always often sometimes usually
rarely your you their its his her our one two for with about
""".split()
)
CONTRACTION_TAILS = {"t", "s", "d", "re", "ve", "ll", "m"}
ORDINAL_TAILS = {"st", "nd", "rd", "th"}
SINGLE_WORDS = {"a", "i"}
_STOPS = ("and", "or", "of", "the", "to", "in", "on", "for", "with", "a", "an",
          "into", "out", "up", "off", "over", "under")


def load_words(path):
    """Load a one-word-per-line list. Returns the number of words loaded."""
    global WORDS
    if not path or not os.path.exists(path):
        WORDS = set()
        return 0
    with open(path, encoding="utf-8", errors="ignore") as f:
        WORDS = {w.strip().lower() for w in f if w.strip()}
    return len(WORDS)


def set_words(words):
    global WORDS
    WORDS = {w.lower() for w in words}


_CORE_RE = re.compile(r"[A-Za-z]+(?:'[A-Za-z]+)?")


def _letters(tok):
    cores = _CORE_RE.findall(tok)
    if len(cores) != 1:
        return None
    core = cores[0].lower()
    alpha = sum(1 for c in core if c.isalpha() or c == "'")
    if alpha / len(tok) < 0.7:
        return None
    return core


def _merge_pass(toks):
    i, res, n, merged_any = 0, [], len(toks), False
    while i < n:
        merged = False
        for k in range(min(6, n - i), 1, -1):
            group = toks[i:i + k]
            if any(not t for t in group):
                continue
            letters = [_letters(t) for t in group]
            if any(x is None for x in letters):
                continue
            cand = "".join(x.replace("'", "") for x in letters)
            left, right = letters[0], letters[-1]
            if len(cand) < 4 and not (len(cand) == 2 and len(left) == 1 and len(right) == 1):
                continue
            if cand not in WORDS:
                continue
            if right in SINGLE_WORDS:
                continue
            if i + k < n:
                nxt = _letters(toks[i + k])
                if nxt and (right + nxt.replace("'", "")) in WORDS:
                    continue
            if left in FUNC and len(right) >= 2 and right in WORDS:
                continue
            if right in CONTRACTION_TAILS and left.endswith("n"):
                continue
            res.append("".join(group))
            i += k
            merged = merged_any = True
            break
        if not merged:
            res.append(toks[i])
            i += 1
    return res, merged_any


def _fix_digit_spacing(line):
    def _split_dl(m):
        d, w = m.group(1), m.group(2)
        if w.lower() in ORDINAL_TAILS or len(w) < 3:
            return m.group(0)
        return d + " " + w
    line = re.sub(r"\b(\d+)([A-Za-z]{3,})\b", _split_dl, line)
    line = re.sub(r"\b([A-Za-z]{3,})(\d+)\b", lambda m: m.group(1) + " " + m.group(2), line)
    return line


def repair_text(text):
    """Fix OCR pseudo-spaces, soft hyphens and digit/word spacing."""
    if not text:
        return text
    text = text.replace("\u00ad", "")
    text = re.sub(r"(?<=[A-Za-z])- (?=[a-z])", "-", text)
    out = []
    for line in text.split("\n"):
        if not line.strip() or re.fullmatch(r"\|?[\s:|-]+\|?", line):
            out.append(line)
            continue
        toks = line.split(" ")
        if len(toks) >= 2 and WORDS:
            for _ in range(4):
                toks, again = _merge_pass(toks)
                if not again:
                    break
            line = " ".join(toks)
        out.append(_fix_digit_spacing(line))
    return "\n".join(out)


def letterspaced_ratio(text):
    toks = [t for t in text.split(" ") if t]
    if not toks:
        return 0.0
    return sum(1 for t in toks if len(t) == 1) / len(toks)


def _split_glued(w):
    core = re.sub(r"[^a-z0-9]", "", w.lower())
    if not WORDS or core in WORDS:
        return [w]
    lw = w.lower()
    for s in _STOPS:
        if lw.endswith(s) and len(w) > len(s) + 1 and w[:-len(s)].lower() in WORDS:
            return _split_glued(w[:-len(s)]) + [w[-len(s):]]
    for i in range(len(w) - 1, 0, -1):
        for s in _STOPS:
            if lw[i:i + len(s)] == s:
                head, tail = w[:i], w[i:]
                if head.lower() in WORDS:
                    parts = _split_glued(tail)
                    if len(parts) > 1 or tail.lower() in FUNC:
                        return _split_glued(head) + parts
    return [w]


def repair_letterspaced(text):
    """'C h a p t e r 1' -> 'Chapter 1' (only for clearly letter-spaced text)."""
    toks = [t for t in text.split(" ") if t]
    if not toks or letterspaced_ratio(text) < 0.6:
        return text
    words = []
    for t in toks:
        stripped = t.lstrip("([<'\"\u201c\u201d\u2018\u2019\u2014-\u2022*#|")
        starts_new = stripped and (stripped[0].isupper() or stripped[0].isdigit())
        if starts_new and words:
            words.append(t)
        elif words:
            words[-1] += t
        else:
            words.append(t)
    fixed = []
    for w in words:
        mm = re.match(r"^([A-Za-z]+)(\d+[:,;.]?)$", w)
        if mm:
            fixed += [mm.group(1), mm.group(2)]
        else:
            fixed.append(w)
    out = []
    for w in fixed:
        out.extend(_split_glued(w))
    return " ".join(out)


# ---------------------------------------------------------------- dice / numbers
_NUMTOK_RE = re.compile(r"^[0-9OlI|]+$")


def fix_dice_numbers(tok):
    """Token-level dice/number normalisation: ld6 -> 1d6, dlOO -> d100, 2O -> 20."""
    m = re.fullmatch(r"(\d*[lI|]?)d([0-9OlI|]+)([.,;:)]?)", tok)
    if m and re.search(r"\d", m.group(2).replace("l", "1").replace("O", "0")):
        head = m.group(1).replace("l", "1").replace("I", "1").replace("|", "1")
        sides = m.group(2).replace("l", "1").replace("I", "1").replace("|", "1").replace("O", "0")
        if sides.isdigit():
            return head + "d" + sides + m.group(3)
    if _NUMTOK_RE.match(tok) and re.search(r"\d", tok):
        t = tok.replace("l", "1").replace("I", "1").replace("|", "1").replace("O", "0")
        if t != tok and not t.startswith("0"):
            return t
    return tok


def fix_line_structure(body):
    body = re.sub(r"\(([-+])\s+(\d)", r"(\1\2", body)
    body = re.sub(r"\{([-+]?\d+)\)", r"(\1)", body)
    body = re.sub(r"(\d)\s+,(\d{3})\b", r"\1,\2", body)
    body = re.sub(r"(\d)ft\.", r"\1 ft.", body)
    body = re.sub(r"(?<=\w) +([,;])", r"\1", body)
    return body


def fix_dice_line(line):
    return " ".join(fix_dice_numbers(t) for t in line.split(" "))


# ---------------------------------------------------------------- structure
PIC_BLOCK_RE = re.compile(
    r"<!--\s*Start of picture text\s*-->.*?<!--\s*End of picture text\s*-->", re.S | re.I)
PIC_MARK_RE = re.compile(r"<!--\s*(?:Start|End) of picture text\s*-->", re.I)


def strip_picture_text(md, stats=None):
    n = len(PIC_BLOCK_RE.findall(md))
    if stats is not None:
        stats["pic"] = stats.get("pic", 0) + n
    md = PIC_BLOCK_RE.sub("", md)
    md = PIC_MARK_RE.sub("", md)
    md = re.sub(r"(?m)^(?:\s|<br\s*/?>|'|\")+\s*$", "", md)
    return md


def structural_pass(lines):
    out = []
    for ln in lines:
        s = ln.strip()
        if re.fullmatch(r"~{3,}", s):
            continue
        if re.fullmatch(r"[\u2022\u00b7*\-_=~.]{4,}", s):
            continue
        if re.fullmatch(r"[Il\dO]{1,4}", s):  # lone page number
            continue
        out.append(ln)
    return out


# Generic glitch rules (book-specific lines were dropped).
GLITCH_RULES = [
    (r"\b(\d+) d(\d+) (\d{1,2})\b(?= *\))", r"\1d\2\3"),
    (r"\bld(\d{1,2}) \+ (\d)\b", r"1d\1 + \2"),
    (r"(?<![A-Za-z])(Ring|Armor|Staff|Wand|Potion|Cloak|Boots|Gloves|Belt|Bracers|Helm|Rod|Orb|Bag|Horn|Robe|Rope)of(?=[A-Za-z ])",
     r"\1 of "),
    (r"\bthem selves\b", "themselves"),
    (r"(?<=\S) {2,}(?=\S)", " "),
]

DIGIT1_LIST = re.compile(r"(- \d{1,2} )1\b(?= [a-z])")
DIGIT1_QUOTE = re.compile(r"(?<=[\u201c\"])1\b(?= [a-z])")
DIGIT1_AFTER = re.compile(r"\b(what|that|if|when|before|someone|people|who|why|how|but|so) 1 (?=[a-z])")
I_VERBS = set("""
have had am know think want need like love hate prefer see get go do would will can must
should could may might was wonder suspect believe hope wish try keep take make find feel
remember realize expect intend plan agree promise swear suppose guess doubt admit insist
look say tell ask also never often sometimes usually really truly only just still even
rather idolize came saw heard went said found got took gave made knew thought felt
""".split())


def fix_digit_one(md):
    md = DIGIT1_LIST.sub(r"\1I", md)
    md = DIGIT1_QUOTE.sub("I", md)

    def _sub(m):
        tail = md_ref[0][m.end():m.end() + 14]
        mw = re.match(r"[A-Za-z\u2019']+", tail)
        if not mw or mw.group(0).lower() not in I_VERBS:
            return m.group(0)
        return m.group(1) + " I "
    md_ref = [md]
    return DIGIT1_AFTER.sub(_sub, md)


# ---------------------------------------------------------------- OCR'd text layers
_GLUE_HEADS = ("of", "if", "to", "in", "on", "at", "is", "as", "by", "and", "the", "that", "for", "with")
_TOKEN_RE = re.compile(r"\b[A-Za-z]{3,}\b")


def _fix_word(w):
    lw = w.lower()
    if not WORDS or lw in WORDS:
        return w
    # l-for-I at word start: "lnitiative" -> "Initiative", "lllusion" -> "Illusion"
    if w[0] == "l" and ("i" + lw[1:]) in WORDS:
        return "I" + w[1:]
    if w[:2] in ("lI", "Il", "ll") and ("i" + lw[1:]) in WORDS:
        return "I" + w[1:]
    # glued function word: "ofyour" -> "of your", "thatyou" -> "that you"
    for h in _GLUE_HEADS:
        rest = lw[len(h):]
        if w.islower() and lw.startswith(h) and len(rest) >= 2 and rest in WORDS and (len(rest) >= 3 or rest in FUNC):
            return w[:len(h)] + " " + w[len(h):]
    return w


_SUP_WORD_RE = re.compile(r"(\w?)<sup>([A-Za-z][A-Za-z,.;:'\u2019 -]{2,})</sup>")


def fix_ocr_words(md):
    """Dictionary-checked fixes for text layers made by OCR software."""
    # baseline jitter turns ordinary words into <sup>words</sup>; real superscripts are short
    md = _SUP_WORD_RE.sub(lambda m: m.group(1) + (" " if m.group(1) else "") + m.group(2), md)
    out = []
    for line in md.split("\n"):
        if line.startswith("<a id=") or line.startswith("<!--") or line.startswith("!["):
            out.append(line)
            continue
        out.append(_TOKEN_RE.sub(lambda m: _fix_word(m.group(0)), line))
    return "\n".join(out)


def apply_glitch_rules(md):
    for pat, rep in GLITCH_RULES:
        md = re.sub(pat, rep, md)
    return fix_ocr_words(fix_digit_one(md))
