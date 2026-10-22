"""Worker status and atomic writes, on synthetic roots only."""

import json
import os

import pytest

from pdf2md import __main__ as cli
from pdf2md import convert, worker


def _book(root, manifest):
    """One empty PDF under Rules/2014 with an optional manifest."""
    (root / "Rules" / "2014").mkdir(parents=True, exist_ok=True)
    (root / "Rules" / "2014" / "Test Book.pdf").write_bytes(b"")
    d = root / "Rules" / "2014" / ".md" / "Test Book"
    d.mkdir(parents=True, exist_ok=True)
    if manifest is not None:
        (d / "manifest.json").write_text(manifest, encoding="utf-8")
    return d


@pytest.mark.parametrize("data", ["[]", "null", '"oops"', '{"page_count": 2, "pages": []}'])
def test_status_reports_wrong_shape_manifest_and_exits_zero(tmp_path, capsys, data):
    root = tmp_path / "LetsPlay"
    _book(root, data)
    rows = worker.status(str(root))
    assert len(rows) == 1
    assert "invalid manifest" in rows[0]["error"]
    assert cli.main(["--root", str(root), "status"]) == 0
    assert "manifest error" in capsys.readouterr().out
    status = json.loads((root / ".pdf2md" / "status.json").read_text(encoding="utf-8"))
    assert status[0]["error"] == rows[0]["error"]
    assert not list((root / ".pdf2md").glob("*.tmp"))


def test_status_accepts_a_good_manifest(tmp_path):
    root = tmp_path / "LetsPlay"
    _book(
        root,
        json.dumps(
            {"page_count": 2, "pages": {"1": {"quality": "low"}, "2": {"ocr": True}}}
        ),
    )
    row = worker.status(str(root))[0]
    assert "error" not in row
    assert row["pages"] == 2 and row["done"] == 2 and row["low"] == 1 and row["ocr"] == 1


def test_write_atomic_flushes_and_leaves_no_temp(tmp_path, monkeypatch):
    calls = []
    real = os.fsync

    def record(fd):
        calls.append(fd)
        return real(fd)

    monkeypatch.setattr(os, "fsync", record)
    p = tmp_path / "sub" / "manifest.json"
    convert._write_atomic(str(p), '{"ok": true}')
    assert json.loads(p.read_text(encoding="utf-8")) == {"ok": True}
    assert calls, "fsync was not called"
    assert not list(p.parent.glob("*.tmp"))
    convert._write_atomic(str(p), b'{"ok": false}')
    assert json.loads(p.read_text(encoding="utf-8")) == {"ok": False}
