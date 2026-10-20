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
class Lock:
    def __init__(self, root):
        self.path = os.path.join(state_dir(root), "worker.lock")

    def acquire(self):
        os.makedirs(os.path.dirname(self.path), exist_ok=True)
        try:
            age = time.time() - os.path.getmtime(self.path)
            if age < LOCK_STALE_SECONDS:
                return False
            os.remove(self.path)
        except OSError:
            pass
        try:
            fd = os.open(self.path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        except FileExistsError:
            return False
        with os.fdopen(fd, "w") as f:
            f.write(str(os.getpid()))
        return True

    def touch(self):
        try:
            os.utime(self.path, None)
        except OSError:
            pass

    def release(self):
        try:
            os.remove(self.path)
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
        ocr_engine="rapid", save=None, nice=True):
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
                [p for p in pdfs if repair_slug(book) not in repair_slug(os.path.basename(p))]
        wanted = parse_pages(pages)
        for pdf in pdfs:
            try:
                b = Book(pdf, chunk_size=chunk_size, ocr_engine=ocr_engine)
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
            if changed or not os.path.exists(os.path.join(b.dir, b.stem + ".md")):
                b.assemble()
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


def status(root):
    rows = []
    for pdf in prioritise(root, scan(root)):
        from .convert import out_dir

        d, stem = out_dir(pdf)
        row = {"pdf": os.path.relpath(pdf, root), "out": os.path.relpath(os.path.join(d, stem + ".md"), root),
               "pages": None, "done": 0, "low": 0, "ocr": 0, "stale": False}
        try:
            with open(os.path.join(d, "manifest.json"), encoding="utf-8") as f:
                m = json.load(f)
            row["pages"] = m["page_count"]
            row["done"] = sum(1 for _ in m["pages"])
            row["low"] = sum(1 for i in m["pages"].values() if i.get("quality") == "low")
            row["ocr"] = sum(1 for i in m["pages"].values() if i.get("ocr"))
            row["stale"] = m.get("size") != os.path.getsize(pdf)
        except (OSError, ValueError, KeyError):
            pass
        rows.append(row)
    return rows


def write_status(root):
    rows = status(root)
    os.makedirs(state_dir(root), exist_ok=True)
    with open(os.path.join(state_dir(root), "status.json"), "w", encoding="utf-8") as f:
        json.dump(rows, f, indent=1, ensure_ascii=False)
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
