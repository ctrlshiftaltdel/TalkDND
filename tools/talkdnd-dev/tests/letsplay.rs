mod common;

use common::{text, TempRepo};

#[test]
fn clean_play_tree_exits_zero() {
    let repo = TempRepo::new();
    repo.write(
        "LetsPlay/Rules/combat/common/conditions.yaml",
        "# Edition-specific ones live in ../2014 and ../2024.\n",
    );
    repo.write(
        "LetsPlay/example/save/combat/campaign.yaml",
        "\
library: [../../../Rules/combat]
adventures: [../../../Adventures/example/combat]
characters: [../Characters, ../NPCs]
",
    );
    repo.write(
        "LetsPlay/Saves/README.md",
        "规则书在 `../Rules/2014/`。WARNING ... acting out of turn\n",
    );
    repo.write(
        "LetsPlay/AGENTS.md",
        "bin/combat-sim\nbin\\combat-sim.exe\nparty: [id, ...]\n",
    );
    repo.write(
        "tools/combat-sim/README.md",
        "see ../LetsPlay and tools/bin\n",
    );
    repo.write("LetsPlay/secret.txt", "../tools/bin\n");
    repo.add(&[
        "LetsPlay/Rules/combat/common/conditions.yaml",
        "LetsPlay/example/save/combat/campaign.yaml",
        "LetsPlay/Saves/README.md",
        "LetsPlay/AGENTS.md",
        "tools/combat-sim/README.md",
        "LetsPlay/secret.txt",
    ]);
    repo.write("LetsPlay/untracked.md", "../tools/bin\n");

    let output = common::run(&repo.path, &["check-letsplay"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "");
}

#[test]
fn escapes_are_printed_and_exit_one() {
    let repo = TempRepo::new();
    repo.write(
        "LetsPlay/AGENTS.md",
        "\
`../README.md`
`../tools/bin`
tools/combat-sim
scripts/validate_kit.py
/workspace/internal
",
    );
    repo.write(
        "LetsPlay/bin/combat-sim.cmd",
        "\"%~dp0..\\..\\tools\\bin\\combat-sim.exe\"\n",
    );
    repo.write(
        "LetsPlay/Saves/README.md",
        "library: [../../../Rules/combat]\n",
    );
    repo.add(&[
        "LetsPlay/AGENTS.md",
        "LetsPlay/bin/combat-sim.cmd",
        "LetsPlay/Saves/README.md",
    ]);

    let output = common::run(&repo.path, &["check-letsplay"]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stdout));
    let stdout = text(&output.stdout);
    for needle in [
        "LetsPlay/AGENTS.md: ../README.md",
        "LetsPlay/AGENTS.md: ../tools/bin",
        "LetsPlay/AGENTS.md: tools/combat-sim",
        "LetsPlay/AGENTS.md: scripts/validate_kit.py",
        "LetsPlay/AGENTS.md: /workspace/internal",
        "LetsPlay/Saves/README.md: ../../../Rules/combat",
    ] {
        assert!(stdout.contains(needle), "{needle} not in {stdout}");
    }
    assert!(
        stdout.contains("..\\..\\tools\\bin\\combat-sim.exe")
            || stdout.contains("../../tools/bin/combat-sim.exe"),
        "{stdout}"
    );
    assert!(!stdout.contains("library:"));
}

#[test]
fn extra_args_exit_two() {
    let repo = TempRepo::new();
    let output = common::run(&repo.path, &["check-letsplay", "extra"]);
    assert_eq!(output.status.code(), Some(2), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "");
    assert!(text(&output.stderr).contains("usage: talkdnd-dev check-letsplay"));
}
