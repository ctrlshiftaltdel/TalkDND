"""Incremental, resumable conversion of one PDF into hidden Markdown output.

Layout for ``<dir>/<Stem>.pdf``::

    <dir>/.md/<Stem>/
        <Stem>.md        assembled book, page anchors <a id="pN"></a>
        index.md         headings / stat blocks / figures -> pages
        index.json
        manifest.json    hash, chunk + page status, quality flags
        pages/p0001-0010.md
        img/p0012.webp   page images for maps and figures (linked, not embedded)
"""

import hashlib
import io
import json
import os
import re
import sys
import time

from . import VERSION, headings, ocr, repair, tables

TEXT_MIN_CHARS = 40          # fewer text-layer characters -> OCR the page
FIGURE_MAX_CHARS = 600       # figure page: little text ...
FIGURE_MIN_COVER = 0.45      # ... and a large picture
OCR_DPI = 200
IMG_DPI = 150
LOW_CONF = 0.80
STATBLOCK_RE = re.compile(
    r"(\bArmor Class\b|\*\*AC\*\*|\bAC \d+\b).{0,400}?(\bHit Points\b|\*\*HP\*\*|\bHP \d+)", re.S)
ABILITY_RE = re.compile(r"\bSTR\b.{0,80}\bDEX\b|\bStr\b.{0,80}\bDex\b", re.S | re.I)


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
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    mode = "wb" if isinstance(data, bytes) else "w"
    kw = {} if mode == "wb" else {"encoding": "utf-8", "newline": "\n"}
    with open(tmp, mode, **kw) as f:
        f.write(data)
    os.replace(tmp, path)


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
        if not m or m.get("sha256") != digest or m.get("tool") != VERSION:
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
        for name in ("manifest.json", "index.json", "index.md", "headings.json", self.stem + ".md"):
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
        md_by_page = {}
        if text_pages:
            with _quiet():
                chunks = pymupdf4llm.to_markdown(
                    self.doc, pages=text_pages, page_chunks=True, write_images=False,
                    use_ocr=False, header=False, footer=False, show_progress=False)
            for ch in chunks:
                md_by_page[ch["metadata"]["page_number"] - 1] = ch["text"]
        for p in ocr_pages:
            page = self.doc[p]
            pix = page.get_pixmap(dpi=OCR_DPI)
            try:
                engine = ocr.get_engine(self.ocr_engine)
                text, conf = ocr.lines_to_markdown(engine.read(pix.tobytes("png")), pix.width)
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
            q = page_quality(body, info[p])
            info[p]["quality"] = q
            if "figure" in info[p]["flags"]:
                rel = self.render_page(p)
                body = (body + "\n\n" if body else "") + \
                    f"![Map or illustration, PDF p.{p + 1}]({rel}) (open the PDF page for full detail)"
            note = ""
            if q == "low":
                note = f"\n> Low-quality extraction ({', '.join(info[p]['flags']) or 'check'}): use PDF p.{p + 1}.\n"
            parts.append(f'<a id="p{p + 1}"></a>\n<!-- PDF p.{p + 1} -->{note}\n{body}\n')
        md = "\n".join(parts)
        _write_atomic(os.path.join(self.dir, "pages", chunk_key(a, b) + ".md"), md)
        for p, i in info.items():
            self.m["pages"][str(p + 1)] = i
        self.m["chunks"][chunk_key(a, b)] = {"seconds": round(time.time() - t0, 2)}
        self.save()
        return md

    def render_page(self, p, dpi=IMG_DPI):
        """Write img/pNNNN.webp and return its path relative to the book folder."""
        name = "p%04d.webp" % (p + 1)
        path = os.path.join(self.dir, "img", name)
        if not os.path.exists(path):
            pix = self.doc[p].get_pixmap(dpi=dpi)
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
            f"> Progress: {self.done_pages()}/{pc} pages. Pages marked low quality, stat blocks, and maps: read the PDF.",
            "",
        ]
        body = []
        for a, b in self.ranges:
            path = os.path.join(self.dir, "pages", chunk_key(a, b) + ".md")
            if chunk_key(a, b) in self.m["chunks"] and os.path.exists(path):
                with open(path, encoding="utf-8") as f:
                    body.append(f.read())
            else:
                body.append(f"> PDF pages {a + 1}-{b} not converted yet.\n")
        md = "\n".join(head) + "\n" + "\n".join(body)
        _write_atomic(os.path.join(self.dir, self.stem + ".md"), md)
        self.write_index(md)
        return md

    def write_index(self, md):
        idx = {"pdf": self.m["pdf"], "book": self.stem + ".md", "page_count": self.m["page_count"],
               "done_pages": self.done_pages(), "headings": [], "statblocks": [], "figures": [],
               "low_quality_pages": [], "ocr_pages": []}
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
            if i.get("quality") == "low":
                idx["low_quality_pages"].append(p)
            if i.get("ocr"):
                idx["ocr_pages"].append(p)
        _write_atomic(os.path.join(self.dir, "index.json"), json.dumps(idx, indent=1, ensure_ascii=False))
        book = self.stem + ".md"
        lines = [f"# Index: {self.stem}", "",
                 f"Book: [{book}](<{book}>). PDF: `{self.m['pdf']}` (authoritative). "
                 f"{idx['done_pages']}/{idx['page_count']} pages converted.", "", "## Headings", ""]
        for h in idx["headings"]:
            lines.append("  " * (h["level"] - 1) + f"- [{h['text']}](<{book}#p{h['page']}>) - p.{h['page']}")
        lines += ["", "## Stat blocks (read the PDF)", "",
                  ", ".join(f"[p.{p}](<{book}#p{p}>)" for p in idx["statblocks"]) or "none", "",
                  "## Maps and illustrations", ""]
        lines += [f"- p.{f['page']}: [image]({f['img']})" for f in idx["figures"]] or ["none"]
        lines += ["", "## Low-quality pages (read the PDF)", "",
                  ", ".join(str(p) for p in idx["low_quality_pages"]) or "none", ""]
        _write_atomic(os.path.join(self.dir, "index.md"), "\n".join(lines))


def postprocess(md, hmap):
    md = repair.strip_picture_text(md)
    lines = []
    for ln in md.split("\n"):
        if headings.HEADING_RE.match(ln):
            ln = headings.normalize_heading(ln, hmap)
        elif not ln.startswith("|"):
            ln = repair.fix_line_structure(repair.fix_dice_line(repair.repair_text(ln)))
        lines.append(ln)
    md = "\n".join(repair.structural_pass(lines))
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
    bad = {"statblock", "table", "ocr-low-conf", "low-yield", "empty"}
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
