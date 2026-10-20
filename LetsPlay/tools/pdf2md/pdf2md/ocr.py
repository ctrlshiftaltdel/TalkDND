"""OCR engines behind one interface.

Engines return lines as ``OcrLine(x0, y0, x1, y1, text, score)`` in pixel
coordinates of the rendered page image.

* ``rapid`` (default): rapidocr-onnxruntime, PP-OCR models on onnxruntime.
* ``paddle`` (optional): PaddleOCR 3.x on paddlepaddle. Install with
  ``setup --ocr paddle``.
"""

from collections import namedtuple

OcrLine = namedtuple("OcrLine", "x0 y0 x1 y1 text score")
ENGINES = ("rapid", "paddle")
_cache = {}


class OcrUnavailable(RuntimeError):
    pass


class RapidEngine:
    name = "rapid"

    def __init__(self):
        try:
            from rapidocr_onnxruntime import RapidOCR
        except ImportError as err:  # pragma: no cover - depends on install
            raise OcrUnavailable("rapidocr-onnxruntime is not installed; run setup") from err
        self._ocr = RapidOCR()

    def read(self, png_bytes):
        result, _ = self._ocr(png_bytes)
        lines = []
        for box, text, score in result or []:
            xs = [p[0] for p in box]
            ys = [p[1] for p in box]
            lines.append(OcrLine(min(xs), min(ys), max(xs), max(ys), text, float(score)))
        return lines


class PaddleEngine:
    name = "paddle"

    def __init__(self):
        try:
            from paddleocr import PaddleOCR
        except ImportError as err:  # pragma: no cover - depends on install
            raise OcrUnavailable("paddleocr is not installed; run setup --ocr paddle") from err
        self._ocr = PaddleOCR(
            lang="en",
            use_doc_orientation_classify=False,
            use_doc_unwarping=False,
            use_textline_orientation=False,
            # Paddle 3.x oneDNN path fails on some CPUs ("ConvertPirAttribute2RuntimeAttribute").
            enable_mkldnn=False,
            # Mobile PP-OCRv5 models: the server models are ~25x slower on CPU.
            text_detection_model_name="PP-OCRv5_mobile_det",
            text_recognition_model_name="en_PP-OCRv5_mobile_rec",
        )

    def read(self, png_bytes):  # pragma: no cover - optional engine, not in CI
        import numpy as np
        import cv2

        img = cv2.imdecode(np.frombuffer(png_bytes, np.uint8), cv2.IMREAD_COLOR)
        lines = []
        for res in self._ocr.predict(img):
            data = res.json.get("res", res.json) if hasattr(res, "json") else res
            boxes = data.get("rec_boxes")
            for k, text in enumerate(data.get("rec_texts", [])):
                score = float(data.get("rec_scores", [1.0] * (k + 1))[k])
                if boxes is not None and len(boxes) > k:
                    x0, y0, x1, y1 = [float(v) for v in boxes[k]]
                else:
                    poly = data["rec_polys"][k]
                    xs = [p[0] for p in poly]
                    ys = [p[1] for p in poly]
                    x0, y0, x1, y1 = min(xs), min(ys), max(xs), max(ys)
                lines.append(OcrLine(x0, y0, x1, y1, text, score))
        return lines


def get_engine(name="rapid"):
    if name not in ENGINES:
        raise ValueError(f"unknown OCR engine {name!r}; expected one of {', '.join(ENGINES)}")
    if name not in _cache:
        _cache[name] = RapidEngine() if name == "rapid" else PaddleEngine()
    return _cache[name]


def lines_to_markdown(lines, width):
    """Order OCR lines (two columns when the page clearly has them) into paragraphs."""
    if not lines:
        return "", 0.0
    mid = width / 2
    left = [ln for ln in lines if ln.x1 <= mid + width * 0.02]
    right = [ln for ln in lines if ln.x0 >= mid - width * 0.02]
    spanning = [ln for ln in lines if ln not in left and ln not in right]
    two_col = len(left) >= 3 and len(right) >= 3 and len(spanning) <= max(2, len(lines) // 10)
    if two_col:
        ordered = sorted(spanning, key=lambda ln: ln.y0) + \
            sorted(left, key=lambda ln: ln.y0) + sorted(right, key=lambda ln: ln.y0)
        groups = [sorted(spanning, key=lambda ln: ln.y0), sorted(left, key=lambda ln: ln.y0),
                  sorted(right, key=lambda ln: ln.y0)]
    else:
        ordered = sorted(lines, key=lambda ln: (round(ln.y0 / 8), ln.x0))
        groups = [ordered]
    paras = []
    for group in groups:
        cur, prev = [], None
        heights = sorted(ln.y1 - ln.y0 for ln in group) or [10]
        typical = heights[len(heights) // 2] or 10
        for ln in group:
            if prev is not None and ln.y0 - prev.y1 > typical * 0.9:
                paras.append(" ".join(cur))
                cur = []
            cur.append(ln.text.strip())
            prev = ln
        if cur:
            paras.append(" ".join(cur))
    conf = sum(ln.score for ln in ordered) / len(ordered)
    return "\n\n".join(p for p in paras if p), conf
