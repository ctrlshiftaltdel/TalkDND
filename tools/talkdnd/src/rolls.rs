//! The save's roll log (`Saves/<slot>/rolls.log`).
//!
//! Every die the programs roll for this save is appended here, even when the
//! table's dice-display setting is off. The DM can always audit what was
//! rolled with `talkdnd log rolls [--last N]`; the narration setting only
//! changes what is said at the table, never whether a roll happened.

use crate::save;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "rolls.log";

pub fn path(save_dir: &Path) -> PathBuf {
    save_dir.join(FILE)
}

/// Append one roll. `who` and `what` are free text (`Tess`, `perception`).
/// On a protocol-enabled save the caller must already hold SAVE.
pub fn append(save_dir: &Path, who: &str, what: &str, text: &str, seeded: bool) -> Result<()> {
    if ::advancement_lock::has_protocol(save_dir)
        && !::advancement_lock::is_save_lock_held(save_dir)
    {
        anyhow::bail!(
            "协议违规：在取得 SAVE 锁之前追加 rolls.log；总序必须是先持有 SAVE 再写日志。未写入任何目标文件"
        );
    }
    let tag = if seeded { "（测试种子）" } else { "" };
    save::append(
        &path(save_dir),
        &format!(
            "- [{}] {who} {what}{tag}：{}",
            chrono::Local::now().format("%Y-%m-%d %H:%M"),
            text.trim()
        ),
    )
}

/// Last `n` roll lines (all lines when `n` is `None`), newest last.
pub fn last(save_dir: &Path, n: Option<usize>) -> Vec<String> {
    let text = fs::read_to_string(path(save_dir)).unwrap_or_default();
    let lines: Vec<String> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect();
    match n {
        Some(n) => {
            let start = lines.len().saturating_sub(n);
            lines[start..].to_vec()
        }
        None => lines,
    }
}

/// `talkdnd log rolls [--last N]` output.
pub fn show(save_dir: &Path, n: Option<usize>) -> Vec<String> {
    let lines = last(save_dir, n);
    if lines.is_empty() {
        return vec![
            "（还没有掷骰记录；检定、建卡掷骰和 combat-sim roll 会自动记入 rolls.log）".into(),
        ];
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolls_are_appended_and_tailed() {
        let d = std::env::temp_dir().join(format!("talkdnd-rolls-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        append(&d, "Tess", "perception", "1d20(14)+5=19", false).unwrap();
        append(&d, "Quill", "attack", "1d20(3)+4=7", true).unwrap();
        let all = last(&d, None);
        assert_eq!(all.len(), 2);
        assert!(all[0].contains("Tess perception"));
        assert!(all[1].contains("测试种子"));
        assert_eq!(last(&d, Some(1)).len(), 1);
        assert!(show(&d, Some(5))[1].contains("Quill"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_protocol_roll_append_requires_save_and_does_not_truncate() {
        let root = std::env::temp_dir().join(format!(
            "talkdnd-rolls-proto-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        let save = root.join("Saves/slot");
        fs::create_dir_all(&save).unwrap();
        fs::write(save.join(FILE), "old-line\n").unwrap();
        match ::advancement_lock::init(&root, &save, &::advancement_lock::InitOptions::new()) {
            Ok(_) => {}
            Err(err) if err.kind() == ::advancement_lock::ErrorKind::StatePublishedNotDurable => {}
            Err(err) => panic!("init protocol: {err}"),
        }
        let before = fs::read(save.join(FILE)).unwrap();
        let err = append(&save, "Tess", "perception", "1d20(14)+5=19", false).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("协议违规") && text.contains("未写入任何目标文件"),
            "{text}"
        );
        assert_eq!(fs::read(save.join(FILE)).unwrap(), before);

        let held = ::advancement_lock::open(&root, &save, &::advancement_lock::OpenOptions::new())
            .unwrap();
        held.check_ordinary_write().unwrap();
        let _meta = fs::symlink_metadata(::advancement_lock::lock_path(&save)).unwrap();
        append(&save, "Tess", "perception", "1d20(14)+5=19", false).unwrap();
        drop(held);
        let after = fs::read(save.join(FILE)).unwrap();
        assert!(after.starts_with(&before));
        assert!(String::from_utf8_lossy(&after).contains("Tess perception"));

        let plain = root.join("plain");
        fs::create_dir_all(&plain).unwrap();
        append(&plain, "Quill", "attack", "1d20(3)+4=7", false).unwrap();
        assert!(fs::read_to_string(plain.join(FILE))
            .unwrap()
            .contains("Quill"));
        assert!(!plain.join(::advancement_lock::LOCK_FILE).exists());
        let _ = fs::remove_dir_all(&root);
    }
}
