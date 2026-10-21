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
