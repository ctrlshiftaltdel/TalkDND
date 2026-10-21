from pdf2md import statblocks as sb

GOOD14 = """<a id="p5"></a>
### GOBLIN BOSS

*Small Humanoid (Goblinoid), Neutral Evil*

**Armor Class** 17 (chain shirt, shield)
**Hit Points** 21 (6d6)
**Speed** 30 ft.

| STR | DEX | CON | INT | WIS | CHA |
| --- | --- | --- | --- | --- | --- |
| 10 (+0) | 14 (+2) | 10 (+0) | 10 (+0) | 8 (−1) | 10 (+0) |

**Challenge** 1 (200 XP) **Proficiency Bonus** +2
"""

BAD24 = """<a id="p9"></a>
### ANIMATED ARMOR

Medium Construct, Unaligned

AC 18 Initiative +2 (12) HP 33 (6d8 + 5)

MOD SAVE Srn 14 +2 +2 Dex 11 +0 +0 Con 13 +1 +1 Int 1 -5 -5 Wis 3 -4 -4 Cha 1 -5 -5

CR 1 (XP 200; PB +2)
"""


def test_parse_2014_and_2024_layouts():
    b = sb.parse(GOOD14)
    assert (b["ac"], b["hp"], b["hd"], b["hp_bonus"], b["cr"], b["xp"], b["pb"]) == (17, 21, (6, 6), 0, "1", 200, 2)
    assert b["abilities"][4] == (8, -1)
    assert sb.check(b) == ("ok", [])
    b = sb.parse(BAD24)
    assert b["abilities"][0] == (14, 2) and b["cr"] == "1" and b["pb"] == 2
    status, issues = sb.check(b)
    assert status == "fail" and any("average 32" in i for i in issues), issues


def test_annotate_marks_failures_only_and_is_idempotent():
    md = GOOD14 + "\n" + BAD24 + "\nPlain prose: the AC 15 of a guard matters.\n"
    out, res = sb.annotate(md)
    assert [r["status"] for r in res] == ["ok", "fail"]
    assert res[1]["name"] == "Animated Armor" and res[1]["page"] == 9
    assert out.count(sb.MARK) == 1 and "check PDF p.9." in out
    assert out.index(sb.MARK) < out.index("AC 18 Initiative")
    again, _ = sb.annotate(out)
    assert again == out


def test_incomplete_and_cross_check(tmp_path):
    out, res = sb.annotate("### Thing\n\n**Armor Class** 12\n**Hit Points** 9 (2d8)\n")
    assert res[0]["status"] == "incomplete" and "abilities" in res[0]["issues"][0]
    (tmp_path / "c.yaml").write_text("creatures:\n  - {id: goblin-boss, name: Goblin Boss, ac: 15, hp: 21}\n")
    known = sb.load_known([str(tmp_path)])
    _, res = sb.annotate(GOOD14, known)
    assert res[0]["status"] == "fail" and "AC 17 ≠ combat data 15" in res[0]["issues"]


def test_cr_tables():
    assert sb.pb_for_cr("1/4") == 2 and sb.pb_for_cr("5") == 3 and sb.pb_for_cr("17") == 6 and sb.pb_for_cr("30") == 9
    b = sb.parse(GOOD14.replace("(200 XP)", "(450 XP)"))
    assert any("200 XP, not 450" in i for i in sb.check(b)[1])
