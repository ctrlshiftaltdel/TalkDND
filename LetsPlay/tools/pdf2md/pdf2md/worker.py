"""Book discovery, priority, lock, budget and the background loop."""

import json
import os
import re
import sys
import time

from . import repair

BOOK_DIRS = ("Adventures", os.path.join("Rules", "2014"), os.path.join("Rules", "2024"))
LOCK_STALE_SECONDS = 600
WORDS_URL = ("https://raw.githubusercontent.com/dwyl/english-words/"
             "8179fe68775df3f553ef19520db065228e65d1d3/words_alpha.txt")
WORDS_SHA256 = "3ed0c94610d8bcf7c11bbb49c56aa49c7234d32b66824df91f554169e572da48"


def letsplay_root():
    env = os.environ.get("PDF2MD_ROOT")
    if env:
        return os.path.abspath(env)
    # LetsPlay/tools/pdf2md/pdf2md/worker.py -> LetsPlay
    return os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))


def state_dir(root):
    return os.path.join(root, ".pdf2md")


def words_path(root):
    return os.environ.get("PDF2MD_WORDS") or os.path.join(state_dir(root), "words_alpha.txt")


def scan(root):
    """All PDFs under the book folders, skipping hidden folders."""
    found = []
    for rel in BOOK_DIRS:
        base = os.path.join(root, rel)
        for dirpath, dirnames, filenames in os.walk(base):
            dirnames[:] = sorted(d for d in dirnames if not d.startswith("."))
            for name in sorted(filenames):
                if name.lower().endswith(".pdf"):
                    found.append(os.path.join(dirpath, name))
    return found


def _slug(s):
    return re.sub(r"[^a-z0-9]+", "", s.lower())


def current_save(root):
    saves = os.path.join(root, "Saves")
    if not os.path.isdir(saves):
        return None
    slots = sorted(d for d in os.listdir(saves) if os.path.isdir(os.path.join(saves, d)) and d[:1].isdigit())
    return os.path.join(saves, slots[-1]) if slots else None


def save_info(save):
    """(ruleset, [adventure dirs]) from Saves/<slot>/combat/campaign.yaml (no YAML dependency)."""
    if not save:
        return "2024", []
    path = os.path.join(save, "combat", "campaign.yaml")
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return "2024", []
    m = re.search(r'^ruleset:\s*"?(\d{4})"?', text, re.M)
    ruleset = m.group(1) if m else "2024"
    dirs = []
    m = re.search(r"^adventures:\s*\[(.*?)\]", text, re.M)
    if m:
        for item in m.group(1).split(","):
            item = item.strip().strip("'\"")
            if item:
                d = os.path.normpath(os.path.join(os.path.dirname(path), item))
                if os.path.basename(d) == "combat":
                    d = os.path.dirname(d)
                dirs.append(d)
    return ruleset, dirs


def prioritise(root, pdfs, save=None, first=None):
    """Current adventure, then this ruleset's SRD and books, then everything else."""
    ruleset, adv_dirs = save_info(save if save is not None else current_save(root))
    adv_slugs = {_slug(os.path.basename(d)) for d in adv_dirs}

    def rank(pdf):
        stem = os.path.splitext(os.path.basename(pdf))[0]
        if first and (_slug(first) in _slug(stem)):
            return 0
        if any(os.path.abspath(pdf).startswith(os.path.abspath(d) + os.sep) for d in adv_dirs):
            return 1
        if _slug(stem) in adv_slugs:
            return 1
        rules = os.path.join(root, "Rules", ruleset) + os.sep
        if pdf.startswith(rules):
            return 2 if "srd" in stem.lower() else 3
        if pdf.startswith(os.path.join(root, "Rules") + os.sep):
            return 5
        return 4
    return sorted(pdfs, key=lambda p: (rank(p), p))


# ------------------------------------------------------------------ lock
def _try_lock(fd):
    """Take an exclusive OS lock on `fd` without blocking. False when taken."""
    if os.name == "nt":  # pragma: no cover - Windows
        import msvcrt

        try:
            os.lseek(fd, 0, os.SEEK_SET)
            if os.fstat(fd).st_size == 0:
                os.write(fd, b"\n")
            os.lseek(fd, 0, os.SEEK_SET)
            msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)
            return True
        except OSError:
            return False
    import fcntl

    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return True
    except OSError:
        return False


def _unlock(fd):
    if os.name == "nt":  # pragma: no cover - Windows
        import msvcrt

        try:
            os.lseek(fd, 0, os.SEEK_SET)
            msvcrt.locking(fd, msvcrt.LK_UNLCK, 1)
        except OSError:
            pass
        return
    import fcntl

    try:
        fcntl.flock(fd, fcntl.LOCK_UN)
    except OSError:
        pass


def _same_file(fd, path):
    """True when `path` still names the inode behind `fd` (no unlink/replace race)."""
    try:
        a, b = os.fstat(fd), os.stat(path)
    except OSError:
        return False
    return (a.st_dev, a.st_ino) == (b.st_dev, b.st_ino)


def _pid_alive(pid):
    """Best-effort liveness check for a lock-file owner pid."""
    if pid <= 0:
        return False
    if pid == os.getpid():
        return True
    if os.name == "nt":  # pragma: no cover - Windows
        import ctypes

        process_query_limited_information = 0x1000
        still_active = 259
        kernel32 = ctypes.windll.kernel32
        handle = kernel32.OpenProcess(process_query_limited_information, False, pid)
        if not handle:
            return False
        try:
            code = ctypes.c_ulong()
            if not kernel32.GetExitCodeProcess(handle, ctypes.byref(code)):
                return False
            return code.value == still_active
        finally:
            kernel32.CloseHandle(handle)
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except (OSError, OverflowError):
        return False


def _read_owner(fd):
    """Owner record from the lock file (JSON, or the old bare-pid format)."""
    try:
        os.lseek(fd, 0, os.SEEK_SET)
        raw = os.read(fd, 4096).decode("utf-8", "replace").strip()
    except OSError:
        return None
    if not raw:
        return None
    try:
        data = json.loads(raw)
        return data if isinstance(data, dict) else None
    except ValueError:
        try:
            return {"pid": int(raw.split()[0])}
        except (ValueError, IndexError):
            return None


def _owner_stale(owner, path):
    """Stale = owner is dead, released, or older than LOCK_STALE_SECONDS."""
    if not owner or owner.get("released"):
        return True
    pid = owner.get("pid")
    if not isinstance(pid, int) or not _pid_alive(pid):
        return True
    updated = owner.get("updated", owner.get("started"))
    if not isinstance(updated, (int, float)):
        try:
            updated = os.path.getmtime(path)
        except OSError:
            return True
    return time.time() - float(updated) >= LOCK_STALE_SECONDS


class Lock:
    """Cross-process worker lock: an OS file lock plus a diagnostic owner record.

    `acquire` never removes a live lock and has no remove-then-create window.
    The OS lock (``flock`` on POSIX, ``msvcrt.locking`` on Windows) is held for
    the object's lifetime; a crashed process releases it automatically, and a
    stale file is replaced after checking its owner pid and age. The file is
    unlinked only while its inode is still locked, so a second worker cannot end
    up holding the same path with a different inode.
    """

    def __init__(self, root):
        self.path = os.path.join(state_dir(root), "worker.lock")
        self.fd = None

    def _write_owner(self, fd, started):
        now = time.time()
        data = json.dumps({"pid": os.getpid(), "started": started, "updated": now})
        os.lseek(fd, 0, os.SEEK_SET)
        os.ftruncate(fd, 0)
        os.write(fd, data.encode("utf-8"))
        os.fsync(fd)
        try:
            os.utime(self.path, None)
        except OSError:
            pass

    def acquire(self):
        os.makedirs(os.path.dirname(self.path), exist_ok=True)
        for _ in range(3):  # retry only when the path was unlinked under us
            try:
                fd = os.open(self.path, os.O_RDWR | os.O_CREAT, 0o644)
            except OSError:
                return False
            if not _try_lock(fd):
                os.close(fd)
                return False
            if not _same_file(fd, self.path):
                _unlock(fd)
                os.close(fd)
                continue
            owner = _read_owner(fd)
            if owner is not None and not _owner_stale(owner, self.path):
                # A live owner without our OS lock (e.g. an older pdf2md):
                # never steal it.
                _unlock(fd)
                os.close(fd)
                return False
            try:
                self._write_owner(fd, time.time())
            except OSError:
                _unlock(fd)
                os.close(fd)
                return False
            self.fd = fd
            return True
        return False

    def touch(self):
        if self.fd is None:
            return
        try:
            os.lseek(self.fd, 0, os.SEEK_SET)
            raw = os.read(self.fd, 4096).decode("utf-8", "replace")
            owner = json.loads(raw) if raw else {}
            if not isinstance(owner, dict):
                owner = {}
            started = owner.get("started", time.time())
            self._write_owner(self.fd, started)
        except (OSError, ValueError):
            pass

    def release(self):
        fd = self.fd
        if fd is None:
            return
        self.fd = None
        if _same_file(fd, self.path):
            try:
                os.remove(self.path)
            except OSError:
                pass
        _unlock(fd)
        try:
            os.close(fd)
        except OSError:
            pass


def lower_priority():
    try:
        if hasattr(os, "nice"):
            os.nice(10)
        elif sys.platform == "win32":  # pragma: no cover
            import ctypes

            below_normal = 0x00004000
            handle = ctypes.windll.kernel32.GetCurrentProcess()
            ctypes.windll.kernel32.SetPriorityClass(handle, below_normal)
    except Exception:  # pragma: no cover
        pass


def log(root, msg):
    os.makedirs(state_dir(root), exist_ok=True)
    line = time.strftime("%Y-%m-%d %H:%M:%S ") + msg
    with open(os.path.join(state_dir(root), "log.txt"), "a", encoding="utf-8") as f:
        f.write(line + "\n")
    print(line, flush=True)


def parse_pages(spec):
    if not spec:
        return None
    pages = set()
    for part in spec.split(","):
        a, _, b = part.partition("-")
        lo, hi = int(a), int(b or a)
        pages.update(range(lo - 1, hi))
    return pages


def run(root, budget_seconds=None, budget_pages=None, book=None, pages=None, chunk_size=10,
        ocr_engine="rapid", save=None, nice=True, only=False, chapter_open=None):
    """Convert pending chunks in priority order until done or out of budget."""
    from .convert import Book

    lock = Lock(root)
    if not lock.acquire():
        log(root, "another pdf2md worker is running; exiting")
        return 3
    try:
        if nice:
            lower_priority()
        repair.load_words(words_path(root))
        start, done_pages = time.time(), 0
        pdfs = prioritise(root, scan(root), save=save, first=book)
        if book:
            pdfs = [p for p in pdfs if repair_slug(book) in repair_slug(os.path.basename(p))] + \
                ([] if only else [p for p in pdfs if repair_slug(book) not in repair_slug(os.path.basename(p))])
        wanted = parse_pages(pages)
        for pdf in pdfs:
            try:
                b = Book(pdf, chunk_size=chunk_size, ocr_engine=ocr_engine,
                         chapter_open=chapter_open)
            except Exception as err:  # corrupt / encrypted PDF
                log(root, f"skip {os.path.basename(pdf)}: {err}")
                continue
            todo = b.pending(wanted if (book and wanted) else None) or ([] if wanted and book else b.pending())
            changed = False
            for a, z in todo:
                if budget_seconds is not None and time.time() - start >= budget_seconds:
                    break
                if budget_pages is not None and done_pages >= budget_pages:
                    break
                b.convert_chunk(a, z)
                done_pages += z - a
                changed = True
                lock.touch()
                log(root, f"{b.stem}: pages {a + 1}-{z} done ({b.done_pages()}/{b.m['page_count']})")
            if changed or not os.path.exists(os.path.join(b.dir, b.stem + ".pages.md")):
                b.assemble()
            b.close()
            if (budget_seconds is not None and time.time() - start >= budget_seconds) or \
                    (budget_pages is not None and done_pages >= budget_pages):
                log(root, "budget used; stopping (run again to continue)")
                break
            wanted = None if book else wanted
        write_status(root)
        return 0
    finally:
        lock.release()


def repair_slug(s):
    return _slug(os.path.splitext(s)[0])


def _intake_from(d, manifest):
    """Intake audit from the manifest, else index.json, else audit.json."""
    from .intake import from_manifest

    a = from_manifest(manifest)
    if a is not None:
        return a
    for name in ("index.json", "audit.json"):
        try:
            with open(os.path.join(d, name), encoding="utf-8") as f:
                data = json.load(f)
        except (OSError, ValueError):
            continue
        a = data.get("intake") if isinstance(data, dict) else None
        if isinstance(a, dict):
            return a
    return None


def status(root):
    rows = []
    for pdf in prioritise(root, scan(root)):
        from .convert import out_dir

        d, stem = out_dir(pdf)
        row = {"pdf": os.path.relpath(pdf, root), "out": os.path.relpath(os.path.join(d, stem + ".md"), root),
               "pages": None, "done": 0, "low": 0, "ocr": 0, "stale": False,
               "fallback": 0, "intake": None}
        try:
            with open(os.path.join(d, "manifest.json"), encoding="utf-8") as f:
                m = json.load(f)
        except OSError:
            m = None
        except ValueError as err:
            row["error"] = f"invalid manifest: {err}"
            m = None
        else:
            if not isinstance(m, dict):
                row["error"] = f"invalid manifest: expected an object, got {type(m).__name__}"
                m = None
        if m is not None:
            try:
                pages = m["pages"]
                page_count = m["page_count"]
                if not isinstance(page_count, int):
                    raise TypeError("page_count is not a number")
                if not isinstance(pages, dict):
                    raise TypeError("pages is not an object")
                row["pages"] = page_count
                row["done"] = len(pages)
                row["low"] = sum(1 for i in pages.values()
                                 if isinstance(i, dict) and i.get("quality") == "low")
                row["ocr"] = sum(1 for i in pages.values()
                                 if isinstance(i, dict) and i.get("ocr"))
                row["fallback"] = sum(
                    1 for i in pages.values() if isinstance(i, dict)
                    and ({"page-fallback", "cropbox-clamped", "page-error", "render-failed"}
                         & set(i.get("flags", []))))
                row["stale"] = m.get("size") != os.path.getsize(pdf)
            except (TypeError, AttributeError, KeyError, ValueError, OSError) as err:
                row["error"] = f"invalid manifest: {err}"
        if m is not None:
            row["intake"] = _intake_from(d, m)
        rows.append(row)
    return rows


def write_status(root):
    from .convert import _write_atomic

    rows = status(root)
    os.makedirs(state_dir(root), exist_ok=True)
    _write_atomic(os.path.join(state_dir(root), "status.json"),
                  json.dumps(rows, indent=1, ensure_ascii=False))
    return rows


def fetch_words(root):
    """Download the public-domain English word list (dwyl/english-words) and verify it."""
    import hashlib
    import urllib.request

    dest = words_path(root)
    if os.path.exists(dest):
        with open(dest, "rb") as f:
            if hashlib.sha256(f.read()).hexdigest() == WORDS_SHA256:
                return dest
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    with urllib.request.urlopen(WORDS_URL, timeout=60) as r:
        data = r.read()
    if hashlib.sha256(data).hexdigest() != WORDS_SHA256:
        raise RuntimeError("word list checksum mismatch")
    with open(dest + ".tmp", "wb") as f:
        f.write(data)
    os.replace(dest + ".tmp", dest)
    return dest
