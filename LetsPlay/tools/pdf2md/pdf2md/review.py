"""Agent review of one page: an agent rewrites the page markdown from the PDF;
the result is accepted only when the audit scores do not regress (with a
documented soft exception for cleaned-up structured pages).

The review role is agent-agnostic: `--engine grok` runs Grok Build headless,
`--engine cmd:<command>` runs any command that reads the prompt file given as
its last argument and prints the corrected markdown on stdout.
"""

import json
import os
import re
import shlex
import subprocess
import time

from . import VERSION, audit, fixes

# Soft-acceptance thresholds (documented in docs/LESSONS.md).
RECALL_SOFT_DROP = 0.05       # max allowed recall regression under soft gate
WORD_FLOOR_SOFT = 0.90        # soft gate requires at least this fraction of words
WORD_FLOOR_HARD = 0.50        # always reject below this (deletion)
MAX_SNIPPET_CHARS = 4000      # local SRD / reference text injected into the prompt
DEFAULT_GROK_MAX_TURNS = "60"
RETRY_GROK_MAX_TURNS = "30"

_STRUCTURE = re.compile(
    r"(?i)(?:armor class|hit points|\bchall?enge\b|\bcr\b\s*\d|"
    r"\|\s*STR\s*\||\*\*STR\*\*|\bSpellcasting\b|\bActions\b\s*$)",
    re.M,
)
_NAME_RE = re.compile(r"\*\*([A-Z][A-Za-z' -]{2,40})\*\*|(?:^|\n)#{1,3}\s+([A-Z][A-Za-z' -]{2,60})\s*$", re.M)
_TITLE_RE = re.compile(r"\b([A-Z][a-z]+(?:\s+[A-Z][a-z]+){0,4})\b")

PROMPT = """You are correcting one page of a Markdown conversion of a PDF book (page {page}).
Files in this folder:
- current.md   the current Markdown for the page (may have OCR errors, missing or jumbled text)
- textlayer.txt the PDF's own text layer for the page (may be empty for scanned pages; may have its own OCR errors)
- page.png     the rendered page image: this is the authority

Task: produce the corrected Markdown for this page only. Work efficiently: read the three files once, use the
Local reference snippets below when a name matches, then answer. Do not browse the whole reference books.
Rules (strict):
1. Keep the first line `<a id="p{page}"></a>` exactly. Keep `![...](...)` image links that are in current.md.
2. Fix OCR/spacing errors (e.g. `lnitiative` -> `Initiative`, `ofyour` -> `of your`, broken hyphenation), restore text that
   is on the page but missing, and put two-column text in reading order (left column, then right).
3. Rebuild tables as Markdown pipe tables. Write stat blocks as Markdown (name heading, AC/HP/Speed lines,
   an ability-score pipe table, then traits and actions with **bold** names).
4. Headings: use #/##/### only for real headings on the page; never turn body text into a heading.
5. Anti-deletion: do not drop sentences, table rows, traits, or action lines that are visible on page.png.
   Prefer keeping slightly noisy text over deleting it. Never invent whole new paragraphs, summaries, or translations.
6. Uncertain spots: first consult authoritative sources, in this order: page.png; Local reference snippets below;
   the reference book paths listed further down (SRD and other converted books, read-only; open only a matching
   section, not the whole file); cross-references elsewhere in this book ({book_md}). If still unresolved, make a
   reasonable inference from context and fill it in, marked inline right after it as `<!-- inferred: short reason -->`.
   Use `<!-- check PDF -->` only when something is truly unreadable and cannot be inferred.
7. Do not modify any file. Your final answer is the JSON object {{"markdown": "<the corrected page Markdown>"}}:
   the markdown field holds ONLY the page content, no commentary.

Local reference snippets (from converted SRD / local books; may be empty):
{snippets}

Reference book paths (Markdown, read-only; open only when a snippet is missing):
{refs}
"""

SHORT_PROMPT = """You are correcting one page of a Markdown conversion of a PDF book (page {page}).
This is a RETRY after a previous attempt ran out of turns. Do NOT search or open any file outside this folder.
Read only current.md, textlayer.txt and page.png in this folder, then answer immediately.

Rules:
1. Keep `<a id="p{page}"></a>` as the first line. Keep image links from current.md.
2. Fix OCR/spacing, restore missing visible text, fix column order, rebuild tables and stat blocks as Markdown.
3. Do not delete visible sentences or table rows. Prefer noisy text over deletion.
4. Uncertain: infer from context and mark `<!-- inferred: reason -->`, or `<!-- check PDF -->` if unreadable.
5. Final answer: JSON object {{"markdown": "<corrected page Markdown only>"}}.
"""


def reference_books(book):
    """Converted SRDs first, then the other converted books under the same LetsPlay folder."""
    from . import worker
    from .convert import out_dir

    try:
        root = worker.letsplay_root()
    except Exception:  # pragma: no cover
        root = os.path.dirname(os.path.dirname(os.path.dirname(book.dir)))
    refs = []
    for pdf in worker.scan(root):
        d, stem = out_dir(pdf)
        md = os.path.join(d, stem + ".pages.md")
        if os.path.exists(md) and d != book.dir:
            refs.append(md)
    refs.sort(key=lambda x: 0 if "srd" in os.path.basename(x).lower() else 1)
    return refs


def candidate_names(text, limit=12):
    """Likely spell / monster / item names from the page markdown."""
    names = []
    for m in _NAME_RE.finditer(text or ""):
        names.append((m.group(1) or m.group(2)).strip())
    for m in _TITLE_RE.finditer(text or ""):
        n = m.group(1).strip()
        if " " in n or len(n) >= 6:
            names.append(n)
    out, seen = [], set()
    for n in names:
        k = n.lower()
        if k in seen or len(n) < 3:
            continue
        seen.add(k)
        out.append(n)
        if len(out) >= limit:
            break
    return out


def _section_at(md, start):
    """Return the heading line at start and body until the next heading of same or higher level."""
    line_end = md.find("\n", start)
    if line_end < 0:
        line_end = len(md)
    heading = md[start:line_end]
    level = len(heading) - len(heading.lstrip("#"))
    i = line_end + 1
    while i < len(md):
        if md[i] == "#":
            j = i
            while j < len(md) and md[j] == "#":
                j += 1
            if j - i <= level and (i == 0 or md[i - 1] == "\n"):
                break
        i += 1
    return md[start:i].strip()


def lookup_snippets(names, ref_paths, max_chars=MAX_SNIPPET_CHARS):
    """Pull short matching ##/### sections from converted reference Markdown files."""
    if not names or not ref_paths:
        return ""
    parts, used = [], 0
    for path in ref_paths:
        try:
            md = open(path, encoding="utf-8").read()
        except OSError:
            continue
        lower = md.lower()
        for name in names:
            needle = name.lower()
            # prefer a markdown heading that contains the name
            idx = 0
            found = -1
            while True:
                h = lower.find("\n#", idx)
                if h < 0:
                    break
                line_end = lower.find("\n", h + 1)
                if line_end < 0:
                    line_end = len(lower)
                if needle in lower[h:line_end]:
                    found = h + 1  # skip the leading newline
                    break
                idx = h + 1
            if found < 0:
                continue
            sec = _section_at(md, found)
            if len(sec) < 40:
                continue
            chunk = sec if len(sec) <= 1200 else sec[:1200] + "\n…"
            label = os.path.basename(path)
            block = f"### from {label}: {name}\n{chunk}"
            if used + len(block) > max_chars:
                return "\n\n".join(parts)
            parts.append(block)
            used += len(block)
            if used >= max_chars:
                break
        if used >= max_chars:
            break
    return "\n\n".join(parts) if parts else "(none matched)"


def has_structure_markers(text):
    return bool(_STRUCTURE.search(text or ""))


def page_text(md_pages, p):
    return md_pages.get(p, f'<a id="p{p}"></a>\n')


def prepare(book, p, workroot, short=False):
    from .convert import safe_pixmap

    wd = os.path.join(workroot, "p%04d" % p)
    os.makedirs(wd, exist_ok=True)
    with open(os.path.join(book.dir, book.stem + ".pages.md"), encoding="utf-8") as f:
        pages = audit.split_pages(f.read())
    cur = page_text(pages, p).strip()
    pdf_text = book.doc[p - 1].get_text()
    with open(os.path.join(wd, "current.md"), "w", encoding="utf-8") as f:
        f.write(cur + "\n")
    with open(os.path.join(wd, "textlayer.txt"), "w", encoding="utf-8") as f:
        f.write(pdf_text)
    safe_pixmap(book.doc[p - 1], 110).save(os.path.join(wd, "page.png"))
    refs_list = reference_books(book)
    refs = "\n".join("- " + r for r in refs_list) or "- (none)"
    book_md = os.path.join(book.dir, book.stem + ".pages.md")
    if short:
        body = SHORT_PROMPT.format(page=p)
    else:
        snippets = lookup_snippets(candidate_names(cur + "\n" + pdf_text), refs_list)
        body = PROMPT.format(page=p, refs=refs, book_md=book_md, snippets=snippets or "(none matched)")
    with open(os.path.join(wd, "prompt.md"), "w", encoding="utf-8") as f:
        f.write(body)
    return wd, cur, pdf_text


def run_engine(engine, wd, timeout=900, max_turns=None):
    prompt = os.path.join(wd, "prompt.md")
    if engine == "grok":
        turns = max_turns or os.environ.get("PDF2MD_GROK_MAX_TURNS", DEFAULT_GROK_MAX_TURNS)
        # read-only sandbox: the agent may read local files but cannot write; web search stays off
        cmd = ["grok", "--no-auto-update", "--prompt-file", prompt, "--cwd", wd, "--output-format", "json",
               "--always-approve", "--sandbox", "read-only", "--disable-web-search",
               "--max-turns", str(turns),
               "--json-schema", json.dumps({"type": "object", "properties": {"markdown": {"type": "string"}},
                                            "required": ["markdown"]})]
        if os.environ.get("PDF2MD_GROK_MODEL"):
            cmd += ["-m", os.environ["PDF2MD_GROK_MODEL"]]
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, cwd=wd)
        if r.returncode != 0:
            raise RuntimeError(f"grok exit {r.returncode}: {r.stderr[-300:]}")
        out = json.loads(r.stdout)
        res = out.get("structuredOutput") or out.get("structured_output") or ""
        if isinstance(res, str):
            try:
                res = json.loads(res[res.index("{"):res.rindex("}") + 1])
            except ValueError:
                raise RuntimeError("engine returned no JSON markdown") from None
        return res["markdown"]
    if engine.startswith("cmd:"):
        # Windows paths contain backslashes: split without POSIX escapes there
        argv = [a.strip('"') for a in shlex.split(engine[4:], posix=os.name != "nt")]
        r = subprocess.run(argv + [prompt], capture_output=True, text=True,
                           timeout=timeout, cwd=wd)
        if r.returncode != 0:
            raise RuntimeError(f"engine exit {r.returncode}: {r.stderr[-300:]}")
        return r.stdout
    raise ValueError(f"unknown engine {engine!r}")


def clean_output(text, p):
    t = text.strip()
    t = re.sub(r"^```[a-z]*\n|\n```$", "", t).strip()
    anchor = f'<a id="p{p}"></a>'
    if anchor not in t:
        t = anchor + "\n" + t
    else:
        t = t[t.index(anchor):]
    return t


def judge(before, after, new_text=""):
    """Accept when scores do not regress, with a soft exception for structured pages.

    Hard reject:
      - unknown/1k rises
      - words fall below WORD_FLOOR_HARD of before
    Soft accept (recall may drop by up to RECALL_SOFT_DROP):
      - unknown/1k improves (strictly decreases)
      - words stay ≥ WORD_FLOOR_SOFT of before
      - badness does not worsen
      - page has a `statblock` flag or structure markers in the new text
    Otherwise any recall drop rejects.
    """
    reasons = []
    if after["unknown_1k"] > before["unknown_1k"]:
        reasons.append(f"unknown/1k {before['unknown_1k']} -> {after['unknown_1k']}")
    if after["words"] < WORD_FLOOR_HARD * before["words"]:
        reasons.append(f"words {before['words']} -> {after['words']}")
        return False, reasons

    recall_drop = (
        before["recall"] is not None
        and after["recall"] is not None
        and after["recall"] < before["recall"]
    )
    if recall_drop:
        drop = before["recall"] - after["recall"]
        soft = (
            drop <= RECALL_SOFT_DROP
            and after["unknown_1k"] < before["unknown_1k"]
            and after["words"] >= WORD_FLOOR_SOFT * before["words"]
            and after["badness"] <= before["badness"]
            and ("statblock" in (before.get("flags") or []) or has_structure_markers(new_text))
        )
        if not soft:
            reasons.append(f"recall {before['recall']} -> {after['recall']}")
    return not reasons, reasons


def _accept(book, p, engine, new, rec):
    fixes.write_fix(audit.fix_path(book, p), book.m["sha256"], VERSION, engine, new)
    rec["status"] = "accepted"


def _reject(book, p, engine, new, rec):
    rej = os.path.join(book.dir, "fixes", "rejected", "p%04d.md" % p)
    fixes.write_fix(rej, book.m["sha256"], VERSION, engine, new)
    rec["status"] = "rejected"


def review_page(book, p, engine="grok", workroot=None):
    workroot = workroot or os.path.join(book.dir, "review")
    t0 = time.time()
    wd, cur, pdf_text = prepare(book, p, workroot)
    info = book.m["pages"].get(str(p), {})
    before = audit.score_page(pdf_text, cur, info)
    rec = {"page": p, "engine": engine, "before": before, "time": None}
    try:
        new = clean_output(run_engine(engine, wd), p)
    except Exception as err:
        err_s = str(err)
        if engine == "grok" and "max turns" in err_s.lower():
            try:
                prepare(book, p, workroot, short=True)
                new = clean_output(
                    run_engine(engine, wd, max_turns=os.environ.get("PDF2MD_GROK_RETRY_TURNS", RETRY_GROK_MAX_TURNS)),
                    p,
                )
                rec["retried"] = "max-turns"
            except Exception as err2:
                rec.update(status="error", reasons=[str(err2)[:300]], time=round(time.time() - t0, 1))
                _log(book, rec)
                return rec
        else:
            rec.update(status="error", reasons=[err_s[:300]], time=round(time.time() - t0, 1))
            _log(book, rec)
            return rec
    after = audit.score_page(pdf_text, new, dict(info, quality="ok"))
    ok, reasons = judge(before, after, new)
    rec.update(after=after, reasons=reasons, time=round(time.time() - t0, 1))
    if ok:
        _accept(book, p, engine, new, rec)
    else:
        _reject(book, p, engine, new, rec)
    _log(book, rec)
    return rec


def rejudge_page(book, p, engine="rejudge"):
    """Re-score an existing rejected output with the current gate; no agent call."""
    rej = os.path.join(book.dir, "fixes", "rejected", "p%04d.md" % p)
    if not os.path.exists(rej):
        return {"page": p, "status": "skipped", "reasons": ["no rejected file"]}
    with open(os.path.join(book.dir, book.stem + ".pages.md"), encoding="utf-8") as f:
        pages = audit.split_pages(f.read())
    cur = page_text(pages, p).strip()
    pdf_text = book.doc[p - 1].get_text()
    info = book.m["pages"].get(str(p), {})
    before = audit.score_page(pdf_text, cur, info)
    meta, body = fixes.read_fix(rej)
    new = clean_output(body, p)
    after = audit.score_page(pdf_text, new, dict(info, quality="ok"))
    ok, reasons = judge(before, after, new)
    rec = {"page": p, "engine": engine, "before": before, "after": after,
           "reasons": reasons, "time": 0, "rejudge": True}
    if ok:
        _accept(book, p, meta.get("engine", engine), new, rec)
        try:
            os.remove(rej)
        except OSError:
            pass
    else:
        rec["status"] = "rejected"
    _log(book, rec)
    return rec


def rejudge_book(book):
    """Rejudge every file under fixes/rejected/. Returns the list of records."""
    d = os.path.join(book.dir, "fixes", "rejected")
    if not os.path.isdir(d):
        return []
    pages = []
    for name in sorted(os.listdir(d)):
        m = re.match(r"p(\d+)\.md$", name)
        if m:
            pages.append(int(m.group(1)))
    return [rejudge_page(book, p) for p in pages]


def _log(book, rec):
    os.makedirs(os.path.join(book.dir, "fixes"), exist_ok=True)
    with open(os.path.join(book.dir, "fixes", "review.log.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps(rec) + "\n")
