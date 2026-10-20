from pdf2md import headings, repair, tables


def test_spacing_merge_and_protections():
    assert repair.repair_text("the com m on goblin") == "the common goblin"
    assert repair.repair_text("W isdom and Charisma") == "Wisdom and Charisma"
    assert repair.repair_text("in to the vault") == "in to the vault"


def test_letterspaced_heading():
    assert repair.repair_letterspaced("C h a p t e r 1") == "Chapter 1"
    assert headings.normalize_heading("## T r e a s u r e", {}) .endswith("Treasure")


def test_dice_and_numbers():
    assert repair.fix_dice_line("ld6 dlOO 2O 3d8") == "1d6 d100 20 3d8"
    assert repair.fix_line_structure("(+ 5) and 5 ,900 gp 10ft.") == "(+5) and 5,900 gp 10 ft."


def test_picture_text_and_structure():
    md = "a\n<!-- Start of picture text -->junk<!-- End of picture text -->\n~~~\n12\nb"
    md = repair.strip_picture_text(md)
    assert "junk" not in md
    assert repair.structural_pass(md.split("\n")) == ["a", "", "b"]


def test_digit_one_only_in_pronoun_context():
    assert repair.fix_digit_one("what 1 want") == "what I want"
    assert repair.fix_digit_one("when 1 hour passes") == "when 1 hour passes"


def test_tables_fixed_and_checked():
    md = "text\n|d6|Item|\n|---|---|\n||Rope|\n|2|Sword|\n|3|Shield|extra|\n"
    fixed = tables.fix_tables(md)
    assert "|1|Rope|" in fixed
    assert tables.check_tables(fixed) == []
    assert tables.check_tables("x\n|a|b|\n|---|---|\n|1|\n") != []
