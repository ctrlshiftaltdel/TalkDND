//! What the players currently know, as one list (`talkdnd known`).
//!
//! Printed in full by `talkdnd known` and after a scene change, and compactly
//! by `talkdnd start`. The DM must build descriptions and options only from
//! this list: revealed facts, shown handouts, identified names, original
//! content the party has met, party basics and the current chapter/location.

use crate::{check, handout, identify, original, progress, reveal};
use std::path::Path;

fn party_line(s: &check::Sheet) -> String {
    let level = s.block.get("level").and_then(|v| v.as_i64()).unwrap_or(1);
    let hp = s
        .block
        .get("hp")
        .and_then(|v| v.as_i64())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "?".into());
    let ac = s
        .block
        .get("ac")
        .and_then(|v| v.as_i64())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "?".into());
    let class = s
        .block
        .get("notes")
        .and_then(|v| v.as_str())
        .map(|n| n.split_whitespace().next().unwrap_or("").to_string())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "?".into());
    format!(
        "{}（{}）：{} {} 级，HP {}，AC {}",
        s.name, s.id, class, level, hp, ac
    )
}

/// Full `talkdnd known` output. `compact` is the `talkdnd start` version:
/// fewer revealed facts and counts instead of full lists where it gets long.
pub fn lines(save_dir: &Path, compact: bool) -> Vec<String> {
    let mut out = Vec::new();
    let ctx = original::context(save_dir).ok();
    let scene = progress::scene(save_dir);
    out.push(format!(
        "当前章节：{}{}",
        ctx.as_ref().map(|c| c.chapter.as_str()).unwrap_or("未分章"),
        scene
            .as_deref()
            .map(|s| format!("；当前场景：{s}"))
            .unwrap_or_else(|| "；当前场景：未记录（`talkdnd scene <名字>`）".into())
    ));

    out.push("队伍：".into());
    let sheets = check::sheets(save_dir);
    let mut listed = 0;
    for s in sheets.iter().filter(|s| {
        s.block
            .get("kind")
            .and_then(|v| v.as_str())
            .is_none_or(|k| k.eq_ignore_ascii_case("pc"))
    }) {
        out.push(format!("- {}", party_line(s)));
        listed += 1;
    }
    if listed == 0 {
        out.push("- （还没有角色卡）".into());
    }

    let facts = reveal::load(save_dir).map(|l| l.facts).unwrap_or_default();
    out.push(format!("已揭示事实（{} 条）：", facts.len()));
    let shown_facts: Vec<&reveal::Fact> = if compact {
        facts.iter().rev().take(5).collect()
    } else {
        facts.iter().collect()
    };
    if shown_facts.is_empty() {
        out.push("- （还没有；得知新事实时 `talkdnd reveal add`）".into());
    }
    for f in shown_facts {
        let mut line = format!("#{} {}", f.n, f.fact);
        if !f.source.is_empty() {
            line += &format!("（来源：{}）", f.source);
        }
        out.push(format!("- {line}"));
    }
    if compact && facts.len() > 5 {
        out.push(format!(
            "- ……另有 {} 条（`talkdnd reveal list`）",
            facts.len() - 5
        ));
    }

    let handouts = handout::shown_lines(save_dir);
    out.push(format!("已展示资料图（{} 张）：", handouts.len()));
    if handouts.is_empty() {
        out.push("- （还没有）".into());
    } else if compact {
        for h in handouts.iter().rev().take(3) {
            out.push(format!("- {h}"));
        }
    } else {
        for h in &handouts {
            out.push(format!("- {h}"));
        }
    }

    let identified = identify::load(save_dir)
        .map(|l| {
            l.entries
                .into_iter()
                .filter(|e| e.identified)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    out.push(format!("已识别怪物/物品（{} 项）：", identified.len()));
    if identified.is_empty() {
        out.push("- （还没有；`talkdnd identify <名字>`）".into());
    } else {
        for e in &identified {
            out.push(format!(
                "- {}{}",
                e.name,
                if e.reason.is_empty() {
                    String::new()
                } else {
                    format!("（{}）", e.reason)
                }
            ));
        }
    }

    let known_originals = original::known_entries(save_dir);
    out.push(format!(
        "已出场的原创内容（{} 项）：",
        known_originals.len()
    ));
    if known_originals.is_empty() {
        out.push("- （还没有）".into());
    } else {
        for e in &known_originals {
            let kind = original::Kind::parse(&e.kind)
                .map(|k| k.zh())
                .unwrap_or(e.kind.as_str());
            out.push(format!("- {}「{}」（{}）", e.id, e.name, kind));
        }
    }

    if !compact {
        out.push(
            "规则：描述和选项只能从此列表出发；未揭示的专名、暗门、陷阱、DC 不出现。改动后重新运行 `talkdnd known`。"
                .into(),
        );
    }
    out
}

/// Compact known block for `talkdnd start` (a short section, not the full list).
pub fn digest_lines(save_dir: &Path) -> Vec<String> {
    lines(save_dir, true)
}

/// `talkdnd scene <name>` records the scene, then prints the full known list.
pub fn scene_change(save_dir: &Path, scene: &str) -> anyhow::Result<Vec<String>> {
    let mut out = vec![progress::set_scene(save_dir, scene)?];
    out.extend(lines(save_dir, false));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate;
    use std::fs;

    #[test]
    fn known_lists_facts_handouts_identified_party_and_scene() {
        let d = std::env::temp_dir().join(format!("talkdnd-known-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for sub in ["Characters", "Sessions", "combat"] {
            fs::create_dir_all(d.join(sub)).unwrap();
        }
        fs::write(
            d.join("combat/campaign.yaml"),
            "party: [hero]\nruleset: \"2014\"\n",
        )
        .unwrap();
        fs::write(
            d.join("Characters/Hero.md"),
            "# Hero\n\n```combat yaml\nid: hero\nname: Hero\nkind: pc\nlevel: 3\nhp: 24\nac: 16\nnotes: \"rogue 3. synthetic\"\n```\n",
        )
        .unwrap();
        fs::write(d.join("Sessions/s.md"), "# log\n").unwrap();
        migrate::init(&d).unwrap();
        reveal::add(&d, "The gala is tonight", "rumour", &[]).unwrap();
        progress::set_scene(&d, "宴会厅").unwrap();
        identify::identify(&d, "Goblin", "check", Some("perception 15")).unwrap();
        let text = lines(&d, false).join("\n");
        assert!(text.contains("当前场景：宴会厅"), "{text}");
        assert!(text.contains("#1 The gala is tonight"), "{text}");
        assert!(
            text.contains("Hero（hero）：rogue 3 级，HP 24，AC 16"),
            "{text}"
        );
        assert!(text.contains("Goblin（perception 15）"), "{text}");
        assert!(text.contains("规则：描述和选项"), "{text}");
        let compact = digest_lines(&d).join("\n");
        assert!(!compact.contains("规则：描述和选项"), "{compact}");
        // A scene change goes through the log.
        let out = scene_change(&d, "地窖").unwrap().join("\n");
        assert!(
            out.contains("场景：地窖") && out.contains("当前场景：地窖"),
            "{out}"
        );
        let log = fs::read_to_string(d.join("Sessions/s.md")).unwrap();
        assert!(log.contains("场景：地窖"), "{log}");
        let _ = fs::remove_dir_all(&d);
    }
}
