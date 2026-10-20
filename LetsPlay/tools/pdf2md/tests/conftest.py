"""Synthetic PDFs only. No copyrighted material is used in tests."""

import os

import pymupdf
import pytest

from pdf2md import repair

WORDS = """the a and of to in is for with common armor goblin warrior dragon chapter
treasure vault spell attack damage hit points speed class some what component
wisdom charisma hero sword shield""".split()


@pytest.fixture(autouse=True)
def small_words():
    repair.set_words(WORDS)
    yield
    repair.set_words([])


def text_page(doc, lines, two_col=False):
    page = doc.new_page()
    y = 72
    for ln in lines:
        page.insert_text((72, y), ln, fontsize=11)
        if two_col:
            page.insert_text((320, y), ln + " (right)", fontsize=11)
        y += 16
    return page


def make_text_pdf(path, pages=12):
    doc = pymupdf.open()
    for i in range(pages):
        page = doc.new_page()
        page.insert_text((72, 260), f"Chapter {i + 1}: The Vault", fontsize=22)
        page.insert_text((72, 300), f"Page {i + 1} text about the treasure vault and the hero.", fontsize=11)
        if i == 2:
            page.insert_text((72, 340), "Goblin Warrior", fontsize=14)
            page.insert_text((72, 360), "Armor Class 15  Hit Points 7 (2d6)", fontsize=11)
            page.insert_text((72, 380), "STR 8 DEX 14 CON 10", fontsize=11)
    doc.save(path)


def make_image_only_pdf(path):
    """Render text to a bitmap and build a PDF that has no text layer."""
    src = pymupdf.open()
    page = src.new_page()
    page.insert_text((72, 100), "Goblin Warrior", fontsize=24)
    page.insert_text((72, 150), "Armor Class 15 Hit Points 7", fontsize=16)
    page.insert_text((72, 190), "The treasure vault is under the city.", fontsize=16)
    pix = page.get_pixmap(dpi=200)
    out = pymupdf.open()
    p = out.new_page(width=page.rect.width, height=page.rect.height)
    p.insert_image(p.rect, stream=pix.tobytes("png"))
    out.save(path)


def make_map_pdf(path):
    out = pymupdf.open()
    p = out.new_page()
    pix = pymupdf.Pixmap(pymupdf.csRGB, pymupdf.IRect(0, 0, 400, 400), False)
    pix.set_rect(pix.irect, (40, 120, 40))
    p.insert_image(p.rect, pixmap=pix)
    p.insert_text((80, 80), "Map 1", fontsize=10)
    out.save(path)


@pytest.fixture
def letsplay(tmp_path, monkeypatch):
    root = tmp_path / "LetsPlay"
    for d in ("Rules/2014", "Rules/2024", "Adventures/my-heist/combat", "Saves/2026-01-01-01/combat"):
        (root / d).mkdir(parents=True)
    (root / "Saves/2026-01-01-01/combat/campaign.yaml").write_text(
        'ruleset: "2024"\nadventures: [../../../Adventures/my-heist/combat]\n', encoding="utf-8")
    make_text_pdf(str(root / "Rules/2024/Core Rules 2024.pdf"), pages=12)
    make_text_pdf(str(root / "Rules/2024/SRD Test.pdf"), pages=3)
    make_text_pdf(str(root / "Adventures/my-heist/My Heist.pdf"), pages=4)
    make_text_pdf(str(root / "Rules/2014/Old Rules.pdf"), pages=2)
    monkeypatch.setenv("PDF2MD_ROOT", str(root))
    monkeypatch.setenv("PDF2MD_WORDS", str(tmp_path / "none.txt"))
    return root
