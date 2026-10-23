"""Small text helpers shared by the extractors.

A PDF line is made of spans that can split even a single word: drop caps,
mixed fonts, decorative first letters and OCR layers all produce
``exam`` + ``ple``. Every extractor must join the spans of a line *before*
splitting or counting words; joining later would count two words and break
headings, word counts and quality checks.
"""


def line_text(line):
    """Text of one PyMuPDF line: spans joined in order, no inserted spaces."""
    return "".join((span.get("text") or "") for span in line.get("spans", []))
