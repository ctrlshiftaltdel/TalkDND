# pdf2md

Incremental PDF → Markdown extraction for the books players put in `Rules/2014/`, `Rules/2024/` and `Adventures/`. The PDF stays authoritative; the Markdown is a searchable copy.

Run everything from the play folder (the extracted package folder, any name) through [uv](https://docs.astral.sh/uv/):

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
- **Tool upgrades:** when the pdf2md version changes, an unchanged PDF keeps its old pages (marked as being redone) and is reconverted chunk by chunk, so the book stays readable. Upgrades listed in `SELECTIVE_UPGRADES` redo only the chunks they affect (0.2.0 → 0.2.1 and 0.2.1 → 0.2.2: pages that used the text-layer fallback; 0.2.2 judges fallback headings against the page's body font size instead of fixed point sizes; 0.2.2 → 0.2.3: OCR and fallback pages, where sentence-like headings are demoted to text).
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

## Audit, page fixes and agent review

```
uv run --project tools/pdf2md python -m pdf2md audit [--book NAME] [--threshold 25]
uv run --project tools/pdf2md python -m pdf2md fix-queue [--book NAME] [--limit 20]
uv run --project tools/pdf2md python -m pdf2md review --book NAME [--page 12,30-32 | --top 10] [--engine grok|cmd:CMD] [--jobs 2] [--rejudge]
uv run --project tools/pdf2md python -m pdf2md apply [--book NAME]
```

- `audit` scores every page against the PDF text layer and writes `.md/<stem>/audit.json` worst-first.
- `fixes/pNNNN.md` overrides page N when the book is assembled (header: PDF sha256 + tool version; ignored for another PDF).
- `review` runs an agent on a page and keeps the result when scores do not regress. A small recall drop (≤ 0.05) is allowed on structured / stat-block pages when unknown/1k improves, word count stays ≥ 90%, and badness does not worsen; hard reject if unknown/1k rises or words fall below 50%. Otherwise the output goes to `fixes/rejected/`. `--rejudge` re-scores rejected files with the current gate (no agent). Grok Build defaults to 60 max turns and retries once on max-turns with a short prompt; set `PDF2MD_GROK_MODEL` to pick a model. Lessons and thresholds: `docs/LESSONS.md`.
- `apply` rebuilds the book now (only when no worker holds the lock).
