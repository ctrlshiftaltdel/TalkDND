"""Incremental, resumable conversion of one PDF into hidden Markdown output.

Layout for ``<dir>/<Stem>.pdf``::

    <dir>/.md/<Stem>/
        <Stem>.md        reader edition: same text, no page anchors (play from this)
        <Stem>.pages.md  lookup edition: page anchors <a id="pN"></a> (index, review, fixes)
        index.md         headings / stat blocks / figures -> pages
        index.json
        manifest.json    hash, chunk + page status, quality flags
        pages/p0001-0010.md
        img/p0012.webp   page images for maps and figures (linked, not embedded)
        img/p0012-a1.webp  art cut out of a scanned page
"""

import hashlib
import io
import json
import os
import re
import sys
import time

from . import VERSION, fallback, headings, ocr, repair, tables

TEXT_MIN_CHARS = 40          # fewer text-layer characters -> OCR the page
FIGURE_MAX_CHARS = 600       # figure page: little text ...
FIGURE_MIN_COVER = 0.45      # ... and a large picture
OCR_DPI = 200
IMG_DPI = 150
SCAN_COVER = 0.90            # one bitmap over (almost) the whole page -> scanned page
LOW_CONF = 0.80
MAX_PIXELS = 40_000_000      # render cap: poster-size pages are drawn at a lower dpi
MIN_ORDER = 0.55             # bigram agreement with column order below this -> interleaved
MIN_RECALL = 0.6             # share of text-layer words kept below this -> fallback
STATBLOCK_RE = re.compile(
    r"(\bArmor Class\b|\*\*AC\*\*|\bAC \d+\b).{0,400}?(\bHit Points\b|\*\*HP\*\*|\bHP \d+)", re.S)
IMG_LINK_RE = re.compile(r"!\[[^\]]*\]\(([^)]+)\)")
MAP_CAPTION_RE = re.compile(r"^MAP\s+\d+\.\d+\s*:", re.I)
MIN_IMAGE_SIDE = 120
ROOM_REF_RE = re.compile(r"\b(?:areas?|room)\s+[A-Z]{1,2}[0-9lI]{1,3}\b|^#+ [A-Z]{1,2}\d{1,3}:", re.M)
ROOM_OCR_DPI = 150         # smaller embedded images are decorations
ABILITY_RE = re.compile(r"\bSTR\b.{0,80}\bDEX\b|\bStr\b.{0,80}\bDex\b", re.S | re.I)


# (from version, to version) -> predicate on a page's manifest entry. Chunks with no
# matching page are kept as they are; the rest are redone.
SELECTIVE_UPGRADES = {
    ("0.2.0", "0.2.1"): lambda info: "fallback" in info,
    ("0.2.1", "0.2.2"): lambda info: "fallback" in info,
    ("0.2.0", "0.2.2"): lambda info: "fallback" in info,
    # 0.2.3 demotes sentence-like headings on OCR and fallback pages
    ("0.2.2", "0.2.3"): lambda info: "fallback" in info or "ocr" in info,
    # 0.4.0 only changes assembly (reader + pages editions): no chunk is redone
    ("0.3.1", "0.4.0"): lambda info: False,
    # 0.4.1 only adds stat-block checks at assemble time: no chunk is redone
    ("0.4.0", "0.4.1"): lambda info: False,
    ("0.3.1", "0.4.1"): lambda info: False,
    # 0.5.0 adds the image stage (art cut out of scanned pages, maps at ~1600 px):
    # every chunk is redone; fixes/ are backed up first and keep their text.
}

ANCHOR_LINE_RE = re.compile(r'^<a id="p\d+"></a>\n?|^<!-- PDF p\.\d+ -->', re.M)
ANCHOR_NOTE = "> Each page starts with `<a id=\"pN\"></a>` where N is the PDF page number."


def pages_name(stem):
    return stem + ".pages.md"


def to_reader(pages_md, stem):
    """Reader edition: the lookup edition without page anchors / page comments.
    Everything else is identical (checked by tests)."""
    md = pages_md.replace(
        ANCHOR_NOTE,
        f"> Reader edition (no page anchors). PDF page lookup: `{pages_name(stem)}` and `index.md`.")
    md = ANCHOR_LINE_RE.sub("", md)
    return re.sub(r"\n{3,}", "\n\n", md)


def _chunk_pages(key):
    a, b = key[1:].split("-")
    return range(int(a) - 1, int(b))


def out_dir(pdf_path):
    pdf_path = os.path.abspath(pdf_path)
    stem = os.path.splitext(os.path.basename(pdf_path))[0]
    return os.path.join(os.path.dirname(pdf_path), ".md", stem), stem


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def _write_atomic(path, data):
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    tmp = path + ".tmp"
    mode = "wb" if isinstance(data, bytes) else "w"
    kw = {} if mode == "wb" else {"encoding": "utf-8", "newline": "\n"}
    try:
        with open(tmp, mode, **kw) as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
    except BaseException:
        try:
            os.remove(tmp)
        except OSError:
            pass
        raise
    _fsync_dir(os.path.dirname(path))


def _fsync_dir(path):
    """Best-effort directory sync so the rename itself is durable on POSIX."""
    if not path or os.name == "nt":
        return
    try:
        fd = os.open(path, os.O_RDONLY)
    except OSError:
        return
    try:
        os.fsync(fd)
    except OSError:
        pass
    finally:
        os.close(fd)


def chunk_ranges(page_count, size):
    return [(a, min(a + size, page_count)) for a in range(0, page_count, size)]


def chunk_key(a, b):
    return "p%04d-%04d" % (a + 1, b)


class Book:
    """One PDF and its manifest."""

    def __init__(self, pdf_path, chunk_size=10, ocr_engine="rapid"):
        import pymupdf

        self.pdf = os.path.abspath(pdf_path)
        self.dir, self.stem = out_dir(self.pdf)
        self.manifest_path = os.path.join(self.dir, "manifest.json")
        self.ocr_engine = ocr_engine
        st = os.stat(self.pdf)
        m = self._load()
        quick = m and m.get("size") == st.st_size and m.get("mtime") == int(st.st_mtime)
        digest = m["sha256"] if quick else sha256_file(self.pdf)
        if m and m.get("sha256") == digest and m.get("tool") != VERSION:
            # tool upgrade: keep the old pages readable and reconvert chunk by chunk
            redo = set(m["chunks"])
            only = SELECTIVE_UPGRADES.get((m.get("tool"), VERSION))
            if only:  # this upgrade only changes some pages: redo just their chunks
                redo = {c for c in m["chunks"] if any(
                    only(m["pages"].get(str(p + 1), {})) for p in _chunk_pages(c))}
            m["stale"] = sorted(set(m.get("stale", [])) | redo)
            m["chunks"] = {c: v for c, v in m["chunks"].items() if c not in redo}
            m["tool"] = VERSION
        if m and (m.get("sha256") != digest or m.get("stale")):
            from . import fixes as _fixes

            _fixes.backup(self.dir, "pdf-changed" if m.get("sha256") != digest else "reconvert")
        if not m or m.get("sha256") != digest:
            self._reset()
            self.doc = pymupdf.open(self.pdf)
            m = {
                "tool": VERSION, "pdf": os.path.basename(self.pdf), "sha256": digest,
                "size": st.st_size, "mtime": int(st.st_mtime),
                "page_count": self.doc.page_count, "chunk_size": chunk_size,
                "created": time.strftime("%Y-%m-%dT%H:%M:%S"), "chunks": {}, "pages": {},
            }
        else:
            self.doc = pymupdf.open(self.pdf)
            m["mtime"] = int(st.st_mtime)
        self.m = m
        self._hmap = None
        self.save()

    # -------------------------------------------------------------- manifest
    def _load(self):
        try:
            with open(self.manifest_path, encoding="utf-8") as f:
                return json.load(f)
        except (OSError, ValueError):
            return None

    def _reset(self):
        """PDF changed (or first run): remove only our own generated files."""
        if not os.path.isdir(self.dir):
            return
        for sub in ("pages", "img"):
            d = os.path.join(self.dir, sub)
            if os.path.isdir(d):
                for name in os.listdir(d):
                    os.remove(os.path.join(d, name))
        for name in ("manifest.json", "index.json", "index.md", "headings.json", self.stem + ".md",
                     pages_name(self.stem)):
            p = os.path.join(self.dir, name)
            if os.path.exists(p):
                os.remove(p)

    def save(self):
        self.m["updated"] = time.strftime("%Y-%m-%dT%H:%M:%S")
        _write_atomic(self.manifest_path, json.dumps(self.m, indent=1, ensure_ascii=False))

    @property
    def ranges(self):
        return chunk_ranges(self.m["page_count"], self.m["chunk_size"])

    def pending(self, wanted=None):
        out = []
        for a, b in self.ranges:
            if chunk_key(a, b) in self.m["chunks"]:
                continue
            if wanted and not any(a <= p < b for p in wanted):
                continue
            out.append((a, b))
        return out

    def done_pages(self):
        return sum(b - a for a, b in self.ranges if chunk_key(a, b) in self.m["chunks"])

    # -------------------------------------------------------------- headings
    def heading_map(self):
        if self._hmap is None:
            path = os.path.join(self.dir, "headings.json")
            try:
                with open(path, encoding="utf-8") as f:
                    self._hmap = json.load(f)
            except (OSError, ValueError):
                self._hmap = headings.build_heading_map(self.doc)
                _write_atomic(path, json.dumps(self._hmap))
        return self._hmap

    # -------------------------------------------------------------- one chunk
    def convert_chunk(self, a, b):
        import pymupdf4llm

        t0 = time.time()
        text_pages, ocr_pages, info = [], [], {}
        for p in range(a, b):
            page = self.doc[p]
            chars = len(page.get_text("text").strip())
            cover = _image_cover(page)
            info[p] = {"chars": chars, "flags": []}
            if chars < TEXT_MIN_CHARS:
                ocr_pages.append(p)
            else:
                text_pages.append(p)
            if chars < FIGURE_MAX_CHARS and cover >= FIGURE_MIN_COVER:
                info[p]["flags"].append("figure")
            elif cover >= SCAN_COVER and len(page.get_images()) <= 1:
                info[p]["flags"].append("scan")  # art is inside the page bitmap: image stage
        md_by_page = {}
        if text_pages:
            with _quiet():
                chunks = pymupdf4llm.to_markdown(
                    self.doc, pages=text_pages, page_chunks=True, write_images=True,
                    image_path=self._raw_dir(), image_format="png", use_ocr=False,
                    header=False, footer=False, show_progress=False)
            for ch in chunks:
                pno = ch["metadata"]["page_number"] - 1
                md_by_page[pno] = self.place_images(ch["text"], pno, info[pno])
        for p in ocr_pages:
            page = self.doc[p]
            try:
                pix = safe_pixmap(page, OCR_DPI)
                engine = ocr.get_engine(self.ocr_engine)
                text, conf = ocr.lines_to_markdown(engine.read(pix.tobytes("png")), pix.width)
            except RuntimeError as err:  # pymupdf render limits
                text, conf = "", 0.0
                info[p]["flags"].append("render-failed")
                print(f"warning: PDF p.{p + 1}: {err}", file=sys.stderr)
            except ocr.OcrUnavailable as err:
                text, conf = "", 0.0
                info[p]["flags"].append("ocr-unavailable")
                print(f"warning: {err}", file=sys.stderr)
            md_by_page[p] = text
            info[p]["ocr"] = self.ocr_engine
            info[p]["ocr_conf"] = round(conf, 3)
        hmap = self.heading_map()
        parts = []
        for p in range(a, b):
            body = postprocess(md_by_page.get(p, ""), hmap)
            if p in text_pages:
                body = self.check_text_page(p, body, hmap, info[p])
            if p not in text_pages or info[p].get("fallback"):
                body = fallback.demote_body_headings(body)
            if p in text_pages:
                from . import spaced

                body, k = spaced.restore(body, self.doc[p])
                if k:
                    info[p]["spaced_headings"] = k
            if p in text_pages and self.adventure:
                body = self.adventure_layout(p, body, info[p])
            if "scan" in info[p]["flags"]:
                body = self.scan_art(p, body, info[p])
            q = page_quality(body, info[p])
            info[p]["quality"] = q
            if "figure" in info[p]["flags"]:
                rel = self.render_page(p)
                if rel:
                    body = (body + "\n\n" if body else "") + \
                        f"![Map or illustration, PDF p.{p + 1}]({rel}) (open the PDF page for full detail)"
                else:
                    info[p]["flags"].append("render-failed")
            note = ""
            if q == "low":
                note = f"\n> Low-quality extraction ({', '.join(info[p]['flags']) or 'check'}): use PDF p.{p + 1}.\n"
            parts.append(f'<a id="p{p + 1}"></a>\n<!-- PDF p.{p + 1} -->{note}\n{body}\n')
        md = "\n".join(parts)
        _write_atomic(os.path.join(self.dir, "pages", chunk_key(a, b) + ".md"), md)
        for p, i in info.items():
            self.m["pages"][str(p + 1)] = i
        self.m["chunks"][chunk_key(a, b)] = {"seconds": round(time.time() - t0, 2)}
        if chunk_key(a, b) in self.m.get("stale", []):
            self.m["stale"].remove(chunk_key(a, b))
        self.save()
        return md

    def check_text_page(self, p, body, hmap, info):
        """Fall back to the plain text layer when the layout extractor lost, repeated
        or interleaved text; re-read garbled headings."""
        page = self.doc[p]
        ref = fallback.column_text(page)
        reason = None
        if len(fallback.words(ref)) >= 30:
            if fallback.has_stutter(body):
                reason = "stutter"
            elif fallback.recall(body, ref) < MIN_RECALL:
                reason = "empty" if not body.strip() else "lost-text"
            elif "\n|" not in "\n" + body and (
                    fallback.order_score(body, ref) < MIN_ORDER or fallback.interleaved_lines(body, ref)):
                # pages with tables keep the layout output: the fallback would flatten them
                reason = "interleaved"
        if reason:
            info["fallback"] = reason
            body = postprocess(ref, hmap)
        body, n = fallback.repair_headings(body, page, self.ocr_engine)
        if n:
            info["headings_ocr"] = n
        return body

    @property
    def adventure(self):
        parts = os.path.normpath(self.pdf).split(os.sep)
        return "Adventures" in parts

    def adventure_layout(self, p, body, info):
        """Read-aloud panels as `>` quotes; room-key headings the text layer lost."""
        from . import rooms

        page = self.doc[p]
        try:
            body, n = rooms.mark_readaloud(body, rooms.boxed_text(page))
        except RuntimeError:
            n = 0
        if n:
            info["readaloud"] = n
        if ROOM_REF_RE.search(body):
            try:
                pix = safe_pixmap(page, ROOM_OCR_DPI)
                lines = ocr.get_engine(self.ocr_engine).read(pix.tobytes("png"))
                body, k = rooms.add_missing_rooms(body, rooms.ocr_rooms(lines, pix.width))
                if k:
                    info["rooms_ocr"] = k
            except (RuntimeError, ocr.OcrUnavailable):
                pass
        return body

    def close(self):
        """Release the PDF (Windows cannot replace an open file) and temp images."""
        doc = getattr(self, "doc", None)
        if doc is not None:
            doc.close()
            self.doc = None
        raw = getattr(self, "_raw", None)
        if raw:
            import shutil

            shutil.rmtree(raw, ignore_errors=True)
            self._raw = None

    def _raw_dir(self):
        # pymupdf4llm rewrites spaces in image paths, so stage images in a
        # space-free temp folder; each image is moved into img/ right away.
        if not getattr(self, "_raw", None):
            import tempfile

            self._raw = tempfile.mkdtemp(prefix="pdf2md-img-")
        return self._raw

    def place_images(self, text, p, info):
        """Keep embedded images where they sit on the page: convert each to
        img/pNNNN-KK.webp, caption it with a nearby `MAP x.y: ...` line, drop tiny
        decorations. The page's own text stays around it."""
        lines = text.split("\n")
        out, k = [], 0
        for i, ln in enumerate(lines):
            m = IMG_LINK_RE.fullmatch(ln.strip())
            if not m:
                out.append(ln)
                continue
            src = m.group(1)
            rel = self._store_image(src, p, k)
            if not rel:
                continue
            k += 1
            near = [x.strip().strip("*").strip() for x in lines[max(0, i - 3):i + 4]]
            cap = next((x for x in near if MAP_CAPTION_RE.match(x)), None)
            alt = cap or f"Illustration, PDF p.{p + 1}"
            if cap:
                info.setdefault("maps", []).append({"caption": cap, "img": rel})
            else:
                info.setdefault("images", []).append(rel)
            out.append(f"![{alt}]({rel})")
        return "\n".join(out)

    def _store_image(self, src, p, k):
        try:
            from PIL import Image

            with Image.open(src) as img:
                if min(img.size) < MIN_IMAGE_SIDE:
                    return None
                name = "p%04d-%02d.webp" % (p + 1, k)
                buf = io.BytesIO()
                img.convert("RGBA" if img.mode in ("RGBA", "LA", "P") else "RGB").save(
                    buf, "WEBP", quality=80, method=2)
            _write_atomic(os.path.join(self.dir, "img", name), buf.getvalue())
            return "img/" + name
        except (OSError, ImportError) as err:
            print(f"warning: PDF p.{p + 1}: image skipped: {err}", file=sys.stderr)
            return None
        finally:
            try:
                os.remove(src)
            except OSError:
                pass

    def scan_art(self, p, body, info):
        """Image stage for scanned pages: a mostly-picture page becomes a full-page
        figure (~1600 px); clean art crops inside text pages become
        img/pNNNN-aK.webp (~1000 px), placed after the paragraph above them."""
        from . import figures

        page = self.doc[p]
        try:
            an = figures.analyse(page)
            if figures.full_page_art(page, an[0], an[1]):
                info["flags"].append("figure")
                return body
            rects = figures.find_art(page, an)
        except (RuntimeError, ValueError, MemoryError) as err:
            print(f"warning: PDF p.{p + 1}: image stage skipped: {err}", file=sys.stderr)
            return body
        for k, rect in enumerate(rects, 1):
            name = "p%04d-a%d.webp" % (p + 1, k)
            try:
                data = figures.render_webp(page, rect, figures.ART_SIDE)
            except RuntimeError as err:
                print(f"warning: PDF p.{p + 1}: art not rendered: {err}", file=sys.stderr)
                continue
            _write_atomic(os.path.join(self.dir, "img", name), data)
            above, below = figures.anchors(an[1], rect)
            body, where = figures.place(body, f"![Illustration, PDF p.{p + 1}](img/{name})", above, below)
            info.setdefault("art", []).append({"img": "img/" + name, "rect": [round(v, 1) for v in rect],
                                               "placed": where})
        return body

    def render_page(self, p, dpi=None):
        """Write img/pNNNN.webp (longest side ~1600 px unless `dpi` is given) and
        return its path relative to the book folder (None when it cannot be rendered)."""
        from .figures import MAP_SIDE

        name = "p%04d.webp" % (p + 1)
        path = os.path.join(self.dir, "img", name)
        if dpi is None:
            r = self.doc[p].rect
            dpi = max(36, min(300, int(MAP_SIDE * 72 / max(r.width, r.height, 1))))
        try:
            pix = safe_pixmap(self.doc[p], dpi)
        except RuntimeError as err:
            print(f"warning: PDF p.{p + 1}: cannot render: {err}", file=sys.stderr)
            return None
        try:
            from PIL import Image

            img = Image.frombytes("RGB" if pix.n < 4 else "RGBA", (pix.width, pix.height), pix.samples)
            buf = io.BytesIO()
            img.save(buf, "WEBP", quality=80, method=2)
            data = buf.getvalue()
        except ImportError:  # pragma: no cover
            name = name.replace(".webp", ".png")
            path = path.replace(".webp", ".png")
            data = pix.tobytes("png")
        _write_atomic(path, data)
        return "img/" + name

    # -------------------------------------------------------------- assemble
    def assemble(self):
        pc = self.m["page_count"]
        head = [
            f"# {self.stem}",
            "",
            f"> Extracted from `{self.m['pdf']}` by pdf2md {VERSION}. **The PDF is authoritative.**",
            "> Each page starts with `<a id=\"pN\"></a>` where N is the PDF page number.",
            f"> Progress: {self.done_pages()}/{pc} pages"
            + (f" ({len(self.m['stale'])} chunks from an older pdf2md, being redone)" if self.m.get("stale") else "")
            + f". Pages marked low quality, stat blocks, and maps: read the PDF.",
            "",
        ]
        body = []
        for a, b in self.ranges:
            path = os.path.join(self.dir, "pages", chunk_key(a, b) + ".md")
            known = chunk_key(a, b) in self.m["chunks"] or chunk_key(a, b) in self.m.get("stale", [])
            if known and os.path.exists(path):
                with open(path, encoding="utf-8") as f:
                    body.append(f.read())
            else:
                body.append(f"> PDF pages {a + 1}-{b} not converted yet.\n")
        md = "\n".join(head) + "\n" + "\n".join(body)
        from . import fixes

        md = fixes.apply_fixes(md, fixes.load_fixes(self.dir, self.m["sha256"]), refresh=True)
        from . import proofread

        md, log = proofread.proofread(md)
        if self.adventure:
            from . import rooms

            md = rooms.fix_area_refs(md)
        from . import statblocks

        md, self._checks = statblocks.annotate(md, statblocks.load_known(self.combat_data()))
        _write_atomic(os.path.join(self.dir, "proofread-report.md"),
                      proofread.report(self.stem, log).encode("utf-8"))
        _write_atomic(os.path.join(self.dir, pages_name(self.stem)), md)
        _write_atomic(os.path.join(self.dir, self.stem + ".md"), to_reader(md, self.stem))
        self.write_index(md)
        return md

    def combat_data(self):
        """combat-sim data to cross-check stat blocks: Rules/combat/{common,<ruleset>}
        for rulebooks, the adventure's combat/ for adventures."""
        parts = os.path.normpath(self.pdf).split(os.sep)
        for key in ("Rules", "Adventures"):
            if key in parts:
                i = len(parts) - 1 - parts[::-1].index(key)
                base = os.sep.join(parts[:i + 1]) or os.sep
                if key == "Rules" and i + 1 < len(parts) - 1:
                    rs = parts[i + 1]
                    return [os.path.join(base, "combat", "common"), os.path.join(base, "combat", rs)]
                if key == "Adventures" and i + 1 < len(parts) - 1:
                    return [os.path.join(base, parts[i + 1], "combat")]
        return []

    def write_index(self, md):
        idx = {"pdf": self.m["pdf"], "book": self.stem + ".md", "pages": pages_name(self.stem),
               "page_count": self.m["page_count"],
               "done_pages": self.done_pages(), "headings": [], "statblocks": [], "figures": [],
               "low_quality_pages": [], "ocr_pages": [],
               "statblock_checks": getattr(self, "_checks", [])}
        page = 0
        for line in md.split("\n"):
            m = re.match(r'<a id="p(\d+)"></a>', line)
            if m:
                page = int(m.group(1))
                continue
            h = headings.HEADING_RE.match(line)
            if h and page:
                idx["headings"].append({"level": len(h.group(1)), "text": h.group(2), "page": page})
        for p, i in sorted(self.m["pages"].items(), key=lambda kv: int(kv[0])):
            p = int(p)
            if "statblock" in i["flags"]:
                idx["statblocks"].append(p)
            if "figure" in i["flags"]:
                idx["figures"].append({"page": p, "img": "img/p%04d.webp" % p})
            for a in i.get("art", []):
                idx.setdefault("art", []).append(dict(a, page=p))
            for mp in i.get("maps", []):
                idx.setdefault("maps", []).append(dict(mp, page=p))
            if i.get("quality") == "low":
                idx["low_quality_pages"].append(p)
            if i.get("ocr"):
                idx["ocr_pages"].append(p)
        _write_atomic(os.path.join(self.dir, "index.json"), json.dumps(idx, indent=1, ensure_ascii=False))
        reader = self.stem + ".md"
        book = pages_name(self.stem)
        lines = [f"# Index: {self.stem}", "",
                 f"Read: [{reader}](<{reader}>) (no page anchors). Page lookup: [{book}](<{book}>). "
                 f"PDF: `{self.m['pdf']}` (authoritative). "
                 f"{idx['done_pages']}/{idx['page_count']} pages converted.", "", "## Headings", ""]
        for h in idx["headings"]:
            lines.append("  " * (h["level"] - 1) + f"- [{h['text']}](<{book}#p{h['page']}>) - p.{h['page']}")
        checks = idx["statblock_checks"]
        ok = sum(c["status"] == "ok" for c in checks)
        lines += ["", "## Stat blocks", "",
                  f"{ok}/{len(checks)} checked OK (numbers consistent; usable from Markdown). "
                  "Marked ⚠ in the text: read the PDF.", ""]
        lines += [f"- {'✓' if c['status'] == 'ok' else '⚠'} [{c['name'] or 'stat block'}](<{book}#p{c['page']}>)"
                  f" - p.{c['page']}" + (f": {'; '.join(c['issues'])}" if c["issues"] else "")
                  for c in checks] or ["none found"]
        lines += ["", "Pages flagged as stat-block pages by the converter: "
                  + (", ".join(f"[p.{p}](<{book}#p{p}>)" for p in idx["statblocks"]) or "none"), "",
                  "## Maps", ""]
        maps = idx.get("maps", [])
        lines += [f"- [{mp['caption']}](<{book}#p{mp['page']}>) - p.{mp['page']}: [image](<{mp['img']}>)"
                  + (" (DM map)" if re.search(r"\bDM", mp["caption"]) else
                     " (player map)" if re.search(r"player", mp["caption"], re.I) else "")
                  for mp in maps] or ["none found by caption; see the full-page images below"]
        lines += ["", "## Full-page maps and illustrations", ""]
        lines += [f"- p.{f['page']}: [image]({f['img']})" for f in idx["figures"]] or ["none"]
        art = [a for p, i in sorted(self.m["pages"].items(), key=lambda kv: int(kv[0]))
               for a in [dict(x, page=int(p)) for x in i.get("art", [])]]
        lines += ["", "## Illustrations cut from scanned pages", ""]
        lines += [f"- p.{a['page']}: [image](<{a['img']}>)" for a in art] or ["none"]
        lines += ["", "## Low-quality pages (read the PDF)", "",
                  ", ".join(str(p) for p in idx["low_quality_pages"]) or "none", ""]
        _write_atomic(os.path.join(self.dir, "index.md"), "\n".join(lines))


def safe_pixmap(page, dpi):
    """get_pixmap with the dpi lowered for huge pages; RuntimeError if it still fails."""
    w, h = page.rect.width / 72, page.rect.height / 72
    if w * h * dpi * dpi > MAX_PIXELS:
        dpi = max(18, int((MAX_PIXELS / (w * h)) ** 0.5))
    while True:
        try:
            return page.get_pixmap(dpi=dpi)
        except Exception as err:  # pymupdf raises FzErrorLimit / MemoryError subclasses
            if dpi <= 18:
                raise RuntimeError(str(err)) from err
            dpi = max(18, dpi // 2)


def postprocess(md, hmap):
    md = repair.strip_picture_text(md)
    lines = []
    for ln in md.split("\n"):
        if headings.HEADING_RE.match(ln):
            ln = headings.normalize_heading(ln, hmap)
        elif not ln.startswith("|"):
            ln = repair.fix_line_structure(repair.fix_dice_line(repair.repair_text(ln)))
        lines.append(ln)
    md = "\n".join(headings.promote_room_lines(repair.structural_pass(lines)))
    md = tables.fix_tables(md)
    md = repair.apply_glitch_rules(md)
    return re.sub(r"\n{3,}", "\n\n", md).strip()


def page_quality(body, info):
    flags = info["flags"]
    if STATBLOCK_RE.search(body) or ABILITY_RE.search(body):
        flags.append("statblock")
    if tables.check_tables(body):
        flags.append("table")
    letters = sum(c.isalpha() for c in body)
    if info.get("ocr") and info.get("ocr_conf", 1) < LOW_CONF:
        flags.append("ocr-low-conf")
    if not info.get("ocr") and info["chars"] > 200 and letters < 0.5 * info["chars"] * 0.7:
        flags.append("low-yield")
    if "ocr-unavailable" in flags or (not body.strip() and "figure" not in flags):
        flags.append("empty")
    bad = {"statblock", "table", "ocr-low-conf", "low-yield", "empty", "render-failed"}
    return "low" if bad & set(flags) else "ok"


def _image_cover(page):
    area = abs(page.rect) or 1
    cover = 0.0
    try:
        for info in page.get_image_info():
            r = info.get("bbox")
            if r:
                x0, y0, x1, y1 = r
                cover += max(0, x1 - x0) * max(0, y1 - y0)
    except Exception:  # pragma: no cover
        return 0.0
    return min(1.0, cover / area)


class _quiet:
    """Silence pymupdf4llm's progress/info prints."""

    def __enter__(self):
        self._out, self._err = sys.stdout, sys.stderr
        sys.stdout = sys.stderr = io.StringIO()

    def __exit__(self, *exc):
        sys.stdout, sys.stderr = self._out, self._err
