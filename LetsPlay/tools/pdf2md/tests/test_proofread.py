"""Synthetic text only."""

from pdf2md import proofread

LEX = set("""the a of to in and character fire damage takes want ball you hit run by goddess
civilization roll gold feet speed""".split())


def fix(md):
    return proofread.proofread(md, LEX)


def test_num_and_one():
    out, log = fix("It takes Sd6 fire damage and ld8 more. Speed SO ft.\n\"1 want gold,\" she says.")
    assert "5d6" in out and "1d8" in out and "50 ft." in out
    assert '"I want gold,"' in out
    kinds = {e[0] for e in log}
    assert {"NUM", "ONE"} <= kinds


def test_split_glue_space():
    common = " ".join(["run by the goddess of civilization character too"] * 25)
    out, _ = fix(common + "\nA charac ter of Ysgard, runby the goddess of civilization,and ofYsgard  too .")
    assert "character" in out
    assert "of Ysgard" in out
    assert "run by" in out
    assert "civilization, and" in out
    assert "too." in out and "  " not in out


def test_markers_tables_headings_untouched():
    md = ('<a id="p3"></a>\n<!-- inferred: Sd6 -->\n## Charac ter Sd6\n| Sd6 | charac ter |\n'
          "See ![map,x](img/p0003-00.webp) and `Sd6` <!-- check PDF -->, then Sd6.")
    out, _ = fix(md)
    lines = out.split("\n")
    assert lines[:4] == md.split("\n")[:4]
    assert "![map,x](img/p0003-00.webp)" in out and "`Sd6`" in out and "<!-- check PDF -->" in out
    assert out.endswith("then 5d6.")


def test_report_counts():
    _, log = fix("Sd6 and ld6.")
    rep = proofread.report("Book", log)
    assert "| NUM | 1 |" in rep


def test_room_codes_and_map_captions():
    from pdf2md import headings

    assert headings.fix_room_code("Vl: GRAND ENTRANCE") == "V1: GRAND ENTRANCE"
    assert headings.fix_room_code("Vl4: VAULT") == "V14: VAULT"
    assert headings.fix_room_code("Vlad: hello") == "Vlad: hello"
    out = headings.promote_room_lines(["**Vl: GRAND ENTRANCE**", "MAP 1.2: DM'S MAP", "V1: the guard says hi."])
    assert out[0] == "#### V1: GRAND ENTRANCE"
    assert out[1] == "#### MAP 1.2: DM'S MAP"
    assert out[2] == "V1: the guard says hi."


def test_real_word_pairs_and_shouting_are_left_alone():
    lex = LEX | {"look", "up", "lookup", "go", "togo", "try", "ng", "vol", "ence"}
    md = "look up and to go now. They try,ng it. THE VOL.ENCE"
    out, _ = proofread.proofread(md, lex)
    assert out == md


def test_rooms_insert_and_fix_refresh():
    from pdf2md import fixes, ocr, rooms

    lines = [ocr.OcrLine(10, 100, 200, 112, "V5:CURATOR'S OFFICE", 0.9),
             ocr.OcrLine(10, 120, 300, 132, "A solid oak desk stands on a plush carpet.", 0.9),
             ocr.OcrLine(400, 100, 600, 112, "V9: OTHER ROOM", 0.9)]
    found = rooms.ocr_rooms(lines, 800)
    assert found == [("V5: CURATOR'S OFFICE", "A solid oak desk stands on a plush carpet.")]
    body = "#### V4: HALL\ntext\n\nA solid oak desk stands on a plush carpet in the centre.\n"
    out, n = rooms.add_missing_rooms(body, found)
    assert n == 1 and "#### V5: CURATOR'S OFFICE\n\nA solid oak desk" in out
    gen = '<a id="p3"></a>\n#### V5: CURATOR\'S OFFICE\nA solid oak desk here.\n![m](img/p0003-00.webp)\n'
    fix = "Corrected intro.\n\nA solid oak desk here, fixed.\n"
    md = '# B\n<a id="p3"></a>\n' + gen.split("\n", 1)[1]
    got = fixes.apply_fixes(md, {3: fix}, refresh=True)
    assert "Corrected intro." in got and "#### V5: CURATOR'S OFFICE" in got and "img/p0003-00.webp" in got


def test_readaloud_marking():
    from pdf2md import rooms

    boxed = rooms.norm("A winding staircase leads up to this cramped space filled with haphazardly stacked boxes.")
    body = "A winding staircase leads up to this cramped space filled with haphazardly stacked boxes.\n\nThe guards patrol here every five minutes and carry keys to the doors."
    out, n = rooms.mark_readaloud(body, boxed)
    assert n == 1 and out.startswith("> A winding") and "\n\nThe guards" in out


def test_room_codes_inline_and_area_refs():
    from pdf2md import rooms
    from pdf2md.headings import fix_room_code

    assert fix_room_code("VlO: ARCHAEOLOGY DISPLAY") == "V10: ARCHAEOLOGY DISPLAY"
    assert fix_room_code("VI3:GEMSTONE WING") == "V13: GEMSTONE WING"
    assert fix_room_code("AB3: HALL") == "AB3: HALL"
    body = "> V7: Boxes BREAK of supplies ROOM AND STORAGE l corners. Chairs here and there.\n"
    out, n = rooms.add_missing_rooms(body, [("V7: BREAK ROOMAND STORAGE", "can take an action")])
    assert n == 1 and out.startswith("#### V7: BREAK ROOMAND STORAGE\n\n> Boxes of supplies l corners.")
    md = "#### V1: HALL\n#### V10: ROOM\nGo to area Vl, or areas VlO and V1. Volume VI stays."
    assert rooms.fix_area_refs(md).endswith("area V1, or areas V10 and V1. Volume VI stays.")


def test_glue_multiword_and_hyphen_space():
    from pdf2md.proofread import Proofreader

    words = {"of", "the", "dark", "stone", "museum", "employee", "only", "door", "staff"}
    md = ("the dark stone of the museum " * 25 + " employee-only door. " +
          "staff only " * 6)
    pr = Proofreader(md, words)
    assert pr.glue_fix("ofthedarkstone") == "of the dark stone"
    assert pr.space_fix("an employee,only door") == "an employee-only door"
    assert pr.space_fix("an employee- only door") == "an employee-only door"
    assert pr.space_fix("staff,only") == "staff, only"


def test_letter_spaced_headings():
    import pymupdf

    from pdf2md import repair, spaced

    repair.WORDS.update({"domain", "ancients", "oath", "of", "the"})
    assert spaced._join_fragments("Trickery D omain Spells") == "Trickery Domain Spells"
    assert spaced._join_fragments("Oath of the A ncients") == "Oath of the Ancients"
    assert spaced._is_spaced("G e a r   S t r i k e") and not spaced._is_spaced("Gear Strike now")
    doc = pymupdf.open()
    page = doc.new_page()
    page.insert_text((50, 100), "G e a r   S t r i k e", fontsize=8)
    page.insert_text((50, 115), "Starting at 2nd level, you can strike with gears.", fontsize=8)
    body = "Intro text here.\n\nStarting at 2nd level, you can strike with gears.\n"
    out, n = spaced.restore(body, page)
    assert n == 1 and "#### Gear Strike\n\nStarting at 2nd" in out
