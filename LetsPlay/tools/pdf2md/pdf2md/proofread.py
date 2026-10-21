"""Deterministic proofreading of assembled Markdown, ported from ConvertPDF
(``fix_ocr.py`` / ``polish.py``, MIT, by the project owner).

Rules (body text only):

* NUM    digit/letter mix-ups in game numbers: ``Sd6`` -> ``5d6``, ``ld8`` -> ``1d8``,
         ``1S`` -> ``15``, ``SO ft.`` -> ``50 ft.``, ``2 d6`` -> ``2d6``
* ONE    digit 1 standing for the pronoun I before a first-person verb
* SPLIT  letter-spaced words: ``charac ter`` -> ``character``
* SPACE  missing space after punctuation (``word,word``), space before
         punctuation, repeated spaces
* GLUE   two words glued together: ``ofYsgard`` -> ``of Ysgard``,
         ``runby`` -> ``run by``

Never touched: page anchors, HTML comments / review markers, image and link
targets, inline code, table rows and headings (headings come from the PDF font map).
Word frequencies come from the book itself; the dictionary is the pdf2md word list.
Every change is logged for the proofread report.
"""

import collections
import re

from . import repair

TOKEN = re.compile(r"[A-Za-z][A-Za-z'\u2019]*")
# Spans that must survive byte-for-byte.
PROTECT = re.compile(
    r"<!--.*?-->|<a id=[^>]*></a>|!\[[^\]]*\]\([^)]*\)|\]\([^)]*\)|`[^`]*`|<[^>]+>|https?://\S+")
SKIP_LINE = re.compile(r"^\s*(?:<a id=|<!--|#{1,6}\s|\||!\[|```|>\s*PDF pages)")
VERBS = ("have", "has", "had", "am", "was", "know", "think", "want", "wanted", "can",
         "cannot", "will", "would", "do", "did", "never", "always", "hope", "believe",
         "need", "must", "should", "could", "may", "see", "saw", "said", "say", "got",
         "went", "like", "love", "hate", "feel", "felt", "wish", "swear", "promise",
         "don't", "didn't", "can't", "won't", "don\u2019t", "didn\u2019t", "can\u2019t")
STOP = {"a", "an", "the", "is", "are", "was", "were", "to", "of", "in", "on", "at", "as",
        "it", "be", "by", "or", "and", "that", "this", "with", "for", "from", "not", "no",
        "so", "if", "we", "he", "she", "they", "i", "you", "my", "your", "his", "her",
        "its", "our", "their"}
GLUE_HEADS = ("of", "in", "to", "and", "the", "on")


def _mask(line):
    saved = []

    def keep(m):
        saved.append(m.group(0))
        return "\x00%d\x00" % (len(saved) - 1)

    return PROTECT.sub(keep, line), saved


def _unmask(line, saved):
    return re.sub(r"\x00(\d+)\x00", lambda m: saved[int(m.group(1))], line)


def _window(a, b, pad=25):
    """The changed part of a line with a little context on each side."""
    i = 0
    while i < min(len(a), len(b)) and a[i] == b[i]:
        i += 1
    j = 0
    while j < min(len(a), len(b)) - i and a[-1 - j] == b[-1 - j]:
        j += 1
    lo = max(0, i - pad)
    return a[lo:len(a) - j + pad].strip(), b[lo:len(b) - j + pad].strip()


def num_fix(line):
    s = line
    s = re.sub(r"\bSd(\d+)\b", r"5d\1", s)
    s = re.sub(r"\bld(\d+)\b", r"1d\1", s)
    s = re.sub(r"\bl0d(\d+)\b", r"10d\1", s)
    s = re.sub(r"\b(\d)S\b", r"\g<1>5", s)
    s = re.sub(r"\bSO\b(?=\s*(?:percent|%|XP|ft|feet|gp|sp|cp|points))", "50", s)
    s = re.sub(r"\bS0\b", "50", s)
    s = re.sub(r"\bS(?=\s+(?:ft\b|feet\b))", "5", s)
    s = re.sub(r"\b(\d+)d(\d)\s(\d)\b(?=\s*[+\-)]|\s*$)", r"\1d\2\3", s)
    s = re.sub(r"\b(\d)\s+d(\d+)\b", r"\1d\2", s)
    return s


def one_fix(line):
    pat = r"(^|[\s\u201c\"(:>\u2014\u2013])1 (?=(?:%s)\b)" % "|".join(re.escape(v) for v in VERBS)
    return re.sub(pat, lambda m: m.group(1) + "I ", line)


class Proofreader:
    def __init__(self, md, words=None):
        self.lex = words if words is not None else repair.WORDS
        self.freq = collections.Counter(w.lower() for w in TOKEN.findall(md))
        self.hyph = collections.Counter(
            h.lower() for h in re.findall(r"\b[A-Za-z]{2,}-[A-Za-z]{2,}\b", md))
        self.log = []
        self.no = 0

    def add(self, kind, before, after):
        self.log.append((kind, self.no, before, after))

    def f(self, w):
        return self.freq.get(w.lower(), 0)

    def known(self, w):
        w = w.lower().replace("\u2019", "'")
        return w in self.lex or self.f(w) >= 5

    def split_fix(self, line):
        toks = list(TOKEN.finditer(line))
        if len(toks) < 2:
            return line
        out, pos, i = [], 0, 0
        while i < len(toks) - 1:
            a, b = toks[i], toks[i + 1]
            al, bl = a.group(0).lower(), b.group(0).lower()
            joined = al + bl
            base = re.sub(r"(?:['\u2019]s|s['\u2019]|['\u2019])$", "", joined)
            ok = (line[a.end():b.start()] == " " and 1 <= len(al) <= 7
                  and al not in STOP and bl not in STOP
                  and not b.group(0)[:1].isupper() and len(base) >= 4
                  and (joined in self.lex or base in self.lex)
                  # at least one fragment is not a word on its own ("charac ter"),
                  # so real word pairs ("look up") are never joined
                  and (al not in self.lex or bl not in self.lex)
                  and max(self.f(joined), self.f(base)) >= 2)
            if ok:
                out.append(line[pos:a.start()] + a.group(0) + b.group(0))
                self.add("SPLIT", f"{a.group(0)} {b.group(0)}", a.group(0) + b.group(0))
                pos = b.end()
                i += 2
                continue
            i += 1
        if not out:
            return line
        return "".join(out) + line[pos:]

    def segment(self, low, max_parts=4):
        """Best split of `low` into 2..max_parts dictionary words: the score is the
        rarest part's frequency in the book (short heads such as `of` count 4x),
        ties go to fewer parts."""
        memo = {}

        def part_ok(x, first):
            return x in self.lex and (len(x) >= 3 or x in GLUE_HEADS or (not first and len(x) == 2))

        def part_score(x):
            return self.f(x) * (4 if x in GLUE_HEADS else 1)

        def go(i, left):
            key = (i, left)
            if key in memo:
                return memo[key]
            rest = low[i:]
            best = (part_score(rest), (rest,)) if part_ok(rest, i == 0) else None
            if left > 1:
                for j in range(i + 2, len(low) - 1):
                    x = low[i:j]
                    if not part_ok(x, i == 0):
                        continue
                    sub = go(j, left - 1)
                    if sub is None:
                        continue
                    cand = (min(part_score(x), sub[0]), (x,) + sub[1])
                    if best is None or (cand[0], -len(cand[1])) > (best[0], -len(best[1])):
                        best = cand
            memo[key] = best
            return best

        best = None
        for j in range(2, len(low) - 1):
            x = low[:j]
            if not part_ok(x, True):
                continue
            sub = go(j, max_parts - 1)
            if sub is None:
                continue
            cand = (min(part_score(x), sub[0]), (x,) + sub[1])
            if best is None or (cand[0], -len(cand[1])) > (best[0], -len(best[1])):
                best = cand
        return best

    def glue_fix(self, line):
        def sub(m):
            w = m.group(0)
            mm = re.fullmatch(r"(of|in|to|and|the)([A-Z][a-z]{2,})", w)
            if mm:
                self.add("GLUE", w, f"{mm.group(1)} {mm.group(2)}")
                return f"{mm.group(1)} {mm.group(2)}"
            low = w.lower()
            if not w.islower() or len(low) < 5 or self.known(low) or not self.lex:
                return w
            best = self.segment(low)
            if best is None or best[0] < 20:
                return w
            self.add("GLUE", w, " ".join(best[1]))
            return " ".join(best[1])

        return TOKEN.sub(sub, line)

    def space_fix(self, line):
        def sp(m):
            a, p, b = m.group(1), m.group(2), m.group(3)
            if p != "." and self.hyph.get(f"{a}-{b}".lower(), 0) >= 1:
                return f"{a}-{b}"  # OCR read the hyphen as punctuation
            if (a + b).lower() in self.lex or not (self.known(a) and self.known(b)):
                return m.group(0)
            if a.isupper() or b.isupper() or (p == "." and len(b) <= 3):
                return m.group(0)
            return a + p + " " + b

        s = re.sub(r"\b([A-Za-z]{2,})([,;:]|\.(?=[A-Z]))([A-Za-z]{3,})\b", sp, line)
        s = re.sub(r"\b([A-Za-z]{2,})(?:- | -| - )([A-Za-z]{2,})\b",
                   lambda m: (f"{m.group(1)}-{m.group(2)}"
                              if self.hyph.get(f"{m.group(1)}-{m.group(2)}".lower(), 0) >= 1 else m.group(0)), s)
        s = re.sub(r"(?<=\w) +([.,;:!?])(?![.\w])", r"\1", s)
        s = re.sub(r"(?<=\S)[ \t]{2,}(?=\S)", " ", s)
        return s

    def line(self, line, no):
        if not line.strip() or SKIP_LINE.match(line):
            return line
        prefix = re.match(r"^(\s*(?:[-*+]|\d+\.|>)?\s*)", line).group(1)
        body, saved = _mask(line[len(prefix):])
        self.no = no
        for kind, fn in (("NUM", num_fix), ("ONE", one_fix), ("SPLIT", self.split_fix),
                         ("GLUE", self.glue_fix), ("SPACE", self.space_fix)):
            new = fn(body)
            if new != body and kind in ("NUM", "ONE", "SPACE"):
                self.add(kind, *_window(_unmask(body, saved), _unmask(new, saved)))
            body = new
        return prefix + _unmask(body, saved)

    def run(self, md):
        return "\n".join(self.line(ln, no) for no, ln in enumerate(md.split("\n"), 1))


def proofread(md, words=None):
    """(new markdown, [(rule, line number, before, after), ...])."""
    p = Proofreader(md, words)
    out = p.run(md)
    return out, p.log


def report(stem, log):
    counts = collections.Counter(e[0] for e in log)
    lines = [f"# Proofread report: {stem}", "",
             "Deterministic fixes applied to body text (anchors, markers, links, tables and headings untouched). "
             "Line numbers refer to the assembled book before proofreading. This file stays local.", "",
             "| Rule | Changes |", "|------|--------:|"]
    for k in ("NUM", "ONE", "SPLIT", "SPACE", "GLUE"):
        lines.append(f"| {k} | {counts.get(k, 0)} |")
    lines += ["", "## Changes", ""]
    for kind, no, a, b in log[:5000]:
        lines.append(f"- {kind} L{no}: `{a}` -> `{b}`")
    return "\n".join(lines) + "\n"
