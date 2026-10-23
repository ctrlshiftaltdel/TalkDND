"""Intake audit on synthetic PDFs only (text layer, bookmarks, chapter phrase)."""

import json

import pymupdf

from pdf2md import __main__ as cli
from pdf2md import intake, worker

from .conftest import make_image_only_pdf


def _text_pdf(path, lines, toc=None):
    doc = pymupdf.open()
    for first in lines:
        page = doc.new_page()
        page.insert_text((72, 100), first, fontsize=16)
        page.insert_text(
            (72, 130),
            "The hero and the goblin guard the treasure vault and the sword.",
            fontsize=11,
        )
    if toc:
        doc.set_toc(toc)
    doc.save(str(path))
    doc.close()


def _audit(path, chapter_open=None):
    doc = pymupdf.open(str(path))
    try:
        return intake.audit_doc(doc, chapter_open)
    finally:
        doc.close()


def test_audit_reports_text_layer_pages_and_usable_verdict(tmp_path):
    pdf = tmp_path / "Book.pdf"
    _text_pdf(pdf, ["Chapter 1: The Vault", "Chapter 2: The Cellar"])
    a = _audit(pdf)
    assert a["pages"] == 2
    assert len(a["chars_per_page"]) == 2 and min(a["chars_per_page"]) > 40
    assert a["zero_pages"] == 0 and a["garbage_rate"] == 0
    assert a["scanned"] is False and a["scanned_guess"] == "digital"
    assert a["verdict"] == "text layer usable"
    assert a["bookmarks"] == 0 and a["chapters"] == []
    # idempotent
    assert _audit(pdf) == a


def test_scanned_book_needs_ocr(tmp_path):
    pdf = tmp_path / "Scan.pdf"
    make_image_only_pdf(str(pdf))
    a = _audit(pdf)
    assert a["scanned"] is True and a["scanned_guess"] == "scanned"
    assert a["zero_pages"] == a["pages"]
    assert a["verdict"] == "OCR needed" and a["reasons"]


def test_garbage_characters_need_ocr():
    """A synthetic text layer with replacement/control characters (no real book)."""

    class Page:
        def __init__(self, text):
            self.text = text

        def get_text(self, mode):
            return self.text

    class Doc:
        page_count = 1

        def __getitem__(self, i):
            return Page("\ufffd" * 120)

        def get_toc(self, simple=True):
            return []

    a = intake.audit_doc(Doc())
    assert a["garbage_chars"] >= 100
    assert a["garbage_rate"] > 0.02
    assert a["verdict"] == "OCR needed"


def test_bookmarks_give_chapter_starts(tmp_path):
    pdf = tmp_path / "Toc.pdf"
    _text_pdf(
        pdf,
        ["One", "Two", "Three"],
        toc=[[1, "Chapter One", 1], [2, "A Part", 1], [1, "Chapter Two", 3]],
    )
    a = _audit(pdf)
    assert a["bookmarks"] == 3
    assert a["chapters"] == [
        {"page": 1, "title": "Chapter One"},
        {"page": 3, "title": "Chapter Two"},
    ]


def test_recurring_opening_phrase_finds_chapters_without_bookmarks(tmp_path):
    pdf = tmp_path / "Opening.pdf"
    _text_pdf(pdf, ["The Adventure Begins"] * 4)
    a = _audit(pdf, "The Adventure Begins")
    assert a["bookmarks"] == 0
    assert [c["page"] for c in a["chapters"]] == [1, 2, 3, 4]
    assert a["chapter_open"] == "The Adventure Begins"
    # Case/punctuation-insensitive phrase matching.
    assert len(_audit(pdf, "the adventure, begins")["chapters"]) == 4


def test_chapter_starts_are_stored_in_index_metadata_and_status(letsplay):
    pdf = letsplay / "Adventures/Opening Book.pdf"
    _text_pdf(str(pdf), ["The Adventure Begins"] * 3)
    root = str(letsplay)
    assert worker.run(root, book="Opening Book", nice=False,
                      chapter_open="The Adventure Begins") == 0
    d = letsplay / "Adventures/.md/Opening Book"
    manifest = json.loads((d / "manifest.json").read_text(encoding="utf-8"))
    index = json.loads((d / "index.json").read_text(encoding="utf-8"))
    assert manifest["intake"]["chapter_open"] == "The Adventure Begins"
    assert [c["page"] for c in index["chapters"]] == [1, 2, 3]
    assert index["intake"]["verdict"] == "text layer usable"
    index_md = (d / "index.md").read_text(encoding="utf-8")
    assert "## Intake audit" in index_md and "## Chapter starts" in index_md
    assert "The Adventure Begins" in index_md
    # Re-running without the option keeps the stored phrase and chapters.
    assert worker.run(root, book="Opening Book", nice=False) == 0
    manifest2 = json.loads((d / "manifest.json").read_text(encoding="utf-8"))
    assert manifest2["intake"]["chapter_open"] == "The Adventure Begins"
    assert [c["page"] for c in manifest2["intake"]["chapters"]] == [1, 2, 3]
    # Status stays readable and mentions the verdict.
    row = next(r for r in worker.status(root) if "Opening Book" in r["pdf"])
    assert row["intake"]["verdict"] == "text layer usable" and row["fallback"] == 0
    from pdf2md import __main__ as main

    assert main.main(["--root", root, "status"]) == 0


def test_audit_command_writes_intake_and_prints_verdict(letsplay, capsys):
    pdf = letsplay / "Rules/2024/Intake Book.pdf"
    _text_pdf(str(pdf), ["Chapter 1: Vault", "Chapter 2: Cellar"])
    root = str(letsplay)
    assert worker.run(root, book="Intake Book", nice=False) == 0
    assert cli.main(["--root", root, "audit", "--book", "Intake Book"]) == 0
    out = capsys.readouterr().out
    assert "page fallbacks" in out and "intake: text layer usable" in out
    data = json.loads(
        (letsplay / "Rules/2024/.md/Intake Book/audit.json").read_text(encoding="utf-8")
    )
    assert data["intake"]["verdict"] == "text layer usable"
    assert data["summary"]["verdict"] == "text layer usable"
