"""Stat-block parse, validate and cross-check.

For every stat block in the assembled book (2014 `**Armor Class** 12` /
`**Challenge** 3 (700 XP)` and 2024 `AC 17 ... HP 33 (5d8 + 5) ... CR 2 (XP 450;
PB +2)` layouts) the numbers are read and checked:

- ability modifiers against scores (and OCR digit confusions l/I/o/O)
- HP against the average of its hit dice (n*(d+1)/2 rounded down + bonus)
- XP and proficiency bonus against the challenge rating
- AC / HP against the local combat data with the same name (when given)

Blocks that pass get no marker; any other block gets one line
`> ⚠ Stat block check: ...; check PDF p.N.` right before it, in both editions,
so Markdown alone is trustworthy where it is not marked.
"""

import re

CR_XP = {
    "0": (0, 10), "1/8": (25,), "1/4": (50,), "1/2": (100,), "1": (200,), "2": (450,), "3": (700,),
    "4": (1100,), "5": (1800,), "6": (2300,), "7": (2900,), "8": (3900,), "9": (5000,),
    "10": (5900,), "11": (7200,), "12": (8400,), "13": (10000,), "14": (11500,), "15": (13000,),
    "16": (15000,), "17": (18000,), "18": (20000,), "19": (22000,), "20": (25000,), "21": (33000,),
    "22": (41000,), "23": (50000,), "24": (62000,), "25": (75000,), "26": (90000,),
    "27": (105000,), "28": (120000,), "29": (135000,), "30": (155000,),
}

MARK = "> ⚠ Stat block check"
ANCHOR_RE = re.compile(r'<a id="p(\d+)"></a>')
_DIG = str.maketrans({"l": "1", "I": "1", "i": "1", "o": "0", "O": "0", "−": "-", "–": "-", "—": "-"})
_SIGN = r"[+\-−–]\s?[\dlIoO]{1,2}"
START_RE = re.compile(r"\*\*Armor Class\*\*|\bArmor Class\b\s*\d|(?:^|\s)\**AC\**\s+\d{1,2}\b", re.M)
AC_RE = re.compile(r"(?:Armor Class|\bAC)\**\s*([\dlIoO]{1,2})\b")
HP_RE = re.compile(
    r"(?:Hit Points|\bHP)\**\s*([\dlIoOs]{1,4})\s*\(\s*([\dlIoO]{1,2})\s*d\s*([\dlIoO]{1,2})"
    r"\s*(?:([+\-−–])\s*([\dlIoOa]{1,4}))?\s*\)")
CR14_RE = re.compile(r"Challenge\**\s*(\d{1,2}(?:/\d)?)\s*\(\s*([\d,]+)\s*XP\)")
PB14_RE = re.compile(r"Proficiency Bonus\**\s*([+\-]\d)")
CR24_RE = re.compile(r"\bCR\**\s*(\d{1,2}(?:/\d)?)\s*\(\s*XP\s*([\d,]+)(?:[^)]*?PB\s*([+\-]\d))?")
TRIPLE_RE = re.compile(r"(?:^|\s)(\S{2,4})\s+([\dlIoO]{1,2})\s+(" + _SIGN + r")\s+(" + _SIGN + r")(?=\s|$)")
PAIR_RE = re.compile(r"\b([\dlIoO]{1,2})\s*\(\s*(" + _SIGN + r")\s*\)")
HEAD_RE = re.compile(r"^#{1,6}\s+(.+?)\s*$")
ABILS = ["str", "dex", "con", "int", "wis", "cha"]


def _n(s):
    try:
        return int(str(s).translate(_DIG).replace(" ", "").replace(",", ""))
    except ValueError:
        return None


def modifier(score):
    return (score - 10) // 2


def pb_for_cr(cr):
    v = 0 if "/" in cr else int(cr)
    return 2 + max(0, (v - 1) // 4) if v > 0 else 2


def norm_name(s):
    return re.sub(r"[^a-z0-9]+", " ", s.lower()).strip()


def parse(text):
    """Numbers of one stat block (missing fields are absent)."""
    out = {}
    m = AC_RE.search(text)
    if m:
        out["ac"] = _n(m.group(1))
    m = HP_RE.search(text)
    if m:
        out["hp"] = _n(m.group(1))
        out["hd"] = (_n(m.group(2)), _n(m.group(3)))
        bonus = _n(m.group(5)) if m.group(5) else 0
        if bonus is not None and m.group(4) in ("-", "−", "–"):
            bonus = -bonus
        out["hp_bonus"] = bonus
    m = CR14_RE.search(text)
    if m:
        out["cr"], out["xp"] = m.group(1), _n(m.group(2))
        pb = PB14_RE.search(text)
        if pb:
            out["pb"] = _n(pb.group(1))
    else:
        m = CR24_RE.search(text)
        if m:
            out["cr"], out["xp"] = m.group(1), _n(m.group(2))
            if m.group(3):
                out["pb"] = _n(m.group(3))
    flat = " ".join(text.replace("|", " ").split())
    trip = TRIPLE_RE.findall(flat)
    pairs = PAIR_RE.findall(flat)
    if len(trip) >= 6:
        out["abilities"] = [(_n(s), _n(md)) for _, s, md, _ in trip[:6]]
    elif len(pairs) >= 6:
        out["abilities"] = [(_n(s), _n(md)) for s, md in pairs[:6]]
    return out


def check(block, known=None, name=""):
    """(status, issues): status is ok | fail | incomplete."""
    issues, missing = [], []
    for k in ("ac", "hp", "abilities", "cr"):
        if block.get(k) is None:
            missing.append({"ac": "AC", "hp": "HP", "abilities": "abilities", "cr": "CR"}[k])
    hp, hd = block.get("hp"), block.get("hd")
    if hp is not None and hd and None not in hd and block.get("hp_bonus") is not None:
        n, d = hd
        avg = n * (d + 1) // 2 + block["hp_bonus"]
        if avg != hp:
            issues.append(f"HP {hp} ≠ {n}d{d}{block['hp_bonus']:+d} (average {avg})")
        ab = block.get("abilities")
        if ab and ab[2][0] is not None and n and block["hp_bonus"] != n * modifier(ab[2][0]):
            issues.append(f"HP bonus {block['hp_bonus']:+d} ≠ {n} × CON mod {modifier(ab[2][0]):+d}")
    for i, (score, mod) in enumerate(block.get("abilities") or []):
        if score is None or mod is None or not 1 <= score <= 30:
            issues.append(f"{ABILS[i].upper()} unreadable")
        elif modifier(score) != mod:
            issues.append(f"{ABILS[i].upper()} {score} has modifier {modifier(score):+d}, not {mod:+d}")
    cr = block.get("cr")
    if cr is not None:
        if cr not in CR_XP:
            issues.append(f"CR {cr} unknown")
        else:
            if block.get("xp") is not None and block["xp"] not in CR_XP[cr]:
                issues.append(f"CR {cr} is {CR_XP[cr][-1]} XP, not {block['xp']}")
            if block.get("pb") is not None and block["pb"] != pb_for_cr(cr):
                issues.append(f"CR {cr} has PB +{pb_for_cr(cr)}, not {block['pb']:+d}")
    if known and name:
        k = known.get(norm_name(name))
        if k:
            if k.get("ac") is not None and block.get("ac") is not None and k["ac"] != block["ac"]:
                issues.append(f"AC {block['ac']} ≠ combat data {k['ac']}")
            if k.get("hp") is not None and hp is not None and k["hp"] != hp:
                issues.append(f"HP {hp} ≠ combat data {k['hp']}")
    if issues:
        return "fail", issues + ([f"not read: {', '.join(missing)}"] if missing else [])
    if missing:
        return "incomplete", [f"not read: {', '.join(missing)}"]
    return "ok", []


def _name_before(lines, i):
    for j in range(i - 1, max(-1, i - 8), -1):
        m = HEAD_RE.match(lines[j])
        if m:
            return m.group(1).strip("*_ ").title() if m.group(1).isupper() else m.group(1).strip("*_ ")
    return ""


def annotate(md, known=None):
    """Insert ⚠ lines before failing / incomplete blocks; returns (md, results)
    with one {page, name, status, issues} per block."""
    lines = md.split("\n")
    lines = [ln for ln in lines if not ln.startswith(MARK)]  # idempotent
    starts, page, pages = [], 0, {}
    for i, ln in enumerate(lines):
        a = ANCHOR_RE.match(ln)
        if a:
            page = int(a.group(1))
        pages[i] = page
        if START_RE.search(ln) and not ln.lstrip().startswith(("|", ">")):
            if starts and i - starts[-1] < 3:
                continue
            starts.append(i)
    results, inserts = [], {}
    for k, s in enumerate(starts):
        end = min(starts[k + 1] if k + 1 < len(starts) else len(lines), s + 40)
        # stop at the next page anchor's following heading block only by window
        text = "\n".join(lines[s:end])
        b = parse(text)
        if "hp" not in b and "cr" not in b:
            continue  # prose mentioning AC, not a stat block
        name = _name_before(lines, s)
        status, issues = check(b, known, name)
        p = pages.get(s, 0)
        results.append({"page": p, "name": name, "status": status, "issues": issues})
        if status != "ok":
            where = f"check PDF p.{p}" if p else "check the PDF"
            inserts[s] = f"{MARK}: {'; '.join(issues)}; {where}."
    out = []
    for i, ln in enumerate(lines):
        if i in inserts:
            out.append(inserts[i])
        out.append(ln)
    return "\n".join(out), results


def load_known(paths):
    """{normalized name or id: {ac, hp}} from combat-sim yaml files (creatures:)."""
    try:
        import yaml
    except ImportError:  # pragma: no cover
        return {}
    import os

    known = {}
    files = []
    for p in paths:
        if os.path.isdir(p):
            for root, dirs, names in os.walk(p):
                dirs[:] = [d for d in dirs if not d.startswith(".")]
                files += [os.path.join(root, n) for n in names if n.endswith((".yaml", ".yml"))]
        elif os.path.isfile(p):
            files.append(p)
    for f in sorted(files):
        try:
            with open(f, encoding="utf-8") as fh:
                data = yaml.safe_load(fh)
        except Exception:
            continue
        for c in (data or {}).get("creatures", []) if isinstance(data, dict) else []:
            if not isinstance(c, dict):
                continue
            rec = {"ac": c.get("ac") if isinstance(c.get("ac"), int) else None,
                   "hp": c.get("hp") if isinstance(c.get("hp"), int) else None}
            for key in (c.get("name"), c.get("id")):
                if key:
                    known[norm_name(str(key).replace("-", " "))] = rec
    return known
