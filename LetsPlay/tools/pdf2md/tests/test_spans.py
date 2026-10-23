"""A word split across PDF text spans must be joined before word counting."""

import pymupdf

from pdf2md import fallback, figures, headings, textutil


def _split_word_page(fontsize=11):
    doc = pymupdf.open()
    page = doc.new_page()
    # Different fonts keep the two halves as separate spans on one line.
    page.insert_text((72, 100), "exam", fontsize=fontsize, fontname="helv")
    page.insert_text((72 + fontsize * 2.3, 100), "ple", fontsize=fontsize, fontname="tiro")
    page.insert_text((72, 130), "A normal line with words.", fontsize=11, fontname="helv")
    return doc, page


def _lines(page):
    return [ln for b in page.get_text("dict")["blocks"] if b.get("type") == 0
            for ln in b["lines"]]


def test_split_word_is_joined_before_tokenization():
    doc, page = _split_word_page()
    split = next(ln for ln in _lines(page) if len(ln["spans"]) == 2)
    assert [s["text"] for s in split["spans"]] == ["exam", "ple"]
    assert textutil.line_text(split) == "example"
    # fallback extraction joins spans, so "example" is one word, not two.
    assert any(t == "example" for _, t, _ in fallback._lines(page))
    words = fallback.words(fallback.column_text(page))
    assert "example" in words
    assert "exam" not in words and "ple" not in words
    # figure anchoring sees the same joined line.
    assert any(t == "example" for _, t in figures.text_lines(page))
    doc.close()


def test_heading_map_joins_split_spans():
    doc, page = _split_word_page(fontsize=22)
    try:
        hmap = headings.build_heading_map(doc)
        assert "example" in hmap, hmap
    finally:
        doc.close()
