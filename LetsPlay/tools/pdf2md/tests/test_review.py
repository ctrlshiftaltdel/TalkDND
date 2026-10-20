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
