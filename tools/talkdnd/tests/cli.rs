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
    ok(&r, &["chargen", "validate"]);
    assert!(!run(&r, &["chargen", "confirm"]).0, "confirm needs a sheet");
    ok(&r, &["chargen", "sheet"]);
    let out = ok(&r, &["chargen", "confirm"]);
    assert!(out.contains("character creation complete"));
    let sheet = fs::read_to_string(save.join("Characters/Aria.md")).unwrap();
    assert!(sheet.contains("ac: 18\n"));
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
