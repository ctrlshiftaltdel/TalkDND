"""Audit, fixes layer and review gate with synthetic PDFs and a fake engine."""

import json
import os
import sys

import pymupdf

from pdf2md import audit, fixes, review
from pdf2md.convert import Book

TEXT = ("the hero and the goblin warrior attack the dragon in the treasure vault with sword and shield "
        "for some common treasure ") * 3


def make_book(tmp_path):
    pdf = tmp_path / "Rules" / "Test Book.pdf"
    pdf.parent.mkdir(parents=True)
    doc = pymupdf.open()
    for i in range(3):
        page = doc.new_page()
        page.insert_textbox(pymupdf.Rect(72, 72, 540, 700), f"Chapter {i + 1}\n" + TEXT, fontsize=11)
    doc.save(pdf)
    b = Book(str(pdf), chunk_size=10)
    b.convert_chunk(0, 3)
    b.assemble()
    return b


def test_score_page_detects_missing_text():
    good = audit.score_page(TEXT, TEXT)
    bad = audit.score_page(TEXT, "the hero ofyour lnitiative")
    assert good["recall"] == 1.0 and bad["recall"] < 0.2
    assert bad["badness"] > good["badness"]
    marked = audit.score_page(TEXT, TEXT + " sword <!-- inferred: SRD --> <!-- check PDF -->")
    assert marked["inferred"] == 1 and marked["check_pdf"] == 1


def test_fix_layer_survives_reassembly_and_hash_mismatch(tmp_path):
    b = make_book(tmp_path)
    p = audit.fix_path(b, 2)
    fixes.write_fix(p, b.m["sha256"], "test", "fake", '<a id="p2"></a>\nFIXED PAGE TWO')
    md = b.assemble()
    pages = audit.split_pages(md)
    assert "FIXED PAGE TWO" in pages[2] and "FIXED" not in pages[1] + pages[3]
    fixes.write_fix(p, "0" * 64, "test", "fake", '<a id="p2"></a>\nOTHER PDF')
    assert "OTHER PDF" not in b.assemble()


def test_audit_writes_ranked_json(tmp_path):
    b = make_book(tmp_path)
    rows = audit.audit_book(audit.BookView(b.pdf))
    data = json.load(open(os.path.join(b.dir, "audit.json")))
    assert len(rows) == 3 and data["summary"]["pages"] == 3
    assert [r["badness"] for r in rows] == sorted((r["badness"] for r in rows), reverse=True)


def _engine(tmp_path, text):
    script = tmp_path / "eng.py"
    script.write_text(f"import sys\nprint({text!r})\n")
    return f'cmd:"{sys.executable}" "{script}"'


def test_review_accepts_non_regressing_fix(tmp_path):
    b = make_book(tmp_path)
    rec = review.review_page(audit.BookView(b.pdf), 1, _engine(tmp_path, "Chapter 1\n" + TEXT))
    assert rec["status"] == "accepted"
    assert os.path.exists(audit.fix_path(b, 1))
    prompt = open(os.path.join(b.dir, "review", "p0001", "prompt.md")).read()
    assert "inferred:" in prompt and "check PDF" in prompt


def test_review_rejects_regression(tmp_path):
    b = make_book(tmp_path)
    rec = review.review_page(audit.BookView(b.pdf), 1, _engine(tmp_path, "the hero"))
    assert rec["status"] == "rejected" and rec["reasons"]
    assert not os.path.exists(audit.fix_path(b, 1))
    log = open(os.path.join(b.dir, "fixes", "review.log.jsonl")).read()
    assert '"rejected"' in log


def test_hyphenated_text_layer_does_not_penalise_joined_words():
    from pdf2md import repair
    repair.set_words("the hero is restored to life materializing in vault characters find some common "
                     "treasure with goblin warrior and dragon for".split())
    ref = ("the hero is restored to life, materi-\nalizing in the vault. Char-\nacters find some common treasure "
           "with the goblin warrior and the dragon in the vault for the hero ") * 2
    md = ref.replace("materi-\nalizing", "materializing").replace("Char-\nacters", "Characters")
    assert audit.score_page(ref, md)["recall"] == 1.0

def test_judge_soft_accepts_small_recall_drop_on_statblock():
    before = {"recall": 0.99, "unknown_1k": 20.0, "words": 500, "badness": 30.0, "flags": ["statblock"]}
    after = {"recall": 0.96, "unknown_1k": 8.0, "words": 480, "badness": 12.0, "flags": ["statblock"]}
    ok, reasons = review.judge(before, after, "## Goblin\n**Armor Class** 15\n**Hit Points** 7\n| STR |")
    assert ok and not reasons


def test_judge_rejects_large_recall_drop_or_deletion():
    before = {"recall": 0.97, "unknown_1k": 200.0, "words": 400, "badness": 80.0, "flags": []}
    deleted = {"recall": 0.86, "unknown_1k": 0.0, "words": 250, "badness": 20.0, "flags": []}
    ok, reasons = review.judge(before, deleted, "short page")
    assert not ok and any("recall" in r for r in reasons)
    hard = {"recall": 0.97, "unknown_1k": 0.0, "words": 100, "badness": 5.0, "flags": ["statblock"]}
    ok2, reasons2 = review.judge(before, hard, "**Armor Class** 10")
    assert not ok2 and any("words" in r for r in reasons2)


def test_lookup_snippets_injects_matching_section(tmp_path):
    ref = tmp_path / "SRD_fake.md"
    ref.write_text("# SRD\n\n## Fire Bolt\nA simple cantrip that deals fire damage to one target.\n\n## Mage Hand\nA spectral hand appears.\n")
    snip = review.lookup_snippets(["Fire Bolt", "Missing Name"], [str(ref)], max_chars=2000)
    assert "Fire Bolt" in snip and "cantrip" in snip
    assert "Missing Name" not in snip
    prompt_bits = review.PROMPT.format(page=1, refs="- x", book_md="y", snippets=snip)
    assert "Local reference snippets" in review.PROMPT and "cantrip" in prompt_bits


def test_rejudge_accepts_soft_improvement(tmp_path):
    b = make_book(tmp_path)
    # Corrupt page 1 in the assembled book so "before" scores poorly on unknown words,
    # then place a cleaned rejected fix that only dips recall slightly and has structure markers.
    md_path = os.path.join(b.dir, b.stem + ".md")
    with open(md_path, encoding="utf-8") as f:
        md = f.read()
    pages = audit.split_pages(md)
    noisy = pages[1].replace("goblin", "g0blin").replace("dragon", "draggon")
    # Keep enough of the text layer words so recall stays measurable; inject junk unknowns.
    noisy += "\nzzxqword qqqxyz foobarz\n"
    new_md = md.replace(pages[1], noisy)
    open(md_path, "w", encoding="utf-8").write(new_md)
    b.m["pages"]["1"] = {**b.m["pages"].get("1", {}), "flags": ["statblock"]}
    open(os.path.join(b.dir, "manifest.json"), "w").write(json.dumps(b.m))
    # Soft-improved fix: almost full TEXT plus structure markers (slightly shorter is ok).
    fixed = ('<a id="p1"></a>\n## Test Creature\n**Armor Class** 12\n**Hit Points** 9\n'
             '| STR | DEX |\n| --- | --- |\n| 10 | 10 |\n' + TEXT)
    rej = os.path.join(b.dir, "fixes", "rejected", "p0001.md")
    fixes.write_fix(rej, b.m["sha256"], "test", "fake", fixed)
    bv = audit.BookView(b.pdf)
    # Reload manifest flags
    bv.m["pages"]["1"] = b.m["pages"]["1"]
    rec = review.rejudge_page(bv, 1)
    assert rec["status"] == "accepted", rec
    assert os.path.exists(audit.fix_path(b, 1))
    assert not os.path.exists(rej)


def test_max_turns_retries_with_short_prompt(tmp_path, monkeypatch):
    b = make_book(tmp_path)
    calls = []

    def fake_run(engine, wd, timeout=900, max_turns=None):
        prompt = open(os.path.join(wd, "prompt.md"), encoding="utf-8").read()
        calls.append({"retry_prompt": "RETRY" in prompt, "max_turns": max_turns})
        if len(calls) == 1:
            raise RuntimeError("grok exit 1: Error: max turns reached\n")
        return "Chapter 1\n" + TEXT

    monkeypatch.setattr(review, "run_engine", fake_run)
    rec = review.review_page(audit.BookView(b.pdf), 1, "grok")
    assert rec["status"] == "accepted"
    assert rec.get("retried") == "max-turns"
    assert len(calls) == 2 and calls[1]["retry_prompt"]
