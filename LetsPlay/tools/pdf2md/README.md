# pdf2md

Incremental PDF → Markdown extraction for the books players put in `Rules/2014/`, `Rules/2024/` and `Adventures/`. The PDF stays authoritative; the Markdown is a searchable copy.

Run everything from `LetsPlay/` through [uv](https://docs.astral.sh/uv/):

```bash
uv sync --project tools/pdf2md                              # first time; add --extra paddle for PaddleOCR
uv run --project tools/pdf2md python -m pdf2md setup        # word list + OCR check
uv run --project tools/pdf2md python -m pdf2md status
uv run --project tools/pdf2md python -m pdf2md run [--budget SECONDS] [--budget-pages N] [--book NAME [--only] --pages 12-20] [--ocr-engine rapid|paddle]
uv run --project tools/pdf2md python -m pdf2md verify
uv run --project tools/pdf2md python -m pdf2md render "<book>" <pdf-page>
```

`Foo.pdf` produces `.md/Foo/` next to it: `Foo.md` (page anchors `<a id="pN"></a>`, N = PDF page), `index.md` / `index.json`, `manifest.json`, `pages/` chunks, and `img/` page images for maps and figures.

- **Incremental:** 10-page chunks, written atomically. `manifest.json` stores the PDF's SHA-256, so a changed PDF starts over and an unchanged one resumes.
- **Tool upgrades:** when the pdf2md version changes, an unchanged PDF keeps its old pages (marked as being redone) and is reconverted chunk by chunk, so the book stays readable.
- **`--book`:** puts that book first; the rest of the queue follows. Add `--only` to stop after it.
- **Single worker:** a lock in `.pdf2md/worker.lock` (stale after 10 minutes) allows one worker at a time.
- **Background-friendly:** it runs at low CPU priority.
- **Order:** the current save's adventure, then that ruleset's SRD, then its other books, then other adventures, then the other ruleset.
- **OCR:** pages without a text layer are rendered at 200 dpi and OCR'd with RapidOCR (default; PP-OCR models on onnxruntime) or PaddleOCR 3.x (optional extra). The page is marked `ocr` with a confidence.
- **Repair passes**, ported from the owner's ConvertPDF (general rules only): OCR pseudo-space merging, letter-spaced headings, dice and number normalisation (`ld6`→`1d6`), table repair, heading levels from font sizes, picture-text removal, and glitch rules.
- **Safety nets (0.2):** text-layer fixes for OCR'd PDFs (`lnitiative`→`Initiative`, `ofyour`→`of your`, words wrongly marked `<sup>`); pages that come out empty, lose text, repeat a phrase, or interleave two columns fall back to the plain text layer in column order (pages with tables keep the layout output); garbled headings from decorative fonts are re-read with OCR; poster-size pages are rendered at a lower dpi and a page that still cannot be rendered is flagged `render-failed` instead of stopping the worker.
- **Quality flags:** stat blocks are flagged `statblock`, which makes the page low quality, so the DM reads the PDF for them. The same goes for broken tables, low OCR confidence and empty pages.

Removing the paddle extra later: run `uv sync --project tools/pdf2md --reinstall-package opencv-python`. Paddle's `opencv-contrib-python` shares the `cv2` directory with `opencv-python`, so uninstalling one breaks the other.

The dependencies (PyMuPDF and pymupdf4llm under AGPL-3.0, RapidOCR, optional PaddleOCR) are installed by `uv sync`. They are never bundled with TalkDND.

Tests use synthetic PDFs only: `uv run --project tools/pdf2md --group dev pytest -q tools/pdf2md/tests`.
