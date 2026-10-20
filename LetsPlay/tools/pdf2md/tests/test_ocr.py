import json

import pytest

from pdf2md import ocr, worker

from .conftest import make_image_only_pdf


def test_lines_to_markdown_orders_two_columns():
    L = ocr.OcrLine
    lines = [L(510, 10, 900, 30, "right one", .9), L(10, 10, 400, 30, "left one", .9),
             L(10, 40, 400, 60, "left two", .9), L(510, 40, 900, 60, "right two", .9),
             L(10, 70, 400, 90, "left three", .9), L(510, 70, 900, 90, "right three", .9)]
    text, conf = ocr.lines_to_markdown(lines, 1000)
    assert text.index("left three") < text.index("right one")
    assert conf == pytest.approx(0.9)


def test_unknown_engine_rejected():
    with pytest.raises(ValueError):
        ocr.get_engine("tesseract")


def test_image_only_page_is_ocred_with_rapid(letsplay):
    make_image_only_pdf(str(letsplay / "Adventures/Scanned.pdf"))
    assert worker.run(str(letsplay), book="Scanned", budget_pages=1, nice=False) == 0
    d = letsplay / "Adventures/.md/Scanned"
    m = json.loads((d / "manifest.json").read_text(encoding="utf-8"))
    page = m["pages"]["1"]
    assert page["ocr"] == "rapid" and page["ocr_conf"] > 0.5
    md = (d / "Scanned.md").read_text(encoding="utf-8")
    assert "Goblin" in md and "treasure vault" in md.lower()
    assert "statblock" in page["flags"] and page["quality"] == "low"
