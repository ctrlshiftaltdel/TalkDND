//! P08 improvisation oracle (TASK-0064): seed reproducibility, read-only
//! behavior on a scope 0 save, the dm-plan template and its backward
//! compatibility, the published-adventure boundary and the help surface
//! (LetsPlay/docs/campaign-continuity.md §5.6).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("talkdnd-oracle-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(p.join("Saves")).unwrap();
    fs::write(p.join("AGENTS.md"), "x").unwrap();
    p
}

/// Exit code plus combined stdout+stderr, for the exact 0/1/2 matrix.
fn code(root: &Path, args: &[&str]) -> (i32, String) {
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
    (out.status.code().unwrap_or(-1), s)
}

fn ok(root: &Path, args: &[&str]) -> String {
    let (c, s) = code(root, args);
    assert_eq!(c, 0, "{args:?} exited {c}: {s}");
    s
}

fn new_save(name: &str) -> (PathBuf, PathBuf) {
    let r = root(name);
    ok(&r, &["new", "--date", "2030-01-02"]);
    let save = r.join("Saves/2030-01-02-01");
    (r, save)
}

/// Recursive snapshot of every regular file: path relative to the base
/// directory mapped to its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                out.insert(
                    p.strip_prefix(base).unwrap().to_path_buf(),
                    fs::read(&p).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

#[test]
fn oracle_commands_are_seed_reproducible() {
    let (r, _save) = new_save("seed");
    // yes-no: the same --seed prints exactly the same one word.
    let a = ok(&r, &["--seed", "42", "oracle", "yes-no", "门后有人吗？"]);
    let b = ok(&r, &["--seed", "42", "oracle", "yes-no", "门后有人吗？"]);
    assert_eq!(a, b);
    let word = a.trim();
    assert!(
        word == "yes" || word == "no",
        "stdout must be exactly yes/no: {a}"
    );
    // The question is never echoed.
    assert!(!a.contains("门后"), "{a}");
    // event: same seed, same phrase; the focus is prefixed.
    let a = ok(&r, &["--seed", "7", "oracle", "event", "--focus", "城门"]);
    let b = ok(&r, &["--seed", "7", "oracle", "event", "--focus", "城门"]);
    assert_eq!(a, b);
    assert!(a.contains("城门："), "{a}");
    // meaning: same seed, same phrase; default chaos is 5.
    let a = ok(&r, &["--seed", "7", "oracle", "meaning"]);
    let b = ok(&r, &["--seed", "7", "oracle", "meaning"]);
    assert_eq!(a, b);
    assert!(a.contains("混乱 5"), "{a}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn meaning_chaos_is_clamped_to_one_through_nine() {
    let (r, _save) = new_save("chaos");
    let out = ok(&r, &["oracle", "meaning", "--chaos", "1"]);
    assert!(out.contains("混乱 1"), "{out}");
    let out = ok(&r, &["oracle", "meaning", "--chaos", "9"]);
    assert!(out.contains("混乱 9"), "{out}");
    let out = ok(&r, &["oracle", "meaning"]);
    assert!(out.contains("混乱 5"), "{out}");
    for bad in ["0", "10"] {
        let (c, _s) = code(&r, &["oracle", "meaning", "--chaos", bad]);
        assert_eq!(c, 2, "--chaos {bad} must be a usage error");
    }
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn oracle_is_read_only_on_a_scope0_save() {
    let (r, save) = new_save("read-only");
    let before = snapshot(&save);
    let cases: &[&[&str]] = &[
        &["--seed", "42", "oracle", "yes-no"],
        &["--seed", "42", "oracle", "yes-no", "门后有人吗？"],
        &["--seed", "42", "oracle", "event"],
        &["--seed", "42", "oracle", "event", "--focus", "城门"],
        &["--seed", "42", "oracle", "meaning"],
        &["--seed", "42", "oracle", "meaning", "--chaos", "9"],
    ];
    // Run everything twice: repeated seeded calls change nothing either.
    for _ in 0..2 {
        for args in cases {
            ok(&r, args);
        }
    }
    let after = snapshot(&save);
    assert_eq!(
        before, after,
        "oracle must not change or create any save file"
    );
    // The named ledgers are untouched (map equality already proves it; these
    // assertions pin the intent for reviewers).
    for name in [
        "progress.yaml",
        "reveals.yaml",
        "original.yaml",
        "save-meta.yaml",
        "rolls.log",
        "canon.md",
    ] {
        assert_eq!(
            before.get(Path::new(name)).map(Vec::as_slice),
            after.get(Path::new(name)).map(Vec::as_slice),
            "{name} changed"
        );
    }
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn dm_plan_template_gets_new_sections_and_never_overwrites() {
    let (r, save) = new_save("dm-plan");
    let plan = save.join("dm-plan.md");
    assert!(!plan.exists(), "a fresh save has no dm-plan.md");
    let out = ok(&r, &["original", "plan"]);
    assert!(out.contains("已创建"), "{out}");
    assert!(plan.is_file());
    let text = fs::read_to_string(&plan).unwrap();
    for section in [
        "## 计划",
        "## 伏笔与回收",
        "## 当前局面",
        "## 参与者目标",
        "## 冲突",
        "## 线索",
        "## 可能行动",
        "## 不介入后果",
    ] {
        assert!(
            text.contains(section),
            "template missing {section}:\n{text}"
        );
    }
    // Backward compatibility: an existing plan is never overwritten,
    // rewritten or parsed — bytes stay identical and the tool says so.
    fs::write(&plan, "# 我的自定义计划\n\n已经写好的内容，绝不覆盖\n").unwrap();
    let before = fs::read(&plan).unwrap();
    let out = ok(&r, &["original", "plan"]);
    assert!(out.contains("已在"), "{out}");
    assert_eq!(fs::read(&plan).unwrap(), before, "dm-plan.md was rewritten");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn oracle_respects_the_published_boundary_on_scope0() {
    let (r, save) = new_save("scope0");
    for args in [
        vec!["oracle", "yes-no", "线索在哪？"],
        vec!["oracle", "event"],
        vec!["oracle", "meaning"],
    ] {
        let out = ok(&r, &args);
        assert!(!out.is_empty(), "{args:?} printed nothing");
    }
    // The oracle never registers original content and never touches the
    // deviation/original ledger — not even indirectly through the template.
    assert!(!save.join("original.yaml").exists());
    ok(&r, &["original", "plan"]);
    assert!(!save.join("original.yaml").exists());
    let out = ok(&r, &["original", "list"]);
    assert!(out.contains("还没有登记原创内容"), "{out}");
    let _ = fs::remove_dir_all(&r);
}

#[test]
fn oracle_help_surface() {
    let r = root("oracle-help");
    let cases: &[(&[&str], &str)] = &[
        (&["oracle", "--help"], "yes-no event meaning"),
        (&["oracle", "yes-no", "--help"], "QUESTION"),
        (&["oracle", "event", "--help"], "--focus"),
        (&["oracle", "meaning", "--help"], "--chaos"),
    ];
    for &(args, needles) in cases {
        let out = ok(&r, args);
        for n in needles.split(' ') {
            assert!(out.contains(n), "{args:?} is missing {n}:\n{out}");
        }
    }
    let _ = fs::remove_dir_all(&r);
}
