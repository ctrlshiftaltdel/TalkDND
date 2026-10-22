//! Tracked-path leak check. Same rules as the former `scripts/check_leaks.sh`.
//! A hit is decided from the path and, for prose, the byte size. File contents
//! are never read.

use std::path::Path;

use crate::{git, git_failure};

/// 300 KiB. The shell used `wc -c` and `-gt`, so this size itself is allowed.
pub const PROSE_BYTE_LIMIT: u64 = 307200;

const BOOK_EXTS: &[&str] = &["pdf", "epub", "docx"];
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "tif", "tiff", "bmp"];
const IMAGE_DIRS: &[&str] = &[
    "LetsPlay/Rules",
    "LetsPlay/Adventures",
    "LetsPlay/Saves",
    "Rules",
    "Adventures",
    "Saves",
];
const PRIVATE_ALLOW: &[&str] = &[
    "LetsPlay/Rules/2014/README.md",
    "LetsPlay/Rules/2024/README.md",
    "Rules/2014/README.md",
    "Rules/2024/README.md",
    "LetsPlay/Adventures/README.md",
    "Adventures/README.md",
    "LetsPlay/Saves/README.md",
    "Saves/README.md",
];
const PRIVATE_DIRS: &[&str] = &[
    "LetsPlay/Rules/2014",
    "LetsPlay/Rules/2024",
    "Rules/2014",
    "Rules/2024",
    "LetsPlay/Adventures",
    "Adventures",
    "LetsPlay/Saves",
    "Saves",
];

pub fn scan(root: &Path) -> Result<Vec<String>, String> {
    let mut hits = Vec::new();
    for rel in git_ls_files(root)? {
        if is_leak(root, &rel) {
            hits.push(rel);
        }
    }
    Ok(hits)
}

pub fn is_leak(root: &Path, rel: &str) -> bool {
    book_container(rel)
        || book_image(rel)
        || private_tree(rel)
        || secret_basename(rel)
        || disallowed_rules_markdown(rel)
        || oversized_prose(root, rel)
        || generated_tree(rel)
}

/// pdf2md output (`.md/`), its state (`.pdf2md/`) and Python environments (`.venv/`).
pub fn generated_tree(path: &str) -> bool {
    path.split('/')
        .any(|segment| matches!(segment, ".md" | ".pdf2md" | ".venv"))
}

fn git_ls_files(root: &Path) -> Result<Vec<String>, String> {
    let output = git()
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|err| format!("git ls-files: {err}"))?;
    if !output.status.success() {
        return Err(git_failure(&output.stderr, "git ls-files failed"));
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

fn book_container(path: &str) -> bool {
    BOOK_EXTS
        .iter()
        .any(|ext| ends_with_lower_or_upper(path, ext))
}

fn book_image(path: &str) -> bool {
    let base = basename(path);
    IMAGE_EXTS
        .iter()
        .any(|ext| ends_with_lower_or_upper(base, ext))
        && IMAGE_DIRS.iter().any(|dir| is_under(path, dir))
}

fn private_tree(path: &str) -> bool {
    if PRIVATE_ALLOW.contains(&path) {
        return false;
    }
    PRIVATE_DIRS.iter().any(|dir| is_under(path, dir))
}

fn secret_basename(path: &str) -> bool {
    let base = basename(path);
    base == "dm-secrets.md" || base == "scarecrow.yaml"
}

fn disallowed_rules_markdown(path: &str) -> bool {
    // Bash `LetsPlay/Rules/*.md` and `Rules/*.md`: `*` matches slashes, and the
    // pattern is anchored to the whole path. `rulings*.md` is the basename:
    // shipped defaults live in `LetsPlay/Rules/defaults/`, older checkouts had
    // them at `LetsPlay/Rules/`, and user copies at `Rules/` are gitignored.
    let under_rules =
        (is_under(path, "LetsPlay/Rules") || is_under(path, "Rules")) && path.ends_with(".md");
    if !under_rules {
        return false;
    }
    let base = basename(path);
    !(base == "README.md" || is_rulings_md(base))
}

fn is_rulings_md(base: &str) -> bool {
    match base.strip_prefix("rulings") {
        Some(rest) => rest.ends_with(".md"),
        None => false,
    }
}

fn oversized_prose(root: &Path, rel: &str) -> bool {
    if !is_prose_suffix(rel) {
        return false;
    }
    // `metadata` follows symlinks, matching `[ -f ]` plus `wc -c`.
    match std::fs::metadata(root.join(rel)) {
        Ok(meta) if meta.is_file() => meta.len() > PROSE_BYTE_LIMIT,
        _ => false,
    }
}

fn is_prose_suffix(path: &str) -> bool {
    path.ends_with(".md")
        || path.ends_with(".txt")
        || path.ends_with(".MD")
        || path.ends_with(".TXT")
}

fn basename(path: &str) -> &str {
    path.rsplit_once('/').map(|(_, base)| base).unwrap_or(path)
}

/// Bash `dir/*`, where `*` also matches slashes and may be empty.
fn is_under(path: &str, dir: &str) -> bool {
    path.strip_prefix(dir)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// Shell `*.pdf|*.PDF` (and the same for the other extensions): all-lower or
/// all-upper only. Mixed case such as `.Pdf` is not a match.
fn ends_with_lower_or_upper(value: &str, ext: &str) -> bool {
    let value = value.as_bytes();
    let ext = ext.as_bytes();
    if value.len() < ext.len() + 1 {
        return false;
    }
    let dot = value.len() - ext.len() - 1;
    if value[dot] != b'.' {
        return false;
    }
    let tail = &value[dot + 1..];
    if !tail.eq_ignore_ascii_case(ext) {
        return false;
    }
    tail.iter().all(|byte| byte.is_ascii_lowercase())
        || tail.iter().all(|byte| byte.is_ascii_uppercase())
}
