use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("talkdnd-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(p.join("Saves")).unwrap();
    fs::write(p.join("AGENTS.md"), "x").unwrap();
    p
}

fn run(root: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_talkdnd"))
        .arg("--root")
        .arg(root)
        .args(args)
        .env("TALKDND_NO_PDF2MD", "1")
        .env("TALKDND_COMBAT_SIM", root.join("no-such-combat-sim"))
        .output()
        .unwrap();
    let s =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    (out.status.success(), s)
}

fn ok(root: &Path, args: &[&str]) -> String {
    let (good, s) = run(root, args);
    assert!(good, "{args:?} failed: {s}");
    s
}

#[test]
fn full_creation_flow() {
    let r = root("flow");
    let out = ok(&r, &["new", "--date", "2030-01-02"]);
    assert!(out.contains("Saves/2030-01-02-01"));
    assert!(out.contains("next step: ruleset"));
    ok(&r, &["chargen", "set", "ruleset", "2024"]);
    ok(&r, &["chargen", "set", "adventure", "Test Adventure"]);
    assert!(r.join("Adventures/test-adventure/combat").is_dir());
    ok(&r, &["chargen", "set", "species", "human"]);
    ok(&r, &["chargen", "set", "class", "fighter"]);
    ok(&r, &["chargen", "set", "level", "1"]);
    ok(&r, &["chargen", "set", "background", "soldier"]);
    let rolled = ok(&r, &["--seed", "5", "chargen", "roll"]);
    assert_eq!(rolled.matches("4d6dl1(").count(), 6);
    // Rolls are final.
    assert!(!run(&r, &["chargen", "roll"]).0);
    assert!(!run(&r, &["chargen", "array"]).0);
    let save = r.join("Saves/2030-01-02-01");
    let state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(save.join("chargen.json")).unwrap()).unwrap();
    let pool: Vec<i64> = state["pool"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    let mut sorted = pool.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    let names = ["str", "con", "dex", "wis", "int", "cha"];
    let assign: Vec<String> = names
        .iter()
        .zip(&sorted)
        .map(|(a, v)| format!("{a}={v}"))
        .collect();
    let mut args = vec!["chargen", "assign"];
    args.extend(assign.iter().map(|s| s.as_str()));
    ok(&r, &args);
    ok(&r, &["chargen", "asi", "str=2", "con=1"]);
    ok(
        &r,
        &[
            "chargen",
            "set",
            "skills",
            "athletics,intimidation,perception,survival,history",
        ],
    );
    ok(&r, &["chargen", "set", "armor", "chain-mail"]);
    ok(&r, &["chargen", "set", "shield", "yes"]);
    ok(&r, &["chargen", "set", "spells", "none"]);
    ok(&r, &["chargen", "set", "name", "Aria"]);
    // Level-1 fighter needs a Fighting Style.
    let (good, out) = run(&r, &["chargen", "validate"]);
    assert!(!good && out.contains("needs a Fighting Style"), "{out}");
    ok(&r, &["chargen", "set", "fighting-style", "defense"]);
    ok(&r, &["chargen", "set", "background-equipment", "B"]);
    ok(&r, &["chargen", "set", "languages", "none"]);
    for f in ["traits", "ideals", "bonds", "flaws"] {
        ok(&r, &["chargen", "set", f, "由玩家决定"]);
    }
    ok(&r, &["chargen", "validate"]);
    assert!(!run(&r, &["chargen", "confirm"]).0, "confirm needs a sheet");
    ok(&r, &["chargen", "sheet"]);
    let out = ok(&r, &["chargen", "confirm"]);
    assert!(out.contains("character creation complete"));
    assert!(out.contains("50 gp"), "background B gold: {out}");
    let sheet = fs::read_to_string(save.join("Characters/Aria.md")).unwrap();
    assert!(
        sheet.contains("ac: 19\n"),
        "chain mail + shield + Defense: {sheet}"
    );
    assert!(save.join("sheets/aria.yaml").is_file());
    assert!(sheet.contains("Fighting Style: Defense"));
    // Sheet changes go through `char`, logged and integrity-clean.
    let out = ok(
        &r,
        &["char", "Aria", "batch", "damage 5; temp 3; item add 20 arrows; ammo arrows -2; gold spend --gp 10; condition add prone; exhaustion +1; inspiration on"],
    );
    assert!(out.contains("Aria HP 7/12 (+3 临时)"), "{out}");
    assert!(out.contains("力竭 1"), "{out}");
    let out = ok(&r, &["char", "Aria", "rest", "long"]);
    assert!(out.contains("HP 12/12") && !out.contains("力竭"), "{out}");
    let ps = fs::read_to_string(save.join("combat/party-state.yaml")).unwrap();
    assert!(ps.contains("aria:") && ps.contains("hp: 12"), "{ps}");
    let out = ok(&r, &["check"]);
    assert!(out.contains("integrity: OK"), "{out}");
    let log = fs::read_to_string(save.join("Sessions/2030-01-02-session-00-setup.md")).unwrap();
    assert!(log.contains("角色 Aria：受到 5 伤害"), "{log}");
    // A failing op in a batch writes nothing.
    let before = fs::read_to_string(save.join("sheets/aria.yaml")).unwrap();
    assert!(
        !run(
            &r,
            &[
                "char",
                "Aria",
                "batch",
                "item add rope; use no-such-resource"
            ]
        )
        .0
    );
    assert_eq!(
        before,
        fs::read_to_string(save.join("sheets/aria.yaml")).unwrap()
    );
    let manifest = fs::read_to_string(save.join("combat/campaign.yaml")).unwrap();
    assert!(manifest.contains("party: [aria]"));
    assert!(manifest.contains("ruleset: \"2024\""));
    let log = fs::read_to_string(save.join("Sessions/2030-01-02-session-00-setup.md")).unwrap();
    assert!(log.contains("属性掷骰 6×4d6dl1 (seeded test roll)"));

    // Companions.
    let out = ok(&r, &["--seed", "9", "npc", "random", "--count", "3"]);
    assert_eq!(out.matches("draft ").count(), 3);
    ok(&r, &["npc", "sheet", "2", "--name", "Kel"]);
    assert!(fs::read_to_string(save.join("NPCs/Kel.md"))
        .unwrap()
        .contains("kind: npc"));
    assert!(fs::read_to_string(save.join("combat/campaign.yaml"))
        .unwrap()
        .contains("party: [aria, kel]"));

    // Log, checkpoint, start.
    ok(&r, &["log", "进入酒馆"]);
    ok(&r, &["checkpoint", "时间：第 1 天 08:00；Aria HP 12/12"]);
    let d = ok(&r, &["start"]);
    assert!(d.contains("规则：**2024**"));
    assert!(d.contains("Aria HP 12/12"));
    assert!(d.contains("`kel`"));
    assert!(d.contains("combat-sim") || d.contains("missing"));
    assert!(save.join("digest.md").is_file());
    assert!(d.len() < 4096);
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn bad_assignment_rejected() {
    let r = root("bad");
    ok(&r, &["new", "--ruleset", "2014", "--date", "2030-01-02"]);
    ok(&r, &["chargen", "array"]);
    ok(
        &r,
        &[
            "chargen",
            "assign",
            "str=15,dex=15,con=13,int=12,wis=10,cha=8",
        ],
    );
    let (good, out) = run(&r, &["chargen", "validate"]);
    assert!(!good);
    assert!(out.contains("not the array"));
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn newest_slot_and_numbering() {
    let r = root("slots");
    ok(&r, &["new", "--ruleset", "2014", "--date", "2030-01-02"]);
    ok(&r, &["new", "--ruleset", "2024", "--date", "2030-01-02"]);
    assert!(r.join("Saves/2030-01-02-02").is_dir());
    let d = ok(&r, &["start"]);
    assert!(d.contains("2030-01-02-02"));
    assert!(d.contains("建卡未完成"));
    let d = ok(&r, &["--save", "2030-01-02-01", "start"]);
    assert!(d.contains("**2014**"));
    assert!(!run(&r, &["--save", "1999-01-01-01", "start"]).0);
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn checkpoint_section_is_extracted() {
    let text = "# log\n## 存档点（1）\nold\n## 场景\nx\n## 存档点（2）\nnew line\n- hp\n";
    assert_eq!(
        talkdnd::save::last_checkpoint(text).unwrap(),
        "## 存档点（2）\nnew line\n- hp"
    );
}

#[test]
fn integrity_reconcile_and_reveal() {
    let r = root("integrity");
    ok(&r, &["new", "--date", "2030-02-03", "--ruleset", "2014"]);
    let save = r.join("Saves/2030-02-03-01");
    assert!(save.join("integrity.yaml").is_file());
    assert!(ok(&r, &["check"]).contains("integrity: OK"));
    // tool writes stay clean
    ok(&r, &["loot", "add", "--gp", "5", "rope", "--to", "party"]);
    assert!(ok(&r, &["check"]).contains("integrity: OK"));
    // a hand edit is flagged and reconciled against the log
    let p = save.join("progress.yaml");
    let t = fs::read_to_string(&p).unwrap();
    fs::write(&p, t.replace("cp: 500", "cp: 900000")).unwrap();
    let out = ok(&r, &["check"]);
    assert!(
        out.contains("progress.yaml: changed outside the tools"),
        "{out}"
    );
    assert!(out.contains("coins of `party`"), "{out}");
    // a later tool write does not launder it
    ok(&r, &["loot", "add", "--gp", "1", "--to", "party"]);
    assert!(ok(&r, &["integrity", "status"]).contains("progress.yaml"));
    ok(&r, &["integrity", "accept", "--reason", "test fix"]);
    assert!(ok(&r, &["check"]).contains("integrity: OK"));
    // reveal ledger
    ok(
        &r,
        &[
            "reveal", "add", "the", "vault", "is", "below", "--source", "map",
        ],
    );
    assert!(ok(&r, &["reveal", "check", "vault"]).contains("#1"));
    assert!(ok(&r, &["reveal", "check", "dragon"]).contains("NOT REVEALED"));
    assert!(ok(&r, &["check"]).contains("integrity: OK"));
    // old save without a manifest keeps working
    fs::remove_file(save.join("integrity.yaml")).unwrap();
    assert!(ok(&r, &["check"]).contains("created a baseline"));
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn old_sheet_repair_without_chargen() {
    let r = root("repair");
    ok(&r, &["new", "--date", "2030-03-04", "--ruleset", "2014"]);
    let save = r.join("Saves/2030-03-04-01");
    fs::create_dir_all(save.join("Characters")).unwrap();
    fs::write(
        save.join("Characters/Gar.md"),
        "# Gar——角色卡\n\n- **等级**：战士 2\n- 护甲：链甲（chain mail）\n\n## 战斗数据块（combat-sim）\n\n```combat yaml\nid: gar\nname: Gar\nkind: pc\nside: party\nsize: medium\nlevel: 2\nac: 16\nhp: 20\nspeed: {walk: 30}\nprof: 2\nabilities: {str: 16, dex: 10, con: 14, int: 10, wis: 10, cha: 12}\nsaves: {str: 5, con: 4}\nskills: {athletics: 5}\nattacks:\n  - {id: greatsword, name: Greatsword, kind: melee, ability: str, to_hit: 5, damage: [{dice: \"2d6+3\", type: slashing}], reach: 5, properties: [heavy, two-handed]}\nresources:\n  hit-dice: {max: 2, recharge: long, die: 10}\nnotes: \"fighter 2\"\n```\n",
    )
    .unwrap();
    ok(&r, &["integrity", "accept", "--reason", "test sheet"]);
    // A homebrew background in local data (stands in for a book import).
    fs::create_dir_all(r.join("Rules/chargen/local")).unwrap();
    fs::write(
        r.join("Rules/chargen/local/manual-2014.yaml"),
        "ruleset: \"2014\"\nbackgrounds:\n  test-heir:\n    name: Test Heir\n    feature: Test Privilege\n    tools: [dice set]\n    languages: 1\n    equipment: [fine clothes, signet ring]\n    gold: 25\n",
    )
    .unwrap();
    let out = ok(
        &r,
        &[
            "char",
            "gar",
            "set",
            "fighting-style",
            "great-weapon-fighting",
        ],
    );
    assert!(out.contains("migrated"), "{out}");
    let out = ok(&r, &["char", "gar", "set", "background", "test-heir"]);
    assert!(out.contains("25 gp"), "{out}");
    let sheet = fs::read_to_string(save.join("Characters/Gar.md")).unwrap();
    assert!(sheet.contains("reroll_damage_below: 2"), "{sheet}");
    assert!(sheet.contains("- 护甲：链甲"), "body kept");
    assert!(sheet.contains("signet ring") && sheet.contains("背景特性：Test Privilege"));
    assert!(ok(&r, &["loot", "show"]).contains("gar: 25 gp"));
    // Defense counts the migrated armor.
    ok(&r, &["char", "gar", "set", "fighting-style", "defense"]);
    assert!(fs::read_to_string(save.join("Characters/Gar.md"))
        .unwrap()
        .contains("ac: 17\n"));
    // level-up edits the rendered block; char keeps it.
    ok(
        &r,
        &["level-up", "gar", "--milestone", "--reason", "test arc"],
    );
    let out = ok(&r, &["char", "gar", "damage", "3"]);
    assert!(out.contains("HP 25/28"), "{out}");
    let y = fs::read_to_string(save.join("sheets/gar.yaml")).unwrap();
    assert!(y.contains("level: 3"), "{y}");
    assert!(fs::read_to_string(save.join("Characters/Gar.md"))
        .unwrap()
        .contains("## 升级记录"));
    assert!(ok(&r, &["check"]).contains("integrity: OK"));
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn handouts_player_safe_only() {
    let r = root("handout");
    ok(&r, &["new", "--date", "2030-05-06", "--ruleset", "2014"]);
    ok(&r, &["chargen", "set", "adventure", "Made Up Quest"]);
    let save = r.join("Saves/2030-05-06-01");
    // A made-up converted book: two generated images, no book content.
    let book = r.join("Adventures/made-up-quest/.md/Made Up Quest");
    fs::create_dir_all(book.join("img")).unwrap();
    for (n, c) in [("p0003-00", [200u8, 30, 30]), ("p0007-00", [30, 30, 200])] {
        image::RgbImage::from_pixel(40, 20, image::Rgb(c))
            .save(book.join(format!("img/{n}.png")))
            .unwrap();
    }
    fs::write(
        book.join("index.json"),
        r#"{"headings":[{"level":2,"text":"Fake Village","page":3},{"level":2,"text":"Fake Crypt","page":7}],
        "maps":[{"caption":"MAP 1.1: Fake Village (Player Version)","img":"img/p0003-00.png","page":3},
                {"caption":"MAP 1.2: Fake Crypt","img":"img/p0007-00.png","page":7}],"figures":[]}"#,
    )
    .unwrap();
    let out = ok(&r, &["handout", "build"]);
    assert!(
        out.contains("2 images") && out.contains("1 player-safe"),
        "{out}"
    );
    // DM-only maps are refused.
    let (good, out) = run(&r, &["handout", "show", "p0007-00"]);
    assert!(!good && out.contains("DM-only"), "{out}");
    assert!(!save.join("handouts/p0007-00.png").exists());
    assert!(ok(&r, &["handout", "list"]).contains("no handouts"));
    // Player-safe map by search.
    let out = ok(&r, &["handout", "show", "fake", "village"]);
    assert!(out.contains("markdown: ![MAP 1.1"), "{out}");
    assert!(save.join("handouts/p0003-00.png").is_file());
    assert!(ok(&r, &["reveal", "list"]).contains("资料：MAP 1.1"));
    assert!(ok(&r, &["handout", "list"]).contains("p0003-00"));
    // Only a region of the crypt map is player-safe.
    ok(
        &r,
        &[
            "handout",
            "mark",
            "p0007-00",
            "--player",
            "--crop",
            "0,0,0.5,1",
            "--note",
            "entrance only",
        ],
    );
    let (good, out) = run(&r, &["handout", "show", "p0007-00", "--crop", "0,0,1,1"]);
    assert!(!good && out.contains("registered crop"), "{out}");
    ok(&r, &["handout", "show", "p0007-00"]);
    let img = image::open(save.join("handouts/p0007-00-crop.png")).unwrap();
    assert_eq!((img.width(), img.height()), (20, 20));
    // Rebuild keeps the marking.
    ok(&r, &["handout", "build"]);
    assert!(ok(&r, &["handout", "list", "--dm"]).contains("p0007-00 [player]"));
    let log = fs::read_dir(save.join("Sessions"))
        .unwrap()
        .flatten()
        .map(|e| fs::read_to_string(e.path()).unwrap_or_default())
        .collect::<String>();
    assert!(log.contains("资料：MAP 1.1"), "{log}");
    let _ = fs::remove_dir_all(&r);
}
