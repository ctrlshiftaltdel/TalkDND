"""Manual page corrections made from the play table (synthetic PDFs only)."""

import json

from pdf2md import __main__ as cli, worker


def _convert(letsplay):
    worker.run(str(letsplay), book="My Heist", chunk_size=10, nice=False)


def test_fix_writes_header_and_rebuilds(letsplay, tmp_path):
    _convert(letsplay)
    src = tmp_path / "fix.md"
    src.write_text('<a id="p2"></a>\n<!-- PDF p.2 -->\nFIXED PAGE TWO\n', encoding="utf-8")
    assert cli.main(["fix", "--book", "My Heist", "--page", "2",
                     "--from", str(src), "--reason", "table test"]) == 0
    d = letsplay / "Adventures/my-heist/index"
    fix = (d / "fixes/p0002.md").read_text(encoding="utf-8")
    assert fix.startswith("<!-- pdf2md-fix sha256=")
    assert "engine=manual" in fix.splitlines()[0]
    pages = (d / "My Heist.pages.md").read_text(encoding="utf-8")
    assert pages.count("FIXED PAGE TWO") == 1
    assert pages.count('<a id="p2"></a>') == 1
    log = (d / "fixes/manual.log.jsonl").read_text(encoding="utf-8")
    assert json.loads(log.strip())["reason"] == "table test"


def test_fix_rejects_bad_page_and_empty_input(letsplay, tmp_path):
    _convert(letsplay)
    missing = tmp_path / "missing.md"
    assert cli.main(["fix", "--book", "My Heist", "--page", "99", "--from", str(missing)]) != 0
    empty = tmp_path / "empty.md"
    empty.write_text("  \n", encoding="utf-8")
    assert cli.main(["fix", "--book", "My Heist", "--page", "1", "--from", str(empty)]) != 0
    d = letsplay / "Adventures/my-heist/index"
    assert not (d / "fixes/p0001.md").exists()
    assert not (d / "fixes/p0099.md").exists()


def test_fix_backs_up_previous_and_honours_no_apply(letsplay, tmp_path):
    _convert(letsplay)
    first = tmp_path / "first.md"
    first.write_text("FIRST FIX\n", encoding="utf-8")
    second = tmp_path / "second.md"
    second.write_text("SECOND FIX\n", encoding="utf-8")
    assert cli.main(["fix", "--book", "My Heist", "--page", "2", "--from", str(first)]) == 0
    assert cli.main(["fix", "--book", "My Heist", "--page", "2",
                     "--from", str(second), "--no-apply"]) == 0
    d = letsplay / "Adventures/my-heist/index"
    backups = sorted((d / "fixes/backups").glob("p0002-*.md"))
    assert backups and "FIRST FIX" in backups[0].read_text(encoding="utf-8")
    pages = (d / "My Heist.pages.md").read_text(encoding="utf-8")
    assert "SECOND FIX" not in pages
