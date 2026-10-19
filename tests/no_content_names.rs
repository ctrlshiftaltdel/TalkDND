//! Content-name guard: `src/` must stay campaign-agnostic.
//!
//! The deny list below is intentionally literal. Tests may use synthetic names;
//! production source may not contain spell, feature, monster, condition or
//! campaign names. `Vec::push` method calls are ignored because "push" is also
//! a weapon-mastery name in campaign data.

use std::fs;
use std::path::{Path, PathBuf};

const DENY: &[&str] = &[
    "sneak",
    "second wind",
    "second_wind",
    "missile",
    "sap",
    "vex",
    "topple",
    "graze",
    "nick",
    "cleave",
    "push",
    "guard",
    "mimic",
    "allosaurus",
    "scarecrow",
    "statue",
    "satyr",
    "marigold",
    "will",
    "rina",
    "milo",
    "sera",
    "bless",
    "blessed",
    "sanctuary",
    "shield",
    "shielded",
    "prone",
    "frightened",
    "paralyzed",
    "unconscious",
    "grappled",
    "poisoned",
    "restrained",
    "stunned",
    "fire bolt",
    "sacred flame",
    "cure wounds",
    "murkmire",
    "varkenbluff",
];

/// Generic Rust spellings that are unavoidable in this crate. Keep this empty
/// unless a word is truly structural rather than content vocabulary.
const ALLOW: &[&str] = &[];

#[test]
fn source_has_no_campaign_content_names() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect(&root, &mut files);
    assert!(!files.is_empty(), "no source files found under {root:?}");
    let mut hits = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path).expect("source file is readable");
        for (number, line) in text.lines().enumerate() {
            let code = strip_comments(line).to_ascii_lowercase();
            for word in DENY {
                if ALLOW.contains(word) {
                    continue;
                }
                if contains_denied_word(&code, word) {
                    hits.push(format!(
                        "{}:{}: `{word}`",
                        path.display(),
                        number + 1
                    ));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "campaign content names leaked into src/:\n{}",
        hits.join("\n")
    );
}

fn collect(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

/// Drops `//` line comments without being fooled by `//` inside a string.
fn strip_comments(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut quote = None;
    let mut escaped = false;
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        if escaped {
            output.push(character);
            escaped = false;
            continue;
        }
        match quote {
            Some(active) => {
                output.push(character);
                if character == '\\' {
                    escaped = true;
                } else if character == active {
                    quote = None;
                }
            }
            None => {
                if character == '"' || character == '\'' {
                    quote = Some(character);
                    output.push(character);
                } else if character == '/'
                    && characters.peek().is_some_and(|next| *next == '/')
                {
                    break;
                } else {
                    output.push(character);
                }
            }
        }
    }
    output
}

fn contains_denied_word(haystack: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = haystack[from..].find(needle) {
        let start = from + offset;
        let end = start + needle.len();
        let before = haystack[..start].chars().next_back();
        let after = haystack[end..].chars().next();
        let boundary_before = before.is_none_or(|character| !is_word(character));
        let boundary_after = after.is_none_or(|character| !is_word(character));
        if boundary_before && boundary_after {
            if needle == "push" && is_push_method(haystack, start, end) {
                from = end;
                continue;
            }
            return true;
        }
        from = start + needle.len();
    }
    false
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// `item.push(value)` / `Vec::push` are ordinary Rust, not the mastery name.
fn is_push_method(haystack: &str, start: usize, end: usize) -> bool {
    let before = &haystack[..start];
    let after = &haystack[end..];
    let method_call = before.trim_end().ends_with('.') && after.trim_start().starts_with('(');
    let path = before.trim_end().ends_with("Vec::") && after.trim_start().starts_with('(');
    method_call || path
}
