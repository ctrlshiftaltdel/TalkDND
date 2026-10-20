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
    a = sub.add_parser("audit", help="score every converted page against the PDF text layer (audit.json)")
    a.add_argument("--book")
    a.add_argument("--threshold", type=float, default=25)
    q = sub.add_parser("fix-queue", help="pages that need review, worst first")
    q.add_argument("--book")
    q.add_argument("--limit", type=int, default=20)
    ap_ = sub.add_parser("apply", help="rebuild <book>.md with fixes/ applied (waits for no worker)")
    ap_.add_argument("--book")
    rv = sub.add_parser("review", help="agent review of pages; keeps a fix only if scores do not regress")
    rv.add_argument("--book", required=True)
    rv.add_argument("--page", help="PDF pages, e.g. 12,30-32 (default: worst pages from fix-queue)")
    rv.add_argument("--top", type=int, default=10)
    rv.add_argument("--engine", default=os.environ.get("PDF2MD_REVIEW_ENGINE", "grok"),
                    help="grok (Grok Build) or cmd:<command> (any agent; prompt file passed last, markdown on stdout)")
    rv.add_argument("--jobs", type=int, default=2)
    rv.add_argument("--rejudge", action="store_true",
                    help="re-score fixes/rejected/ with the current gate; no agent call")
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
    if args.cmd in ("audit", "fix-queue", "apply", "review"):
        return reviewcmd(root, args)
    return 2


def _books(root, name):
    from .convert import out_dir

    for pdf in worker.scan(root):
        if name and worker.repair_slug(name) not in worker.repair_slug(os.path.basename(pdf)):
            continue
        d, stem = out_dir(pdf)
        if os.path.exists(os.path.join(d, "manifest.json")) and os.path.exists(os.path.join(d, stem + ".md")):
            yield pdf


def _pages(spec):
    out = []
    for part in spec.split(","):
        a, _, b = part.partition("-")
        out += list(range(int(a), int(b or a) + 1))
    return out


def reviewcmd(root, args):
    import json
    from concurrent.futures import ThreadPoolExecutor

    from . import audit, repair, review

    repair.load_words(os.path.join(worker.state_dir(root), "words_alpha.txt"))
    rc = 0
    for pdf in _books(root, args.book):
        if args.cmd == "apply":
            from .convert import Book

            lock = worker.Lock(root)
            if not lock.acquire():
                print("worker running: fixes will be applied when it next assembles the book")
                return 1
            try:
                Book(pdf).assemble()
            finally:
                lock.release()
            print("rebuilt", os.path.basename(pdf))
            continue
        bv = audit.BookView(pdf)
        rows = audit.audit_book(bv, getattr(args, "threshold", 25))
        if args.cmd == "audit":
            sm = json.load(open(os.path.join(bv.dir, "audit.json"), encoding="utf-8"))["summary"]
            print(f"{bv.stem}: {sm['pages']} pages, {sm['need_review']} need review, {sm['fixed']} fixed")
        elif args.cmd == "fix-queue":
            for r in [r for r in rows if r["review"] and not r["fixed"]][:args.limit]:
                print(f"{bv.stem} p.{r['page']:<4} badness {r['badness']:<6} recall {r['recall']} "
                      f"unknown/1k {r['unknown_1k']} heads {r['sus_heads']} tables {r['table_issues']} {','.join(r['flags'])}")
        elif getattr(args, "rejudge", False):
            recs = review.rejudge_book(bv)
            if args.page:
                want = set(_pages(args.page))
                recs = [r for r in recs if r["page"] in want]
                # also rejudge specific pages even if not currently under rejected/
                have = {r["page"] for r in recs}
                for p in want - have:
                    recs.append(review.rejudge_page(bv, p))
            for rec in recs:
                b, a_ = rec.get("before") or {}, rec.get("after") or {}
                print(f"{bv.stem} p.{rec['page']}: {rec['status']} (rejudge) "
                      f"recall {b.get('recall')}->{a_.get('recall')} unknown/1k {b.get('unknown_1k')}->{a_.get('unknown_1k')} "
                      f"{'; '.join(rec.get('reasons', []))}", flush=True)
            audit.audit_book(bv)
        else:
            pages = _pages(args.page) if args.page else \
                [r["page"] for r in rows if r["review"] and not r["fixed"]][:args.top]
            with ThreadPoolExecutor(max(1, args.jobs)) as ex:
                for rec in ex.map(lambda p: review.review_page(bv, p, args.engine), pages):
                    b, a_ = rec["before"], rec.get("after") or {}
                    print(f"{bv.stem} p.{rec['page']}: {rec['status']} in {rec['time']}s "
                          f"recall {b['recall']}->{a_.get('recall')} unknown/1k {b['unknown_1k']}->{a_.get('unknown_1k')} "
                          f"inferred {a_.get('inferred', 0)} {'; '.join(rec.get('reasons', []))}", flush=True)
                    rc |= rec["status"] == "error"
            audit.audit_book(bv)
    return rc


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
