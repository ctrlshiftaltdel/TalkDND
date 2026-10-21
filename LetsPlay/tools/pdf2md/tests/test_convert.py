import json
import os

import pymupdf
import pytest

from pdf2md import __main__ as cli
from pdf2md import convert, worker

from .conftest import make_map_pdf


def test_priority_current_adventure_then_srd_then_ruleset(letsplay):
    order = [os.path.basename(p) for p in worker.prioritise(str(letsplay), worker.scan(str(letsplay)))]
    assert order[:3] == ["My Heist.pdf", "SRD Test.pdf", "Core Rules 2024.pdf"]
    assert order[-1] == "Old Rules.pdf"


def test_output_named_after_pdf_and_resumable(letsplay):
    pdf = letsplay / "Rules/2024/Core Rules 2024.pdf"
    root = str(letsplay)
    assert worker.run(root, budget_pages=4, chunk_size=5, book="Core Rules", nice=False) == 0
    d = letsplay / "Rules/2024/.md/Core Rules 2024"
    m = json.loads((d / "manifest.json").read_text(encoding="utf-8"))
    assert list(m["chunks"]) == ["p0001-0005"]
    book = (d / "Core Rules 2024.pages.md").read_text(encoding="utf-8")
    assert '<a id="p1"></a>' in book and "not converted yet" in book
    assert "The PDF is authoritative" in book
    reader = (d / "Core Rules 2024.md").read_text(encoding="utf-8")
    assert "<a id=" not in reader and "<!-- PDF p." not in reader
    assert "not converted yet" in reader and "Core Rules 2024.pages.md" in reader
    # resume: finishes the rest without redoing chunk 1
    before = m["chunks"]["p0001-0005"]
    worker.run(root, book="Core Rules", chunk_size=5, nice=False)
    m = json.loads((d / "manifest.json").read_text(encoding="utf-8"))
    assert list(m["chunks"]) == ["p0001-0005", "p0006-0010", "p0011-0012"]
    assert m["chunks"]["p0001-0005"] == before
    assert m["pages"]["3"]["quality"] == "low" and "statblock" in m["pages"]["3"]["flags"]
    idx = json.loads((d / "index.json").read_text(encoding="utf-8"))
    assert 3 in idx["statblocks"] and idx["done_pages"] == 12
    assert any(h["page"] == 1 for h in idx["headings"])
    assert cli.verify(root, "Core Rules") == 0
    assert (d / "index.md").exists()
    # PDF changes -> hash differs -> restart
    doc = pymupdf.open()
    doc.new_page().insert_text((72, 72), "new edition", fontsize=12)
    doc.save(str(pdf))
    b = convert.Book(str(pdf), chunk_size=5)
    assert b.m["chunks"] == {} and b.m["page_count"] == 1


def test_lock_blocks_second_worker(letsplay):
    lock = worker.Lock(str(letsplay))
    assert lock.acquire()
    try:
        assert worker.run(str(letsplay), budget_pages=1, nice=False) == 3
    finally:
        lock.release()


def test_budget_zero_does_nothing(letsplay):
    worker.run(str(letsplay), budget_seconds=0, nice=False)
    assert not (letsplay / "Adventures/my-heist/.md/My Heist/pages").exists()


def test_figure_page_linked_not_embedded(letsplay):
    make_map_pdf(str(letsplay / "Adventures/Maps.pdf"))
    worker.run(str(letsplay), book="Maps", budget_pages=1, nice=False)
    d = letsplay / "Adventures/.md/Maps"
    md = (d / "Maps.md").read_text(encoding="utf-8")
    assert "](img/p0001.webp)" in md and "base64" not in md
    assert (d / "img/p0001.webp").exists()


def test_corrupt_pdf_is_skipped(letsplay):
    (letsplay / "Adventures/broken.pdf").write_bytes(b"not a pdf")
    assert worker.run(str(letsplay), budget_pages=1, nice=False) == 0


def test_status_and_scan_cli(letsplay, capsys):
    assert cli.main(["scan"]) == 0
    assert "My Heist.pdf" in capsys.readouterr().out.splitlines()[0]
    assert cli.main(["status"]) == 0


def _content_lines(md):
    import re

    md = re.sub(r'^<a id="p\d+"></a>$|^<!-- PDF p\.\d+ -->$|^> (Each page starts|Reader edition).*$', "", md, flags=re.M)
    return [ln for ln in md.split("\n") if ln.strip()]


def test_reader_and_pages_editions_are_equal_without_anchors(letsplay):
    worker.run(str(letsplay), book="Core Rules", chunk_size=5, nice=False)
    d = letsplay / "Rules/2024/.md/Core Rules 2024"
    pages = (d / "Core Rules 2024.pages.md").read_text(encoding="utf-8")
    reader = (d / "Core Rules 2024.md").read_text(encoding="utf-8")
    import re

    assert len(re.findall(r'<a id="p\d+"></a>', pages)) == 12 and "<a id=" not in reader
    assert _content_lines(pages) == _content_lines(reader)
    assert reader == convert.to_reader(pages, "Core Rules 2024")
    idx = (d / "index.md").read_text(encoding="utf-8")
    assert "Core Rules 2024.pages.md#p1" in idx and "[Core Rules 2024.md]" in idx
    assert cli.verify(str(letsplay), "Core Rules") == 0
    (d / "Core Rules 2024.md").write_text(reader + "drift\n", encoding="utf-8")
    assert cli.verify(str(letsplay), "Core Rules") == 1


def test_upgrade_from_031_keeps_chunks_and_backs_up_fixes(letsplay, monkeypatch):
    from pdf2md import fixes

    monkeypatch.setattr(convert, "VERSION", "0.4.1")  # an assemble-only upgrade

    worker.run(str(letsplay), book="Core Rules", chunk_size=5, nice=False)
    d = letsplay / "Rules/2024/.md/Core Rules 2024"
    m = json.loads((d / "manifest.json").read_text(encoding="utf-8"))
    fixes.write_fix(str(d / "fixes/p0002.md"), m["sha256"], "0.3.1", "fake", '<a id="p2"></a>\nFixed page two.')
    m["tool"] = "0.3.1"
    (d / "manifest.json").write_text(json.dumps(m), encoding="utf-8")
    b = convert.Book(str(letsplay / "Rules/2024/Core Rules 2024.pdf"), chunk_size=5)
    assert len(b.m["chunks"]) == 3 and not b.m.get("stale"), "0.4.1 redoes no chunk"
    b.close()
    # a real reconversion (older tool) backs up fixes first
    m = json.loads((d / "manifest.json").read_text(encoding="utf-8"))
    m["tool"] = "0.1.0"
    (d / "manifest.json").write_text(json.dumps(m), encoding="utf-8")
    convert.Book(str(letsplay / "Rules/2024/Core Rules 2024.pdf"), chunk_size=5).close()
    backups = list((d / "backups").glob("*-reconvert/fixes/p0002.md"))
    assert backups and "Fixed page two." in backups[0].read_text(encoding="utf-8")
    assert (d / "fixes/p0002.md").exists()
