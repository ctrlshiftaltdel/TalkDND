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
    let sheet = fs::read_to_string(save.join("Characters/aria.md")).unwrap();
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
    assert!(fs::read_to_string(save.join("NPCs/kel.md"))
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
fn deviation_is_validated_locked_and_tamper_checked() {
    let r = root("deviation");
    for args in [
        vec![
            "new",
            "--date",
            "2030-01-03",
            "--scope",
            "3",
            "--ratio",
            "10",
        ],
        vec![
            "new",
            "--date",
            "2030-01-03",
            "--scope",
            "0",
            "--ratio",
            "10",
        ],
        vec![
            "new",
            "--date",
            "2030-01-03",
            "--scope",
            "1",
            "--ratio",
            "4",
        ],
        vec![
            "new",
            "--date",
            "2030-01-03",
            "--scope",
            "1",
            "--ratio",
            "26",
        ],
        vec!["new", "--date", "2030-01-03", "--scope", "1"],
    ] {
        let (good, out) = run(&r, &args);
        assert!(!good, "{args:?} accepted: {out}");
        assert!(
            out.contains("scope") || out.contains("ratio"),
            "{args:?}: {out}"
        );
    }
    assert_eq!(fs::read_dir(r.join("Saves")).unwrap().count(), 0);
    ok(
        &r,
        &[
            "new",
            "--date",
            "2030-01-03",
            "--scope",
            "2",
            "--ratio",
            "15",
            "--ruleset",
            "2014",
        ],
    );
    let save = r.join("Saves/2030-01-03-01");
    let meta = fs::read_to_string(save.join("save-meta.yaml")).unwrap();
    assert!(
        meta.contains("schema: 5") && meta.contains("scope: 2") && meta.contains("ratio: 15"),
        "{meta}"
    );
    let out = ok(&r, &["original", "status"]);
    for want in [
        "scope 2",
        "比例上限 15%",
        "允许原创类型",
        "稀有度上限",
        "本章原创",
        "全战役原创",
    ] {
        assert!(out.contains(want), "missing {want}: {out}");
    }
    let (good, out) = run(
        &r,
        &[
            "original",
            "add",
            "--type",
            "sidequest",
            "--name",
            "Test",
            "--rarity",
            "uncommon",
        ],
    );
    assert!(
        !good && out.contains("预算") && out.contains("回到主线"),
        "{out}"
    );
    // No setter: editing save-meta by hand is flagged by `check`.
    fs::write(
        save.join("save-meta.yaml"),
        meta.replace("scope: 2", "scope: 1"),
    )
    .unwrap();
    let out = ok(&r, &["check"]);
    assert!(
        out.contains("save-meta.yaml: changed outside the tools"),
        "{out}"
    );
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

fn v1_save(r: &Path) -> PathBuf {
    let save = r.join("Saves/2030-06-07-01");
    for dir in ["Characters", "NPCs", "Sessions", "combat"] {
        fs::create_dir_all(save.join(dir)).unwrap();
    }
    fs::write(
        save.join("combat/campaign.yaml"),
        "name: X\nruleset: \"2024\"\nparty: []\n",
    )
    .unwrap();
    fs::write(
        save.join("Characters/hero.md"),
        "# Hero\n\n- **等级**：战士 2\n\n```combat yaml\nid: hero\nname: Hero\nkind: pc\nside: party\nlevel: 2\nac: 16\nhp: 20\nspeed: {walk: 30}\nprof: 2\nabilities: {str: 16, dex: 10, con: 14, int: 10, wis: 10, cha: 12}\nnotes: \"fighter 2\"\n```\n",
    )
    .unwrap();
    fs::write(
        save.join("Sessions/2030-06-07-session-00-setup.md"),
        "# log\n",
    )
    .unwrap();
    fs::write(save.join("progress.yaml"), "xp: {hero: 0}\n").unwrap();
    save
}

#[test]
fn start_migrates_a_v1_save_once_and_checks_clean() {
    let r = root("migrate");
    let save = v1_save(&r);
    let out = ok(&r, &["start"]);
    assert!(out.contains("存档结构"), "{out}");
    assert!(
        out.contains("schema 1 → 5") || out.contains("schema 1→5"),
        "{out}"
    );
    assert!(save.join("sheets/hero.yaml").is_file());
    let meta = fs::read_to_string(save.join("save-meta.yaml")).unwrap();
    assert!(
        meta.contains("schema: 5") && meta.contains("scope: 0"),
        "{meta}"
    );
    let backups: Vec<PathBuf> = fs::read_dir(r.join("Saves/_backups"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(backups.len(), 1);
    assert!(backups[0].join("Characters/hero.md").is_file());
    let log = fs::read_to_string(save.join("Sessions/2030-06-07-session-00-setup.md")).unwrap();
    assert!(log.contains("存档结构迁移 schema 1→5"), "{log}");
    assert!(ok(&r, &["check"]).contains("integrity: OK"));
    assert!(ok(&r, &["start"]).contains("无需迁移"));
    assert_eq!(fs::read_dir(r.join("Saves/_backups")).unwrap().count(), 1);
    let after = fs::read_to_string(save.join("Sessions/2030-06-07-session-00-setup.md")).unwrap();
    assert_eq!(after.lines().count(), log.lines().count());
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn a_save_schema_newer_than_talkdnd_is_refused() {
    let r = root("schema99");
    ok(&r, &["new", "--date", "2030-06-08"]);
    let save = r.join("Saves/2030-06-08-01");
    fs::write(save.join("save-meta.yaml"), "schema: 99\ntool: future\n").unwrap();
    let (good, out) = run(&r, &["start"]);
    assert!(
        !good && out.contains("99") && out.contains("update talkdnd"),
        "{out}"
    );
    assert!(!r.join("Saves/_backups").exists());
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn migrate_command_dry_run_writes_nothing_then_applies_once() {
    let r = root("migratecmd");
    let save = v1_save(&r);
    let out = ok(&r, &["migrate", "--dry-run"]);
    assert!(out.contains("dry-run"), "{out}");
    assert!(!save.join("save-meta.yaml").exists());
    assert!(!save.join("sheets/hero.yaml").exists());
    let out = ok(&r, &["migrate"]);
    assert!(out.contains("迁移完成"), "{out}");
    assert!(save.join("save-meta.yaml").is_file());
    assert!(save.join("sheets/hero.yaml").is_file());
    assert!(ok(&r, &["migrate"]).contains("无需迁移"));
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn migration_does_not_launder_a_hand_edit() {
    let r = root("mighand");
    ok(&r, &["new", "--date", "2030-06-09"]);
    let save = r.join("Saves/2030-06-09-01");
    ok(&r, &["loot", "add", "--gp", "5", "rope", "--to", "party"]);
    assert!(ok(&r, &["check"]).contains("integrity: OK"));
    fs::remove_file(save.join("save-meta.yaml")).unwrap();
    let p = save.join("progress.yaml");
    let text = fs::read_to_string(&p)
        .unwrap()
        .replace("cp: 500", "cp: 900000");
    fs::write(&p, text).unwrap();
    let out = ok(&r, &["start"]);
    assert!(
        out.contains("schema 2 → 5") || out.contains("schema 2→5"),
        "{out}"
    );
    let out = ok(&r, &["check"]);
    assert!(
        out.contains("progress.yaml: changed outside the tools"),
        "{out}"
    );
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn rulings_defaults_are_copied_once_and_never_overwrite_the_dm_copy() {
    let r = root("defaults");
    fs::create_dir_all(r.join("Rules/defaults")).unwrap();
    fs::write(r.join("Rules/defaults/rulings.md"), "默认一\n").unwrap();
    ok(&r, &["new", "--date", "2030-06-10"]);
    assert_eq!(
        fs::read_to_string(r.join("Rules/rulings.md")).unwrap(),
        "默认一\n"
    );
    fs::write(r.join("Rules/rulings.md"), "我的裁定\n").unwrap();
    fs::write(r.join("Rules/defaults/rulings.md"), "默认二\n").unwrap();
    let out = ok(&r, &["start"]);
    assert!(out.contains("请手动合并"), "{out}");
    assert_eq!(
        fs::read_to_string(r.join("Rules/rulings.md")).unwrap(),
        "我的裁定\n"
    );
    assert!(r.join(".talkdnd/state.yaml").is_file());
    let out = ok(&r, &["start"]);
    assert!(!out.contains("请手动合并"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn start_moves_obsolete_shipped_files_but_keeps_user_data() {
    let r = root("obsolete");
    ok(&r, &["new", "--date", "2030-06-12"]);
    fs::write(r.join("old-shipped.md"), "old body\n").unwrap();
    fs::write(r.join("Saves/keep.md"), "save body\n").unwrap();
    fs::create_dir_all(r.join(".talkdnd")).unwrap();
    let old_sha = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"old body\n");
        format!("{:x}", h.finalize())
    };
    fs::write(
        r.join(".talkdnd/installed-files.txt"),
        format!("{old_sha}  old-shipped.md\n"),
    )
    .unwrap();
    fs::write(r.join(".talkdnd-files.txt"), "00  README.md\n").unwrap();
    let out = ok(&r, &["start"]);
    assert!(!r.join("old-shipped.md").exists());
    assert!(r.join("Saves/keep.md").is_file());
    assert!(out.contains("旧版本文件已移走"), "{out}");
    assert!(fs::read_dir(r.join(".talkdnd/obsolete"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("old-shipped.md")
        .is_file());
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn a_newer_integrity_manifest_is_refused() {
    let r = root("man99");
    ok(&r, &["new", "--date", "2030-06-11"]);
    let save = r.join("Saves/2030-06-11-01");
    let p = save.join("integrity.yaml");
    let text = fs::read_to_string(&p)
        .unwrap()
        .replacen("version: 1", "version: 99", 1);
    fs::write(&p, text).unwrap();
    let (good, out) = run(&r, &["check"]);
    assert!(
        !good && out.contains("integrity.yaml") && out.contains("99"),
        "{out}"
    );
    let _ = fs::remove_dir_all(&r);
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
        save.join("Characters/gar.md"),
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
    let sheet = fs::read_to_string(save.join("Characters/gar.md")).unwrap();
    assert!(sheet.contains("reroll_damage_below: 2"), "{sheet}");
    assert!(sheet.contains("- 护甲：链甲"), "body kept");
    assert!(sheet.contains("signet ring") && sheet.contains("背景特性：Test Privilege"));
    assert!(ok(&r, &["loot", "show"]).contains("gar: 25 gp"));
    // Defense counts the migrated armor.
    ok(&r, &["char", "gar", "set", "fighting-style", "defense"]);
    assert!(fs::read_to_string(save.join("Characters/gar.md"))
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
    assert!(fs::read_to_string(save.join("Characters/gar.md"))
        .unwrap()
        .contains("## 升级记录"));
    // HP transitions carry the campaign's dying condition (fallback here).
    let out = ok(&r, &["char", "gar", "damage", "28"]);
    assert!(out.contains("倒地"), "{out}");
    let party = fs::read_to_string(save.join("combat/party-state.yaml")).unwrap();
    assert!(party.contains("unconscious"), "{party}");
    ok(&r, &["char", "gar", "heal", "5"]);
    let party = fs::read_to_string(save.join("combat/party-state.yaml")).unwrap();
    assert!(!party.contains("unconscious"), "{party}");
    assert!(ok(&r, &["check"]).contains("integrity: OK"));
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn original_item_add_requires_canon_or_a_ledger_entry() {
    let r = root("origitem");
    ok(
        &r,
        &[
            "new",
            "--date",
            "2030-07-01",
            "--scope",
            "2",
            "--ratio",
            "25",
            "--ruleset",
            "2014",
        ],
    );
    let save = r.join("Saves/2030-07-01-01");
    fs::write(
        save.join("Characters/hero.md"),
        "# Hero\n\n```combat yaml\nid: hero\nname: Hero\nkind: pc\nside: party\nlevel: 1\nac: 12\nhp: 10\nprof: 2\nabilities: {str: 14, dex: 12, con: 12, int: 10, wis: 10, cha: 10}\nnotes: \"fighter 1\"\n```\n",
    )
    .unwrap();
    ok(&r, &["char", "hero", "item", "add", "longsword"]);
    let (good, out) = run(&r, &["char", "hero", "item", "add", "vorpal sword"]);
    assert!(
        !good && out.contains("--canon") && out.contains("--original"),
        "{out}"
    );
    ok(
        &r,
        &["char", "hero", "item", "add", "vorpal sword", "--canon"],
    );
    for reason in ["one", "two", "three"] {
        ok(&r, &["xp", "add", "10", "--to", "hero", "--reason", reason]);
    }
    let out = ok(
        &r,
        &[
            "original",
            "add",
            "--type",
            "item",
            "--name",
            "Moonblade",
            "--rarity",
            "uncommon",
            "--hook",
            "player",
        ],
    );
    assert!(out.contains("O-001") && out.contains("玩家钩子"), "{out}");
    ok(
        &r,
        &[
            "char",
            "hero",
            "item",
            "add",
            "Moonblade",
            "--original",
            "O-001",
        ],
    );
    let sheet = fs::read_to_string(save.join("sheets/hero.yaml")).unwrap();
    assert!(sheet.contains("original: O-001"), "{sheet}");
    let rendered = fs::read_to_string(save.join("Characters/hero.md")).unwrap();
    assert!(!rendered.contains("O-001"), "{rendered}");
    let (good, out) = run(
        &r,
        &[
            "char",
            "hero",
            "item",
            "add",
            "Other",
            "--original",
            "O-999",
        ],
    );
    assert!(!good && out.contains("O-999"), "{out}");
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

fn run_input(root: &Path, args: &[&str], input: &str) -> (bool, String) {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_talkdnd"))
        .arg("--root")
        .arg(root)
        .args(args)
        .env("TALKDND_NO_PDF2MD", "1")
        .env("TALKDND_COMBAT_SIM", root.join("no-such-combat-sim"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let s =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    (out.status.success(), s)
}

#[test]
fn guard_secret_known_and_dice_settings() {
    let r = root("guard");
    ok(&r, &["new", "--date", "2030-07-01", "--scope", "0"]);
    let save = r.join("Saves/2030-07-01-01");
    assert!(ok(&r, &["settings"]).contains("骰子显示：开"));
    assert!(!ok(&r, &["settings", "dice", "off"]).contains("开"));
    assert!(ok(&r, &["settings"]).contains("骰子显示：关"));
    let log = fs::read_to_string(save.join("Sessions/2030-07-01-session-00-setup.md")).unwrap();
    assert!(log.contains("骰子显示：关"), "{log}");
    ok(&r, &["scene", "舞厅"]);
    assert!(ok(&r, &["known"]).contains("当前场景：舞厅"));
    ok(&r, &["secret", "add", "Red Mask", "--chapter", "2"]);
    assert!(ok(&r, &["secret", "list"]).contains("Red Mask"));
    let (good, out) = run(&r, &["guard", "The Red Mask waits"]);
    assert!(!good, "guard should exit 1: {out}");
    assert!(out.contains("剧透词：Red Mask"), "{out}");
    let (good, out) = run(&r, &["guard", "A quiet street"]);
    assert!(good && out.contains("clean"), "{out}");
    let (good, out) = run_input(&r, &["guard", "-"], "Meet Red Mask");
    assert!(!good && out.contains("剧透词"), "{out}");
    let (good, out) = run_input(&r, &["guard"], "Meet Red Mask");
    assert!(!good && out.contains("剧透词"), "{out}");
    assert!(ok(&r, &["log", "rolls"]).contains("还没有掷骰记录"));
    ok(&r, &["secret", "remove", "red mask"]);
    assert!(run(&r, &["guard", "The Red Mask waits"]).0);
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn adventure_search_returns_pdf_pages_fuzzily() {
    let r = root("advsearch");
    ok(
        &r,
        &[
            "new",
            "--date",
            "2030-08-02",
            "--scope",
            "0",
            "--adventure",
            "Test Book",
        ],
    );
    let book = r.join("Adventures/test-book/.md/Test Book");
    fs::create_dir_all(&book).unwrap();
    fs::write(
        book.join("Test Book.pages.md"),
        "# Test Book\n\n<a id=\"p4\"></a>\n<!-- PDF p.4 -->\n..mown to the hero’s ﬁnal exam-\nple.\n",
    )
    .unwrap();
    let out = ok(&r, &["adventure", "search", "known"]);
    assert!(out.contains("p.4:"), "{out}");
    let out = ok(&r, &["adventure", "search", "hero's final example"]);
    assert!(out.contains("p.4:") && out.contains("example"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn adventure_section_refuses_ahead_and_logs_with_reason() {
    let r = root("adventure");
    ok(
        &r,
        &[
            "new",
            "--date",
            "2030-08-01",
            "--scope",
            "0",
            "--adventure",
            "Test Book",
        ],
    );
    let save = r.join("Saves/2030-08-01-01");
    fs::write(
        save.join("progress.yaml"),
        "version: 1\ntrack:\n  - {name: First Job, level: 1}\n  - {name: Second Job, level: 3}\n",
    )
    .unwrap();
    let book = r.join("Adventures/test-book/.md/Test Book");
    fs::create_dir_all(&book).unwrap();
    fs::write(
        book.join("Test Book.md"),
        "# Test Book\n\nintro\n\n## Chapter 1: First Job\n\nfirst body\n\n## Chapter 2: Second Job\n\nsecond body\n\n## DM Notes\n\nA secret [[Red Mask]]\n",
    )
    .unwrap();
    let list = ok(&r, &["adventure", "section", "--list"]);
    assert!(
        list.contains("Chapter 2: Second Job") && !list.contains("second body"),
        "{list}"
    );
    let (good, out) = run(&r, &["adventure", "section", "Second Job"]);
    assert!(!good && out.contains("--reason"), "{out}");
    let (good, out) = run(
        &r,
        &[
            "adventure",
            "section",
            "Second Job",
            "--reason",
            "player skipped ahead",
        ],
    );
    assert!(
        good && out.contains("second body") && out.contains("超前阅读"),
        "{out}"
    );
    let log = fs::read_to_string(save.join("Sessions/2030-08-01-session-00-setup.md")).unwrap();
    assert!(
        log.contains("超前阅读") && log.contains("player skipped ahead"),
        "{log}"
    );
    let (good, out) = run(&r, &["adventure", "section", "First Job"]);
    assert!(
        good && out.contains("first body") && !out.contains("second body"),
        "{out}"
    );
    let _ = fs::remove_dir_all(&r);
}
