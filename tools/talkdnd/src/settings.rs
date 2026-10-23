//! Per-save settings (`save-meta.yaml` -> `settings:`).
//!
//! `dice_display` (default on) controls whether the DM reads dice values,
//! DCs and modifiers aloud. It never changes whether the programs roll: every
//! roll is made by the programs and written to `rolls.log`, and the DM can
//! show it later with `talkdnd log rolls`.

use crate::migrate::{self, Settings};
use crate::{save, save::append};
use anyhow::{bail, Result};
use std::path::Path;

pub fn get(save_dir: &Path) -> Settings {
    migrate::load(save_dir)
        .ok()
        .flatten()
        .map(|m| m.settings)
        .unwrap_or_default()
}

pub fn dice_display(save_dir: &Path) -> bool {
    get(save_dir).dice_display
}

/// `talkdnd settings` (show) and after a change.
pub fn lines(save_dir: &Path) -> Vec<String> {
    let s = get(save_dir);
    vec![if s.dice_display {
        "骰子显示：开（每个检定/伤害展示完整算式；见 docs/dice.md）".to_string()
    } else {
        "骰子显示：关（只叙述结果，不报骰值、DC、修正；掷骰仍由程序完成并记入 rolls.log）"
            .to_string()
    }]
}

/// Change dice display and log it. `--reason` is optional and only recorded.
pub fn set_dice(
    root: &Path,
    save_dir: &Path,
    on: bool,
    reason: Option<&str>,
) -> Result<Vec<String>> {
    // Old saves migrate first so they get a schema-5 save-meta.yaml.
    migrate::ensure(save_dir, root, false)?;
    let Some(mut meta) = migrate::load(save_dir)? else {
        bail!("{}: save-meta.yaml is missing", save_dir.display());
    };
    meta.settings.dice_display = on;
    migrate::write_meta(save_dir, &meta)?;
    let why = reason
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(|r| format!("（{r}）"))
        .unwrap_or_default();
    let line = format!("骰子显示：{}{why}", if on { "开" } else { "关" });
    if let Some(log) = save::latest_log(save_dir) {
        append(
            &log,
            &format!("- [{}] {line}", chrono::Local::now().format("%H:%M")),
        )?;
    }
    Ok(vec![line])
}

/// `talkdnd settings dice on|off`; no value shows the current setting.
pub fn parse_dice(value: Option<&str>) -> Result<Option<bool>> {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) if v.eq_ignore_ascii_case("on") || v == "开" => Ok(Some(true)),
        Some(v) if v.eq_ignore_ascii_case("off") || v == "关" => Ok(Some(false)),
        Some(v) => bail!("骰子显示只能是 on 或 off（收到 `{v}`）"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::CURRENT_SCHEMA;

    fn root(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("talkdnd-settings-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("Saves/2030-01-01-01/Sessions")).unwrap();
        std::fs::write(
            p.join("Saves/2030-01-01-01/Sessions/2030-01-01-session-00.md"),
            "# log\n",
        )
        .unwrap();
        crate::migrate::init(&p.join("Saves/2030-01-01-01")).unwrap();
        p
    }

    #[test]
    fn default_is_on_and_changes_are_logged() {
        let r = root("toggle");
        let save = r.join("Saves/2030-01-01-01");
        assert!(dice_display(&save));
        assert_eq!(
            migrate::load(&save).unwrap().unwrap().schema,
            CURRENT_SCHEMA
        );
        assert!(lines(&save)[0].contains("开"));
        let out = set_dice(&r, &save, false, Some("table vote")).unwrap();
        assert!(out[0].contains("关") && out[0].contains("table vote"));
        assert!(!dice_display(&save));
        let log = std::fs::read_to_string(save.join("Sessions/2030-01-01-session-00.md")).unwrap();
        assert!(log.contains("骰子显示：关"), "{log}");
        set_dice(&r, &save, true, None).unwrap();
        assert!(dice_display(&save));
        assert!(parse_dice(Some("on")).unwrap() == Some(true));
        assert!(parse_dice(Some("off")).unwrap() == Some(false));
        assert!(parse_dice(None).unwrap().is_none());
        assert!(parse_dice(Some("maybe")).is_err());
        let _ = std::fs::remove_dir_all(&r);
    }
}
