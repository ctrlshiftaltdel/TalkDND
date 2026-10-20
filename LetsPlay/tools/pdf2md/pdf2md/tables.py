"""Markdown table repair. Ported from ConvertPDF (pdf_to_markdown.fix_tables)."""

import re


def cells(line):
    s = line.strip()
    if s.startswith("|"):
        s = s[1:]
    if s.endswith("|"):
        s = s[:-1]
    return [c.strip() for c in s.split("|")]


def is_sep(line):
    s = line.strip()
    return s.startswith("|") and s.endswith("|") and "-" in s \
        and bool(re.fullmatch(r"\|[\s:\-|]+\|", s))


def _bold_all(row):
    ne = [c for c in row if c]
    return len(ne) >= 2 and all(c.startswith("**") and c.endswith("**") for c in ne)


_D100_RE = re.compile(r"^d\s*[li1]?\s*[o0]{2}$", re.I)


def _is_subheader(row):
    if len([c for c in row if c]) < 2:
        return False
    if _bold_all(row):
        return True
    return bool(_D100_RE.match(re.sub(r"[*_`]", "", row[0]).strip()))


def _trim_trailing(header, rows):
    last = -1
    for r in [header] + rows:
        for k in range(len(r) - 1, -1, -1):
            if r[k]:
                last = max(last, k)
                break
    width = last + 1
    if 0 < width < len(header):
        return header[:width], [r[:width] for r in rows]
    return header, rows


def _no_digits(row):
    return all(not re.search(r"\d", c) for c in row if c)


def _fix_one_table(block):
    header = cells(block[0])
    rows = [cells(b) for b in block[1:]]
    rows = [r for r in rows if any(c for c in r)]
    if rows and sum(1 for c in rows[0] if c) > sum(1 for c in header if c) \
            and ((_bold_all(header) and _bold_all(rows[0]))
                 or (header[0] == "" and all(c for c in rows[0]) and _no_digits(rows[0]))):
        header, rows = rows[0], rows[1:]
    header, rows = _trim_trailing(header, rows)
    ncols = max(1, len(header))
    for idx, r in enumerate(rows):
        if len(r) < ncols:
            r = r + [""] * (ncols - len(r))
        elif len(r) > ncols:
            r = r[:ncols - 1] + [" ".join(x for x in r[ncols - 1:] if x)]
        rows[idx] = r
    fixed = []
    for r in rows:
        if fixed and r[0] == "":
            prev = fixed[-1]
            for k in range(min(ncols, len(r))):
                if r[k]:
                    prev[k] = (prev[k] + " " + r[k]).strip() if prev[k] else r[k]
            continue
        fixed.append(r)
    if fixed and fixed[0][0] == "":
        sec = fixed[1][0].strip() if len(fixed) > 1 else ""
        hdr = re.sub(r"[*_`]", "", header[0]).strip()
        if re.match(r"^d[\dlIO]{1,4}$", hdr, re.I) or sec in ("2", "3"):
            fixed[0][0] = "1"
        elif re.match(r"^(\d{1,3})\s*[-\u2013\u2014]\s*(\d{1,3})$", sec):
            lo = int(re.match(r"^(\d{1,3})", sec).group(1))
            if lo >= 2:
                fixed[0][0] = "01-%02d" % (lo - 1)
    if header[0] == "" and fixed and fixed[0][0].strip() in ("2", "3"):
        header[0] = "1"
    sep = "|" + "|".join(["---"] * ncols) + "|"
    return ["|" + "|".join(header) + "|", sep] + ["|" + "|".join(r) + "|" for r in fixed]


def _split_subtables(header_line, body):
    blocks, cur = [], [header_line]
    for idx, ln in enumerate(body):
        if len(cur) > 1 and _is_subheader(cells(ln)) and idx + 1 < len(body):
            blocks.append(cur)
            cur = [ln]
        else:
            cur.append(ln)
    blocks.append(cur)
    return blocks


def fix_tables(md):
    lines = md.split("\n")
    out, i, n = [], 0, len(lines)
    while i < n:
        if lines[i].startswith("|") and i + 1 < n and is_sep(lines[i + 1]):
            j = i + 2
            while j < n and lines[j].startswith("|"):
                j += 1
            if out and out[-1].strip():
                out.append("")
            for k, sub in enumerate(_split_subtables(lines[i], lines[i + 2:j])):
                if k:
                    out.append("")
                out.extend(_fix_one_table(sub))
            if j < n and lines[j].strip():
                out.append("")
            i = j
        else:
            out.append(lines[i])
            i += 1
    return "\n".join(out)


def check_tables(md):
    """Return a list of (line_no, problem). Ported from ConvertPDF check_md.py."""
    lines = md.split("\n")
    bad, i, n = [], 0, len(lines)
    while i < n:
        if lines[i].startswith("|") and i + 1 < n and is_sep(lines[i + 1]):
            j = i + 2
            while j < n and lines[j].startswith("|"):
                j += 1
            block = lines[i:j]
            ncols = len(cells(block[0]))
            if i > 0 and lines[i - 1].strip():
                bad.append((i + 1, "no blank line before table"))
            if len(cells(block[1])) != ncols:
                bad.append((i + 2, "separator width"))
            for k, b in enumerate(block[2:], start=2):
                if len(cells(b)) != ncols:
                    bad.append((i + k + 1, "row width"))
            if j < n and lines[j].strip():
                bad.append((j + 1, "no blank line after table"))
            i = j
        else:
            i += 1
    if any("picture text" in ln for ln in lines):
        bad.append((0, "picture text residue"))
    return bad
