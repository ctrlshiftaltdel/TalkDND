"""Command line: ``uv run --project tools/pdf2md python -m pdf2md <command>`` from LetsPlay/."""

import argparse
import os
import sys

from . import VERSION, ocr, worker


def main(argv=None):
    ap = argparse.ArgumentParser(prog="pdf2md", description="Incremental PDF -> Markdown for TalkDND books.")
    ap.add_argument("--root", help="LetsPlay folder (default: auto)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("setup", help="download the word list and check the OCR engine")
    s.add_argument("--ocr-engine", choices=ocr.ENGINES, default="rapid")
    s.add_argument("--no-words", action="store_true")
    sub.add_parser("scan", help="list PDFs in priority order")
    r = sub.add_parser("run", help="convert pending pages")
    r.add_argument("--budget", type=float, help="stop after this many seconds")
    r.add_argument("--budget-pages", type=int, help="stop after this many pages")
    r.add_argument("--book", help="PDF name (or part of it) to do first; the rest of the queue follows unless --only")
    r.add_argument("--only", action="store_true", help="with --book: stop after that book")
    r.add_argument("--pages", help="with --book: PDF pages first, e.g. 12-20,31")
    r.add_argument("--chunk", type=int, default=10, help="pages per chunk (new books only)")
    r.add_argument("--ocr-engine", choices=ocr.ENGINES, default=os.environ.get("PDF2MD_OCR", "rapid"))
    r.add_argument("--no-nice", action="store_true")
    sub.add_parser("status", help="show progress")
    v = sub.add_parser("verify", help="check converted books; exit 1 on structural problems")
    v.add_argument("--book")
    g = sub.add_parser("render", help="write a page image for a map or figure")
    g.add_argument("book")
    g.add_argument("page", type=int, help="PDF page number (1-based)")
    args = ap.parse_args(argv)
    root = os.path.abspath(args.root) if args.root else worker.letsplay_root()

    if args.cmd == "setup":
        print(f"pdf2md {VERSION}; LetsPlay = {root}")
        if not args.no_words:
            print("word list:", worker.fetch_words(root))
        eng = ocr.get_engine(args.ocr_engine)
        print(f"OCR engine ready: {eng.name}")
        import pymupdf
        import pymupdf4llm

        print(f"pymupdf {pymupdf.__version__}, pymupdf4llm {pymupdf4llm.__version__}")
        print("setup OK")
        return 0
    if args.cmd == "scan":
        for p in worker.prioritise(root, worker.scan(root)):
            print(os.path.relpath(p, root))
        return 0
    if args.cmd == "run":
        return worker.run(root, budget_seconds=args.budget, budget_pages=args.budget_pages, book=args.book, only=args.only,
                          pages=args.pages, chunk_size=args.chunk, ocr_engine=args.ocr_engine,
                          nice=not args.no_nice)
    if args.cmd == "status":
        rows = worker.write_status(root)
        for row in rows:
            total = row["pages"] if row["pages"] is not None else "?"
            flag = " (PDF changed: will restart)" if row["stale"] else ""
            print(f"{row['done']:>4}/{total:<4} low {row['low']:<3} ocr {row['ocr']:<3} {row['pdf']}{flag}")
        return 0
    if args.cmd == "verify":
        return verify(root, args.book)
    if args.cmd == "render":
        from .convert import Book

        pdf = _find(root, args.book)
        b = Book(pdf)
        print(os.path.join(b.dir, b.render_page(args.page - 1)))
        return 0
    return 2


def _find(root, name):
    slug = worker.repair_slug(name)
    for p in worker.scan(root):
        if slug in worker.repair_slug(os.path.basename(p)):
            return p
    sys.exit(f"no PDF matches {name!r}")


def verify(root, book=None):
    import json
    import re

    from . import tables
    from .convert import out_dir

    rc = 0
    for pdf in worker.scan(root):
        if book and worker.repair_slug(book) not in worker.repair_slug(os.path.basename(pdf)):
            continue
        d, stem = out_dir(pdf)
        md_path = os.path.join(d, stem + ".md")
        if not os.path.exists(md_path):
            continue
        with open(md_path, encoding="utf-8") as f:
            md = f.read()
        with open(os.path.join(d, "manifest.json"), encoding="utf-8") as f:
            m = json.load(f)
        anchors = [int(x) for x in re.findall(r'<a id="p(\d+)"></a>', md)]
        problems = []
        if anchors != sorted(anchors) or len(anchors) != len(set(anchors)):
            problems.append("page anchors out of order or duplicated")
        if set(anchors) != {int(p) for p in m["pages"]}:
            problems.append("anchors do not match converted pages")
        bad = tables.check_tables(md)
        low = sum(1 for i in m["pages"].values() if i.get("quality") == "low")
        print(f"{stem}: {len(anchors)}/{m['page_count']} pages, {low} low-quality, "
              f"{len(bad)} table issues, {'OK' if not problems else '; '.join(problems)}")
        if problems:
            rc = 1
    return rc


if __name__ == "__main__":
    sys.exit(main())
