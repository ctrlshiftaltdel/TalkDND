//! `talkdnd-dev release-notes <tag> [--changelog CHANGELOG.md]`.
//!
//! Prints the tag's CHANGELOG section body (the `## <tag> — <date>` heading
//! itself is dropped) followed by a fixed download section naming the three
//! release assets. The release workflow writes the result to `notes.md` and
//! passes it to `softprops/action-gh-release` as `body_path`, so the release
//! notes cannot drift from the tracked changelog.

use std::fs;
use std::path::PathBuf;

/// Default changelog path, relative to the current directory (repo root in CI).
pub const DEFAULT_CHANGELOG: &str = "CHANGELOG.md";

pub fn run(args: &[String]) -> Result<u8, String> {
    let (tag, changelog) = parse_args(args)?;
    let path = PathBuf::from(&changelog);
    let text =
        fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let body = render(&text, &tag)?;
    print!("{body}");
    Ok(0)
}

fn parse_args(args: &[String]) -> Result<(String, String), String> {
    let mut tag: Option<String> = None;
    let mut changelog: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if let Some(value) = arg.strip_prefix("--changelog=") {
            if value.is_empty() {
                return Err(usage());
            }
            changelog = Some(value.to_string());
            i += 1;
        } else if arg == "--changelog" {
            let Some(value) = args.get(i + 1) else {
                return Err(usage());
            };
            changelog = Some(value.clone());
            i += 2;
        } else if arg.starts_with('-') {
            return Err(usage());
        } else if tag.is_none() {
            tag = Some(arg.clone());
            i += 1;
        } else {
            return Err(usage());
        }
    }
    let Some(tag) = tag else {
        return Err(usage());
    };
    validate_tag(&tag)?;
    Ok((
        tag,
        changelog.unwrap_or_else(|| DEFAULT_CHANGELOG.to_string()),
    ))
}

pub fn usage() -> String {
    "usage: talkdnd-dev release-notes <tag> [--changelog CHANGELOG.md]".to_string()
}

/// Release tags become archive names, so keep them to characters the
/// packaging step accepts.
fn validate_tag(tag: &str) -> Result<(), String> {
    if tag.is_empty()
        || !tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
    {
        return Err(format!(
            "invalid tag `{tag}`: use only ASCII letters, digits, `.`, `_`, `+`, `-`"
        ));
    }
    Ok(())
}

/// The section body for `tag`, plus the fixed download block.
pub fn render(changelog: &str, tag: &str) -> Result<String, String> {
    let mut body: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in changelog.lines() {
        if is_tag_heading(line, tag) {
            inside = true;
            continue;
        }
        if inside {
            if line.starts_with("## ") {
                break;
            }
            body.push(line);
        }
    }
    if !inside {
        return Err(format!(
            "no `## {tag} — <date>` section in the changelog; add it before releasing"
        ));
    }
    while body.last().is_some_and(|line| line.trim().is_empty()) {
        body.pop();
    }
    let start = body
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(body.len());
    let body = &body[start..];
    let mut out = String::new();
    for line in body {
        out.push_str(line);
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&download_section(tag));
    Ok(out)
}

fn is_tag_heading(line: &str, tag: &str) -> bool {
    line.strip_prefix("## ")
        .and_then(|rest| rest.split_whitespace().next())
        == Some(tag)
}

/// Fixed download section: the three release assets and the checksum note.
pub fn download_section(tag: &str) -> String {
    format!(
        "## 下载\n\n\
         - Linux x86_64：`talkdnd-letsplay-{tag}-linux-x86_64.tar.gz`\n\
         - Windows x86_64：`talkdnd-letsplay-{tag}-windows-x86_64.zip`\n\
         - macOS Apple Silicon：`talkdnd-letsplay-{tag}-macos-arm64.tar.gz`\n\n\
         每个压缩包都有同名、以 `.sha256` 结尾的校验文件；下载后用 `sha256sum -c <压缩包>.sha256`（macOS 用 `shasum -a 256 -c`）校验。\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const CHANGELOG: &str = "\
# 更新日志

## v0.7.0 — 2026-11-01

### 新功能
- newer thing

## v0.6.0 — 2026-10-22

### 修复
- fixed one

### 已知限制
- limit

## v0.5.0 — 2026-10-01

- older thing
";

    fn temp_changelog(name: &str, text: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("talkdnd-dev-notes-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("CHANGELOG.md");
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn picks_the_requested_section_only() {
        let body = render(CHANGELOG, "v0.6.0").unwrap();
        assert!(body.starts_with("### 修复\n- fixed one"), "{body}");
        assert!(body.contains("### 已知限制\n- limit"), "{body}");
        assert!(!body.contains("newer thing"), "{body}");
        assert!(!body.contains("older thing"), "{body}");
        assert!(
            !body.contains("## v0.6.0"),
            "heading must be dropped:\n{body}"
        );
        // Trailing blank lines before 下载 are trimmed.
        assert!(body.contains("- limit\n\n## 下载"), "{body}");
    }

    #[test]
    fn lists_assets_and_checksum_note() {
        let body = render(CHANGELOG, "v0.7.0").unwrap();
        for asset in [
            "talkdnd-letsplay-v0.7.0-linux-x86_64.tar.gz",
            "talkdnd-letsplay-v0.7.0-windows-x86_64.zip",
            "talkdnd-letsplay-v0.7.0-macos-arm64.tar.gz",
        ] {
            assert!(body.contains(asset), "{asset} missing:\n{body}");
        }
        assert!(body.contains(".sha256"), "{body}");
    }

    #[test]
    fn missing_tag_is_an_error() {
        let err = render(CHANGELOG, "v9.9.9").unwrap_err();
        assert!(err.contains("v9.9.9"), "{err}");
        assert!(err.contains("section"), "{err}");
    }

    #[test]
    fn tag_must_be_the_whole_heading_token() {
        // `v0.6.0` must not match a `v0.6.0-beta` heading.
        let text = "## v0.6.0-beta — 2026-10-22\n\n- beta\n";
        assert!(render(text, "v0.6.0").is_err());
        assert!(render(text, "v0.6.0-beta").is_ok());
    }

    #[test]
    fn run_reads_the_given_changelog() {
        let path = temp_changelog("arg", CHANGELOG);
        let args = vec![
            "v0.5.0".to_string(),
            "--changelog".to_string(),
            path.display().to_string(),
        ];
        let (tag, changelog) = parse_args(&args).unwrap();
        assert_eq!(tag, "v0.5.0");
        assert_eq!(changelog, path.display().to_string());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn bad_invocations_are_refused() {
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&["v0.6.0".into(), "--nope".into()]).is_err());
        assert!(parse_args(&["v0.6.0".into(), "--changelog".into()]).is_err());
        assert!(parse_args(&["../evil".into()]).is_err());
    }
}
