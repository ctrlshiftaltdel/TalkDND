"""TASK-0011 regressions. Synthetic PDFs and invented words only."""

import json
import os

import pymupdf

from pdf2md import convert, fallback, ocr, repair, worker

LEFT = ("alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike "
        "november oscar papa quebec romeo sierra tango").split()
RIGHT = ("uniform victor whiskey xray yankee zulu amber birch cedar dahlia elm fern "
         "ginger hazel iris jasmine kale lotus maple nutmeg").split()


SYL = ["ka", "lo", "mi", "nu", "pe", "ra", "si", "tu"]
VOCAB = [a + b + c for a in SYL for b in SYL for c in SYL]  # invented, unique words


def col_line(side, i):
    base = (0 if side == "L" else 400) + i * 5
    return " ".join(VOCAB[base:base + 5])


def _two_col_pdf(path):
    doc = pymupdf.open()
    page = doc.new_page()
    for i in range(20):
        page.insert_text((60, 80 + 16 * i), f"{LEFT[i]} {col_line('L', i)}", fontsize=10)
        page.insert_text((330, 80 + 16 * i), f"{RIGHT[i]} {col_line('R', i)}", fontsize=10)
    doc.save(path)


def _book(tmp_path, maker, name="Book.pdf"):
    pdf = tmp_path / name
    maker(str(pdf))
    return convert.Book(str(pdf), chunk_size=10)


def test_poster_size_page_does_not_crash(tmp_path, monkeypatch):
    def poster(path):
        doc = pymupdf.open()
        p = doc.new_page(width=6300, height=9000)
        pix = pymupdf.Pixmap(pymupdf.csRGB, pymupdf.IRect(0, 0, 50, 50), False)
        p.insert_image(p.rect, pixmap=pix)
        p.insert_text((100, 200), "Map", fontsize=40)
        doc.save(path)
    b = _book(tmp_path, poster)
    seen = []
    real = pymupdf.Page.get_pixmap

    def spy(self, *a, **k):
        seen.append(k.get("dpi"))
        return real(self, *a, **k)
    monkeypatch.setattr(pymupdf.Page, "get_pixmap", spy)
    md = b.convert_chunk(0, 1)
    assert "img/p0001.webp" in md and seen and seen[-1] < convert.IMG_DPI


def test_render_failure_is_flagged_not_fatal(tmp_path, monkeypatch):
    b = _book(tmp_path, lambda p: _poster_small(p))

    def boom(self, *a, **k):
        raise RuntimeError("code=5: Overly large image")
    monkeypatch.setattr(pymupdf.Page, "get_pixmap", boom)
    b.convert_chunk(0, 1)
    assert "render-failed" in b.m["pages"]["1"]["flags"] and b.m["pages"]["1"]["quality"] == "low"


def _poster_small(path):
    doc = pymupdf.open()
    p = doc.new_page()
    pix = pymupdf.Pixmap(pymupdf.csRGB, pymupdf.IRect(0, 0, 50, 50), False)
    p.insert_image(p.rect, pixmap=pix)
    doc.save(path)


def test_ocr_word_cleanup():
    repair.set_words(["initiative", "illusion", "your", "you", "that", "iron", "often", "the"])
    out = repair.fix_ocr_words("Roll lnitiative; lllusion of lron. ofyour gear thatyou often Tobias")
    assert out == "Roll Initiative; Illusion of Iron. of your gear that you often Tobias"
    assert repair.fix_ocr_words("Cult<sup>members</sup> x<sup>2</sup>") == "Cult members x<sup>2</sup>"


def test_column_text_reads_left_column_first(tmp_path):
    _two_col_pdf(str(tmp_path / "c.pdf"))
    t = fallback.column_text(pymupdf.open(str(tmp_path / "c.pdf"))[0])
    assert t.index("tango") < t.index("uniform")


def _check(tmp_path, body):
    repair.set_words(LEFT + RIGHT + VOCAB)
    b = _book(tmp_path, _two_col_pdf)
    info = {"chars": 999, "flags": []}
    return b.check_text_page(0, body, {}, info), info


def test_empty_page_falls_back_to_text_layer(tmp_path):
    out, info = _check(tmp_path, "")
    assert info["fallback"] == "empty" and f"tango {col_line('L', 19)}" in out


def test_stutter_falls_back(tmp_path):
    out, info = _check(tmp_path, "alpha bravo charlie delta " * 8)
    assert info["fallback"] == "stutter" and "charlie delta alpha" not in out


def test_interleaved_columns_fall_back(tmp_path):
    body = " ".join(f"{LEFT[i]} {col_line('L', i)} {RIGHT[i]} {col_line('R', i)}" for i in range(20))
    out, info = _check(tmp_path, body)
    assert info["fallback"] == "interleaved"
    assert out.index("tango") < out.index("uniform")


def test_good_page_is_kept(tmp_path):
    body = " ".join(f"{LEFT[i]} {col_line('L', i)}" for i in range(20)) + "\n\n" + \
        " ".join(f"{RIGHT[i]} {col_line('R', i)}" for i in range(20))
    out, info = _check(tmp_path, body)
    assert "fallback" not in info and out == body


def test_garbled_heading_is_reread_with_ocr(tmp_path, monkeypatch):
    def pdf(path):
        doc = pymupdf.open()
        p = doc.new_page()
        p.insert_text((72, 100), "Xqvv Wrrkz", fontsize=20)
        doc.save(path)
    repair.set_words(["goblin", "warrior"])
    b = _book(tmp_path, pdf)

    class Fake:
        def read(self, png):
            return [ocr.OcrLine(0, 0, 10, 10, "Goblin Warrior", 0.99)]
    monkeypatch.setattr(ocr, "get_engine", lambda name="rapid": Fake())
    md, n = fallback.repair_headings("## Xqvv Wrrkz\n\nbody", b.doc[0], "rapid")
    assert n == 1 and md.startswith("## Goblin Warrior")


def test_book_only_stops_after_that_book(letsplay):
    root = str(letsplay)
    assert worker.run(root, book="SRD Test", only=True, nice=False) == 0
    assert (letsplay / "Rules/2024/.md/SRD Test/SRD Test.md").exists()
    assert not (letsplay / "Rules/2024/.md/Core Rules 2024").exists()


def test_tool_upgrade_keeps_old_pages_readable(letsplay, monkeypatch):
    root = str(letsplay)
    worker.run(root, book="SRD Test", only=True, nice=False)
    d = letsplay / "Rules/2024/.md/SRD Test"
    monkeypatch.setattr(convert, "VERSION", "9.9.9")
    b = convert.Book(str(letsplay / "Rules/2024/SRD Test.pdf"))
    assert b.pending() and b.m["stale"]
    md = b.assemble()
    assert "being redone" in md and '<a id="p1"></a>' in md
    worker.run(root, book="SRD Test", only=True, nice=False)
    m = json.loads((d / "manifest.json").read_text(encoding="utf-8"))
    assert m["tool"] == "9.9.9" and not m["stale"] and m["chunks"]
    assert os.path.exists(d / "SRD Test.md")
