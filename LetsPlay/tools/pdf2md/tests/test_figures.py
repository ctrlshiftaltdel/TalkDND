import os

import numpy as np
import pymupdf
import pytest

from pdf2md import figures
from pdf2md.convert import Book

PARA = "The vault door is guarded by a sleepless golem of iron and brass."


def _scanned(path, art=True, full=False):
    """A 'scanned' page: one page bitmap plus an invisible OCR text layer."""
    src = pymupdf.open()
    pg = src.new_page(width=612, height=792)
    lines = []
    if full:
        rng = np.random.default_rng(1)
        for _ in range(400):
            x, y = rng.uniform(20, 590), rng.uniform(20, 770)
            pg.draw_rect(pymupdf.Rect(x, y, x + 30, y + 30), color=(0.2, 0.3, 0.1), fill=(0.3, 0.4, 0.2))
        lines = [(40, 760, "MAP 1.1: THE VAULT")]
    else:
        y = 60
        for i in range(10):
            lines.append((50, y, f"{PARA} Line {i}."))
            y += 14
        if art:
            pg.draw_oval(pymupdf.Rect(150, 230, 460, 520), color=(0, 0, 0), fill=(0.25, 0.1, 0.1))
            pg.draw_rect(pymupdf.Rect(200, 280, 300, 400), color=(0, 0, 0), fill=(0.15, 0.2, 0.1))
        y = 600
        for i in range(8):
            lines.append((50, y, f"After the picture comes more prose, row {i} of it."))
            y += 14
    for x, y, t in lines:
        pg.insert_text((x, y), t, fontsize=10)
    pix = pg.get_pixmap(dpi=150)
    doc = pymupdf.open()
    out = doc.new_page(width=612, height=792)
    out.insert_image(out.rect, stream=pix.tobytes("png"))
    for x, y, t in lines:
        out.insert_text((x, y), t, fontsize=10, render_mode=3)
    doc.save(path)


def test_crop_found_and_placed(tmp_path):
    pdf = str(tmp_path / "Scan.pdf")
    _scanned(pdf)
    page = pymupdf.open(pdf)[0]
    an = figures.analyse(page)
    assert not figures.full_page_art(page, an[0], an[1])
    rects = figures.find_art(page, an)
    assert len(rects) == 1
    x0, y0, x1, y1 = rects[0]
    assert x0 <= 160 and y0 <= 240 and x1 >= 450 and y1 >= 510
    above, below = figures.anchors(an[1], rects[0])
    assert "Line 9" in above and "row 0" in below




def test_full_page_map(tmp_path):
    pdf = str(tmp_path / "Map.pdf")
    _scanned(pdf, full=True)
    page = pymupdf.open(pdf)[0]
    assert figures.full_page_art(page)


def test_no_art_on_text_page(tmp_path):
    pdf = str(tmp_path / "Text.pdf")
    _scanned(pdf, art=False)
    assert figures.find_art(pymupdf.open(pdf)[0]) == []


def test_place():
    body = "First para line\ncontinues here.\n\nSecond paragraph of the page."
    out, where = figures.place(body, "![x](img/a.webp)", "First para line", "")
    assert where == "after" and out.index("![x]") > out.index("continues here.") < out.index("Second")
    out, where = figures.place(body, "![x](img/a.webp)", "", "Second paragraph of the page.")
    assert where == "before" and out.index("![x]") < out.index("Second")
    out, where = figures.place(body, "![x](img/a.webp)", "", "")
    assert where == "end" and out.endswith("![x](img/a.webp)")


def test_book_image_stage(tmp_path):
    pdf = str(tmp_path / "Scan Book.pdf")
    _scanned(pdf)
    b = Book(pdf, chunk_size=10)
    b.convert_chunk(0, 1)
    md = b.assemble()
    b.close()
    info = b.m["pages"]["1"]
    assert "scan" in info["flags"] and len(info["art"]) == 1
    img = os.path.join(b.dir, info["art"][0]["img"])
    from PIL import Image

    with Image.open(img) as im:
        assert im.format == "WEBP" and 900 <= max(im.size) <= 1100
    assert "![Illustration, PDF p.1](img/p0001-a1.webp)" in md
    assert "p0001-a1.webp" in open(os.path.join(b.dir, "index.md"), encoding="utf-8").read()


def test_map_side(tmp_path):
    pdf = str(tmp_path / "Map Book.pdf")
    _scanned(pdf, full=True)
    b = Book(pdf, chunk_size=10)
    rel = b.render_page(0)
    from PIL import Image

    with Image.open(os.path.join(b.dir, rel)) as im:
        assert 1500 <= max(im.size) <= 1700
    b.close()


def test_contact_sheet(tmp_path):
    from PIL import Image

    paths = []
    for i in range(18):
        p = tmp_path / f"p{i:04d}.webp"
        Image.new("RGB", (300, 200), (i * 10, 50, 50)).save(p, "WEBP")
        paths.append(str(p))
    sheets = figures.contact_sheet(paths, [f"p.{i}" for i in range(18)])
    assert len(sheets) == 2
