//! Fail when a tracked play file points outside `LetsPlay/`.
//!
//! Scanned files are tracked `LetsPlay/**/*.{md,yaml,yml,cmd,sh}`. A `..`
//! segment is resolved from that file's directory. A save manifest three
//! levels down (`LetsPlay/<a>/<b>/combat/campaign.yaml`) may use
//! `../../../Rules` and still stay inside `LetsPlay/`. Root development paths
//! (`tools/`, `scripts/`) and `/workspace` are always escapes.
//!
//! It also fails when a tracked play file names a dice source other than
//! `combat-sim roll` (Python `random`, `$RANDOM`, `shuf`, `Get-Random` ...).
//! All dice in play go through `bin/combat-sim roll`.

use std::path::Path;

const EXTS: &[&str] = &["md", "yaml", "yml", "cmd", "sh"];

pub fn scan(root: &Path) -> Result<Vec<String>, String> {
    let mut hits = Vec::new();
    for rel in git_ls_files(root)? {
        if !is_letsplay_text(&rel) {
            continue;
        }
        let text =
            std::fs::read_to_string(root.join(&rel)).map_err(|err| format!("read {rel}: {err}"))?;
        hits.extend(escapes_in(&rel, &text));
        hits.extend(other_dice_in(&rel, &text));
    }
    Ok(hits)
}

/// `rel` is a repo-relative path using `/`.
pub fn escapes_in(rel: &str, text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for_each_token(text, |token| {
        if escapes(rel, token) {
            hits.push(format!("{rel}: {token}"));
        }
    });
    hits
}

/// Text that would tell the DM to roll with something other than combat-sim.
pub const OTHER_DICE_SOURCES: &[&str] = &[
    "systemrandom",
    "import random",
    "random.randint",
    "random.choice",
    "secrets.randbelow",
    "$random",
    "%random%",
    "get-random",
    "math.random",
    "/dev/urandom",
    "shuf ",
    "rand::",
];

pub fn other_dice_in(rel: &str, text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    OTHER_DICE_SOURCES
        .iter()
        .filter(|needle| lower.contains(*needle))
        .map(|needle| {
            format!(
                "{rel}: dice source other than combat-sim roll: `{}`",
                needle.trim()
            )
        })
        .collect()
}

pub fn is_letsplay_text(rel: &str) -> bool {
    let Some((dir, name)) = rel.rsplit_once('/') else {
        return false;
    };
    if !(dir == "LetsPlay" || dir.starts_with("LetsPlay/")) {
        return false;
    }
    EXTS.iter().any(|ext| has_ext(name, ext))
}

fn escapes(file_rel: &str, raw: &str) -> bool {
    let token = raw.replace('\\', "/");
    if token.contains("/workspace") || token.starts_with("workspace/") {
        return true;
    }
    let logical = token.trim_start_matches("./");
    // `LetsPlay/tools/pdf2md` is the one play tool that lives under LetsPlay/;
    // the repository root has no `tools/pdf2md`, so the name is unambiguous.
    let play_tool = logical == "tools/pdf2md" || logical.starts_with("tools/pdf2md/");
    if !play_tool && (logical.starts_with("tools/") || logical.starts_with("scripts/")) {
        return true;
    }
    if !token_has_parent(&token) {
        return false;
    }
    let mut stack = parent_dirs(file_rel);
    if stack.first().map(String::as_str) != Some("LetsPlay") {
        return true;
    }
    for component in components(&token) {
        match component {
            Component::Parent => {
                if stack.len() <= 1 {
                    return true;
                }
                stack.pop();
            }
            Component::Normal(segment) => stack.push(segment.to_string()),
        }
    }
    false
}

enum Component<'a> {
    Parent,
    Normal(&'a str),
}

/// `%~dp0..\..\tools` has no slash between the batch prefix and the first `..`.
/// That glued `..` is still a parent of the file's directory. A run of three
/// dots is an ellipsis, not a parent.
fn components(token: &str) -> Vec<Component<'_>> {
    let mut out = Vec::new();
    for piece in token
        .split('/')
        .filter(|piece| !piece.is_empty() && *piece != ".")
    {
        push_piece(&mut out, piece);
    }
    out
}

fn push_piece<'a>(out: &mut Vec<Component<'a>>, mut piece: &'a str) {
    loop {
        let Some(idx) = parent_dot(piece) else {
            if !piece.is_empty() && piece != "." {
                out.push(Component::Normal(piece));
            }
            return;
        };
        // A prefix glued onto `..` (the `dp0` in `%~dp0..`) is the script
        // directory itself, which is already the resolution base.
        piece = &piece[idx + 2..];
        out.push(Component::Parent);
    }
}

fn token_has_parent(token: &str) -> bool {
    token.split('/').any(|piece| parent_dot(piece).is_some())
}

fn parent_dot(segment: &str) -> Option<usize> {
    let bytes = segment.as_bytes();
    let mut idx = 0;
    while idx + 1 < bytes.len() {
        let parent = bytes[idx] == b'.'
            && bytes[idx + 1] == b'.'
            && bytes.get(idx + 2) != Some(&b'.')
            && (idx == 0 || bytes[idx - 1] != b'.');
        if parent {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

fn parent_dirs(file_rel: &str) -> Vec<String> {
    let mut parts: Vec<&str> = file_rel
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    parts.pop();
    parts.into_iter().map(str::to_string).collect()
}

fn for_each_token(text: &str, mut visit: impl FnMut(&str)) {
    let mut start = None;
    for (idx, ch) in text.char_indices() {
        if is_path_char(ch) {
            if start.is_none() {
                start = Some(idx);
            }
        } else if let Some(begin) = start.take() {
            visit(&text[begin..idx]);
        }
    }
    if let Some(begin) = start {
        visit(&text[begin..]);
    }
}

fn is_path_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '\\')
}

fn has_ext(name: &str, ext: &str) -> bool {
    let name = name.as_bytes();
    let ext = ext.as_bytes();
    name.len() > ext.len()
        && name[name.len() - ext.len() - 1] == b'.'
        && name[name.len() - ext.len()..].eq_ignore_ascii_case(ext)
}

fn git_ls_files(root: &Path) -> Result<Vec<String>, String> {
    let output = crate::git()
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|err| format!("git ls-files: {err}"))?;
    if !output.status.success() {
        return Err(crate::git_failure(&output.stderr, "git ls-files failed"));
    }
    let mut files = Vec::new();
    for chunk in output.stdout.split(|byte| *byte == 0) {
        if chunk.is_empty() {
            continue;
        }
        let rel = String::from_utf8(chunk.to_vec())
            .map_err(|_| "git ls-files returned a non-utf8 path".to_string())?;
        files.push(rel);
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_manifest_rules_path_stays_inside() {
        let rel = "LetsPlay/example/save/combat/campaign.yaml";
        let text = "\
library: [../../../Rules/combat]
adventures: [../../../Adventures/example/combat]
characters: [../Characters, ../NPCs]
state_dir: .
";
        assert!(escapes_in(rel, text).is_empty(), "{text}");
        assert!(is_letsplay_text(rel));
    }

    #[test]
    fn sibling_dirs_and_edition_layers_stay_inside() {
        assert!(escapes_in(
            "LetsPlay/Rules/combat/common/conditions.yaml",
            "# Edition-specific ones live in ../2014 and ../2024.\n"
        )
        .is_empty());
        assert!(escapes_in(
            "LetsPlay/Saves/README.md",
            "规则书在 `../Rules/2014/` 与 `../Adventures/`。\n"
        )
        .is_empty());
    }

    #[test]
    fn ellipsis_is_not_a_parent_segment() {
        let text = "\
WARNING ... acting out of turn
party: [角色id, ...]
attack ... --nonlethal
bin/combat-sim
";
        assert!(escapes_in("LetsPlay/AGENTS.md", text).is_empty());
        assert!(escapes_in("LetsPlay/Rules/rulings.md", text).is_empty());
        assert!(escapes_in("LetsPlay/Rules/defaults/rulings.md", text).is_empty());
    }

    #[test]
    fn paths_that_leave_letsplay_are_reported() {
        let hits = escapes_in(
            "LetsPlay/AGENTS.md",
            "see `../README.md` and `../tools/bin` and tools/combat-sim and scripts/validate_kit.py and /workspace/x\n",
        );
        assert!(hits.iter().any(|hit| hit.contains("../README.md")));
        assert!(hits.iter().any(|hit| hit.contains("../tools/bin")));
        assert!(hits.iter().any(|hit| hit.contains("tools/combat-sim")));
        assert!(
            escapes_in("LetsPlay/AGENTS.md", "run `uv sync --project tools/pdf2md`").is_empty()
        );
        assert!(hits
            .iter()
            .any(|hit| hit.contains("scripts/validate_kit.py")));
        assert!(hits.iter().any(|hit| hit.contains("/workspace/x")));
        assert!(hits
            .iter()
            .all(|hit| hit.starts_with("LetsPlay/AGENTS.md: ")));
    }

    #[test]
    fn readme_depth_cannot_use_a_save_manifest_prefix() {
        let hits = escapes_in(
            "LetsPlay/Saves/README.md",
            "library: [../../../Rules/combat]\n",
        );
        assert_eq!(
            hits,
            vec!["LetsPlay/Saves/README.md: ../../../Rules/combat".to_string()]
        );
    }

    #[test]
    fn windows_parent_paths_escape() {
        let hits = escapes_in(
            "LetsPlay/bin/combat-sim.cmd",
            "\"%~dp0..\\..\\tools\\bin\\combat-sim.exe\"\n",
        );
        assert!(
            hits.iter()
                .any(|hit| hit.contains("..\\..\\tools\\bin\\combat-sim.exe")),
            "{hits:?}"
        );
    }

    #[test]
    fn other_dice_sources_are_reported() {
        let text = "Roll with python3 -c \"import random;r=random.SystemRandom()\"";
        let hits = other_dice_in("LetsPlay/AGENTS.md", text);
        assert!(hits.iter().any(|h| h.contains("systemrandom")), "{hits:?}");
        assert!(hits.iter().any(|h| h.contains("import random")), "{hits:?}");
        assert!(other_dice_in("LetsPlay/x.md", "echo $RANDOM").len() == 1);
        assert!(other_dice_in("LetsPlay/x.md", "Get-Random -Max 20").len() == 1);
        assert!(other_dice_in("LetsPlay/x.md", "bin/combat-sim roll 1d20+5 --adv").is_empty());
        assert!(other_dice_in("LetsPlay/x.md", "true random dice; randomly chosen").is_empty());
    }

    #[test]
    fn extension_filter_skips_other_files() {
        assert!(is_letsplay_text("LetsPlay/AGENTS.md"));
        assert!(is_letsplay_text(
            "LetsPlay/Rules/combat/common/conditions.yaml"
        ));
        assert!(is_letsplay_text("LetsPlay/bin/combat-sim.cmd"));
        assert!(is_letsplay_text("LetsPlay/bin/run.SH"));
        assert!(!is_letsplay_text("LetsPlay/bin/combat-sim"));
        assert!(!is_letsplay_text("tools/combat-sim/README.md"));
        assert!(!is_letsplay_text("README.md"));
    }
}
