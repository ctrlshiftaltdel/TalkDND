"""Worker status and atomic writes, on synthetic roots only."""

import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

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


def test_current_save_ignores_backup_and_hidden_directories(tmp_path):
    saves = tmp_path / "Saves"
    for name in ["2030-01-01-01", "2030-01-02-01", "_backups", ".cache"]:
        (saves / name).mkdir(parents=True, exist_ok=True)
    assert worker.current_save(str(tmp_path)) == str(saves / "2030-01-02-01")
    only_internal = tmp_path / "only-internal"
    for name in ["_backups", ".cache", "not-a-slot"]:
        (only_internal / "Saves" / name).mkdir(parents=True, exist_ok=True)
    assert worker.current_save(str(only_internal)) is None


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


def test_status_counts_page_fallbacks(tmp_path):
    root = tmp_path / "LetsPlay"
    _book(
        root,
        json.dumps(
            {
                "page_count": 3,
                "pages": {
                    "1": {"flags": ["cropbox-clamped"]},
                    "2": {"flags": ["page-error"]},
                    "3": {"flags": ["render-failed"]},
                },
            }
        ),
    )
    row = worker.status(str(root))[0]
    assert row["fallback"] == 3


def _lock_root(tmp_path):
    root = tmp_path / "LetsPlay"
    root.mkdir()
    return str(root)


def test_lock_exactly_one_winner_across_threads(tmp_path):
    root = _lock_root(tmp_path)
    barrier = threading.Barrier(2)
    results = []

    def attempt():
        barrier.wait()
        lock = worker.Lock(root)
        results.append((lock, lock.acquire()))

    threads = [threading.Thread(target=attempt) for _ in range(2)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert sum(ok for _, ok in results) == 1
    for lock, ok in results:
        if ok:
            lock.release()
    # Release allows the next owner.
    lock = worker.Lock(root)
    assert lock.acquire()
    lock.release()
    assert not (tmp_path / "LetsPlay/.pdf2md/worker.lock").exists()


def test_a_live_lock_is_not_stolen_or_removed(tmp_path):
    root = _lock_root(tmp_path)
    first = worker.Lock(root)
    assert first.acquire()
    path = Path(first.path)
    owner = path.read_text(encoding="utf-8")
    try:
        second = worker.Lock(root)
        assert not second.acquire()
        assert path.exists() and path.read_text(encoding="utf-8") == owner
        # A live owner without our OS lock (an older worker) is also respected.
        assert not worker.Lock(root).acquire()
    finally:
        first.release()
    assert not path.exists()


def test_a_dead_owner_lock_is_replaced(tmp_path):
    root = _lock_root(tmp_path)
    state = Path(root) / ".pdf2md"
    state.mkdir()
    path = state / "worker.lock"
    old = time.time() - worker.LOCK_STALE_SECONDS - 5
    path.write_text(
        json.dumps({"pid": 999999, "started": old, "updated": old}), encoding="utf-8"
    )
    os.utime(path, (old, old))
    lock = worker.Lock(root)
    assert lock.acquire()
    data = json.loads(path.read_text(encoding="utf-8"))
    assert data["pid"] == os.getpid() and data["updated"] > old
    lock.release()


def test_an_old_lock_is_replaced_even_if_the_pid_is_live(tmp_path):
    root = _lock_root(tmp_path)
    state = Path(root) / ".pdf2md"
    state.mkdir()
    path = state / "worker.lock"
    old = time.time() - worker.LOCK_STALE_SECONDS - 5
    path.write_text(
        json.dumps({"pid": os.getpid(), "started": old, "updated": old}),
        encoding="utf-8",
    )
    os.utime(path, (old, old))
    lock = worker.Lock(root)
    assert lock.acquire()
    lock.release()


def test_a_live_legacy_lock_file_is_not_stolen(tmp_path):
    root = _lock_root(tmp_path)
    state = Path(root) / ".pdf2md"
    state.mkdir()
    path = state / "worker.lock"
    now = time.time()
    path.write_text(
        json.dumps({"pid": os.getpid(), "started": now, "updated": now}),
        encoding="utf-8",
    )
    assert not worker.Lock(root).acquire()
    assert path.exists()
    path.unlink()


def test_lock_is_visible_to_another_process(tmp_path):
    root = _lock_root(tmp_path)
    package = Path(__file__).resolve().parents[1]
    code = (
        "import sys\n"
        "from pdf2md.worker import Lock\n"
        "print('busy' if not Lock(sys.argv[1]).acquire() else 'free')\n"
    )
    lock = worker.Lock(root)
    assert lock.acquire()
    try:
        busy = subprocess.run(
            [sys.executable, "-c", code, root],
            cwd=str(package),
            capture_output=True,
            text=True,
            timeout=60,
        )
        assert busy.returncode == 0 and busy.stdout.strip() == "busy"
    finally:
        lock.release()
    free = subprocess.run(
        [sys.executable, "-c", code, root],
        cwd=str(package),
        capture_output=True,
        text=True,
        timeout=60,
    )
    assert free.returncode == 0 and free.stdout.strip() == "free"


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
