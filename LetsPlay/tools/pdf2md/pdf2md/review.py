"""Agent review of one page: an agent rewrites the page markdown from the PDF;
the result is accepted only when the audit scores do not regress.

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

PROMPT = """You are correcting one page of a Markdown conversion of a PDF book (page {page}).
Files in this folder:
- current.md   the current Markdown for the page (may have OCR errors, missing or jumbled text)
- textlayer.txt the PDF's own text layer for the page (may be empty for scanned pages; may have its own OCR errors)
- page.png     the rendered page image: this is the authority

Task: produce the corrected Markdown for this page only. Work efficiently: read the three files once, look up
references only for spots you are unsure of (a targeted search, not browsing), then answer.
Rules (strict):
1. Keep the first line `<a id="p{page}"></a>` exactly. Keep `![...](...)` image links that are in current.md.
2. Fix OCR/spacing errors (e.g. `lnitiative` -> `Initiative`, `ofyour` -> `of your`, broken hyphenation), restore text that
   is on the page but missing, and put two-column text in reading order (left column, then right).
3. Rebuild tables as Markdown pipe tables. Write stat blocks as Markdown (name heading, AC/HP/Speed lines,
   an ability-score pipe table, then traits and actions with **bold** names).
4. Headings: use #/##/### only for real headings on the page; never turn body text into a heading.
5. Never add content that is not on the page: no summaries, no notes, no guessing. Do not translate.
6. Uncertain spots: first consult authoritative sources, in this order: page.png; the reference books listed
   below (SRD 5.1 / 5.2.1 and other converted books, read-only; search them for the same spell, monster or item);
   cross-references elsewhere in this book ({book_md}). If still unresolved, make a reasonable inference from
   context and fill it in, marked inline right after it as `<!-- inferred: short reason -->`. Use
   `<!-- check PDF -->` only when something is truly unreadable and cannot be inferred.
7. Do not modify any file. Your final answer is the JSON object {{"markdown": "<the corrected page Markdown>"}}:
   the markdown field holds ONLY the page content, no commentary.
Reference books (Markdown, read-only):
{refs}
"""


def reference_books(book):
    """Converted SRDs first, then the other converted books under the same LetsPlay folder."""
    from . import worker

    root = os.path.dirname(os.path.dirname(os.path.dirname(book.dir)))
    try:
        root = worker.letsplay_root()
    except Exception:  # pragma: no cover
        pass
    from .convert import out_dir

    refs = []
    for pdf in worker.scan(root):
        d, stem = out_dir(pdf)
        md = os.path.join(d, stem + ".md")
        if os.path.exists(md) and d != book.dir:
            refs.append(md)
    refs.sort(key=lambda x: 0 if "srd" in os.path.basename(x).lower() else 1)
    return refs


def page_text(md_pages, p):
    return md_pages.get(p, f'<a id="p{p}"></a>\n')


def prepare(book, p, workroot):
    from .convert import safe_pixmap

    wd = os.path.join(workroot, "p%04d" % p)
    os.makedirs(wd, exist_ok=True)
    with open(os.path.join(book.dir, book.stem + ".md"), encoding="utf-8") as f:
        pages = audit.split_pages(f.read())
    cur = page_text(pages, p).strip()
    pdf_text = book.doc[p - 1].get_text()
    with open(os.path.join(wd, "current.md"), "w", encoding="utf-8") as f:
        f.write(cur + "\n")
    with open(os.path.join(wd, "textlayer.txt"), "w", encoding="utf-8") as f:
        f.write(pdf_text)
    safe_pixmap(book.doc[p - 1], 110).save(os.path.join(wd, "page.png"))
    with open(os.path.join(wd, "prompt.md"), "w", encoding="utf-8") as f:
        refs = "\n".join("- " + r for r in reference_books(book)) or "- (none)"
        f.write(PROMPT.format(page=p, refs=refs, book_md=os.path.join(book.dir, book.stem + ".md")))
    return wd, cur, pdf_text


def run_engine(engine, wd, timeout=900):
    prompt = os.path.join(wd, "prompt.md")
    if engine == "grok":
        # read-only sandbox: the agent may read and search anything but cannot write
        cmd = ["grok", "--no-auto-update", "--prompt-file", prompt, "--cwd", wd, "--output-format", "json",
               "--always-approve", "--sandbox", "read-only", "--disable-web-search",
               "--max-turns", os.environ.get("PDF2MD_GROK_MAX_TURNS", "40"),
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
        r = subprocess.run(shlex.split(engine[4:]) + [prompt], capture_output=True, text=True,
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


def judge(before, after):
    """Accept when recall does not drop and unknown words do not rise."""
    reasons = []
    if before["recall"] is not None and after["recall"] is not None and after["recall"] < before["recall"]:
        reasons.append(f"recall {before['recall']} -> {after['recall']}")
    if after["unknown_1k"] > before["unknown_1k"]:
        reasons.append(f"unknown/1k {before['unknown_1k']} -> {after['unknown_1k']}")
    if after["words"] < 0.5 * before["words"]:
        reasons.append(f"words {before['words']} -> {after['words']}")
    return not reasons, reasons


def review_page(book, p, engine="grok", workroot=None):
    workroot = workroot or os.path.join(book.dir, "review")
    t0 = time.time()
    wd, cur, pdf_text = prepare(book, p, workroot)
    info = book.m["pages"].get(str(p), {})
    before = audit.score_page(pdf_text, cur, info)
    rec = {"page": p, "engine": engine, "before": before, "time": None}
    try:
        new = clean_output(run_engine(engine, wd), p)
    except Exception as err:  # engine failure: logged, page unchanged
        rec.update(status="error", reasons=[str(err)[:300]], time=round(time.time() - t0, 1))
        _log(book, rec)
        return rec
    after = audit.score_page(pdf_text, new, dict(info, quality="ok"))
    ok, reasons = judge(before, after)
    rec.update(after=after, reasons=reasons, time=round(time.time() - t0, 1))
    if ok:
        fixes.write_fix(audit.fix_path(book, p), book.m["sha256"], VERSION, engine, new)
        rec["status"] = "accepted"
    else:
        rej = os.path.join(book.dir, "fixes", "rejected", "p%04d.md" % p)
        fixes.write_fix(rej, book.m["sha256"], VERSION, engine, new)
        rec["status"] = "rejected"
    _log(book, rec)
    return rec


def _log(book, rec):
    os.makedirs(os.path.join(book.dir, "fixes"), exist_ok=True)
    with open(os.path.join(book.dir, "fixes", "review.log.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps(rec) + "\n")
