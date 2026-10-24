//! Improvisation oracle (P08; LetsPlay/docs/campaign-continuity.md §5.6).
//!
//! `talkdnd oracle ...` answers yes/no questions and suggests event focus
//! and meaning phrases from this module's own original lists. Every command
//! is strictly read-only on the save: nothing is read, created or written
//! (no authoritative state, no rolls.log, no session log), dice are never
//! faked (they go through combat-sim's engine via the shared RNG) and the
//! output never registers original content or bypasses the deviation rules.
//! The phrases below are written for this module — never taken from a
//! published book or random table.

use anyhow::Result;
use clap::Subcommand;
use combat_sim::dice::DiceRng;
use std::path::Path;

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Yes or no (roll 1d6: 1-3 yes, 4-6 no) through combat-sim's dice engine
    YesNo {
        /// The question (optional)
        question: Vec<String>,
    },
    /// One event focus phrase from the built-in original list
    Event {
        /// Optional focus for the event
        #[arg(long)]
        focus: Option<String>,
    },
    /// One meaning phrase from the built-in original list, with the chaos value
    Meaning {
        /// Chaos level 1-9 (default 5)
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u8).range(1..=9))]
        chaos: u8,
    },
}

/// Original, generic event-focus phrases (never from a published book or
/// random table).
const EVENT_FOCUS: &[&str] = &[
    "一位不带恶意的陌生人向队伍求助",
    "队伍的行踪被无关的第三方注意到",
    "一封没有署名的信件被送到队伍手里",
    "当地说得上话的人突然想见队伍",
    "一笔意外之财在原地引发争夺",
    "天气突变，原本的行程走不通了",
    "装备或补给出了必须立刻处理的问题",
    "一名旧识带来一个意外的消息",
    "消息比队伍先一步传到下一个地点",
    "道路或入口被说不清的原因挡住",
    "夜里出现不属于熟悉动物的动静",
    "一处平时紧闭的场所敞开了门",
    "有人向队伍打听此前那场战斗",
    "一份旧文书里浮出一个新名字",
    "一次小意外暴露了有人留下的记号",
    "队伍里有人被单独搭话",
    "附近的居民开始回避某个话题",
    "有威望的人做事和传闻对不上",
    "一件物品在没人碰它时换了位置",
    "一个孩子说出了不该知道的事",
    "两位盟友因为误会互相提防",
    "有人一直在远处观察队伍",
    "一处陷阱显出刚被人维护过的痕迹",
    "市价无声地涨了，没人肯说原因",
];

/// Original, generic meaning phrases (never from a published book or random
/// table).
const MEANINGS: &[&str] = &[
    "关键藏在一个被忽视的普通人身上",
    "答案就在公开的记录里",
    "有人为了保护别人而说谎",
    "挡在面前的敌人不是幕后的推手",
    "时机比力量更重要",
    "失去一件旧物会换来新的机会",
    "帮别人一把就是推进自己的目标",
    "再拖延下去对手会抢先一步",
    "眼前最省事的出路其实是陷阱",
    "被拒绝过的地方值得再试一次",
    "一句闲谈里藏着要害",
    "一句旧誓言在新局面下生效",
    "代价早就付过，只是没人认账",
    "旁观的人掌握着决定性的细节",
    "态度太硬会关上唯一的大门",
    "分头行动才能同时赶上两件事",
    "需要一个敌人暂时成为盟友",
    "名字被刻意弄错，掩盖着真实身份",
    "这条路既通向出口也通向深处",
    "物品的真正用途和它的名字相反",
    "有人在等队伍先动手",
    "恐惧比守卫更难对付",
    "留下记号的人希望被跟上",
    "结局取决于谁先承认错误",
];

/// `oracle yes-no|event|meaning` (P08, TASK-0064). Strictly read-only: the
/// save paths are accepted for signature uniformity with the other command
/// groups and are never read or written.
pub fn run(root: &Path, save: &Path, cmd: Cmd, rng: &mut dyn DiceRng) -> Result<()> {
    let _ = (root, save);
    match cmd {
        Cmd::YesNo { question } => {
            let _ = question; // The question is never echoed back.
            println!("{}", yes_no(rng));
        }
        Cmd::Event { focus } => println!("{}", event_line(focus.as_deref(), rng)),
        Cmd::Meaning { chaos } => println!("{}", meaning_line(chaos, rng)),
    }
    Ok(())
}

/// 1d6 through the shared RNG: 1-3 yes, 4-6 no.
fn yes_no(rng: &mut dyn DiceRng) -> &'static str {
    if rng.roll_die(6) <= 3 {
        "yes"
    } else {
        "no"
    }
}

/// One event-focus phrase, prefixed with the focus when it is non-empty.
fn event_line(focus: Option<&str>, rng: &mut dyn DiceRng) -> String {
    let phrase = pick(EVENT_FOCUS, rng);
    match focus.map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => format!("{f}：{phrase}"),
        None => phrase.to_string(),
    }
}

/// One meaning phrase with the chaos value (clap clamps it to 1-9).
fn meaning_line(chaos: u8, rng: &mut dyn DiceRng) -> String {
    format!("{}（混乱 {chaos}）", pick(MEANINGS, rng))
}

/// Uniform pick from a non-empty table: `roll_die(len)` yields 1..=len.
fn pick(table: &'static [&'static str], rng: &mut dyn DiceRng) -> &'static str {
    let roll = rng.roll_die(table.len() as u32);
    table[(roll as usize).saturating_sub(1) % table.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Scripted RNG honoring the DiceRng contract: each roll pops the next
    /// scripted value, which must be in 1..=sides.
    struct Scripted(VecDeque<u32>);

    impl DiceRng for Scripted {
        fn roll_die(&mut self, sides: u32) -> u32 {
            let v = self.0.pop_front().expect("script exhausted");
            assert!((1..=sides).contains(&v), "{v} out of 1..={sides}");
            v
        }
    }

    #[test]
    fn yes_no_maps_whole_d6() {
        for (roll, expected) in [
            (1, "yes"),
            (2, "yes"),
            (3, "yes"),
            (4, "no"),
            (5, "no"),
            (6, "no"),
        ] {
            let mut rng = Scripted(VecDeque::from([roll]));
            assert_eq!(yes_no(&mut rng), expected);
        }
    }

    #[test]
    fn pick_reaches_first_and_last_entries() {
        let mut first = Scripted(VecDeque::from([1]));
        assert_eq!(pick(EVENT_FOCUS, &mut first), EVENT_FOCUS[0]);
        let mut last = Scripted(VecDeque::from([EVENT_FOCUS.len() as u32]));
        assert_eq!(
            pick(EVENT_FOCUS, &mut last),
            EVENT_FOCUS[EVENT_FOCUS.len() - 1]
        );
        let mut m_first = Scripted(VecDeque::from([1]));
        assert_eq!(pick(MEANINGS, &mut m_first), MEANINGS[0]);
        let mut m_last = Scripted(VecDeque::from([MEANINGS.len() as u32]));
        assert_eq!(pick(MEANINGS, &mut m_last), MEANINGS[MEANINGS.len() - 1]);
    }

    #[test]
    fn event_line_prefixes_only_a_nonempty_focus() {
        let mut rng = Scripted(VecDeque::from([1]));
        assert_eq!(
            event_line(Some("城门"), &mut rng),
            format!("城门：{}", EVENT_FOCUS[0])
        );
        let mut rng = Scripted(VecDeque::from([1]));
        assert_eq!(event_line(None, &mut rng), EVENT_FOCUS[0]);
        // Blank focus counts as absent.
        let mut rng = Scripted(VecDeque::from([1]));
        assert_eq!(event_line(Some("  "), &mut rng), EVENT_FOCUS[0]);
    }

    #[test]
    fn meaning_line_carries_chaos_bounds_and_default() {
        for chaos in [1, 5, 9] {
            let mut rng = Scripted(VecDeque::from([1]));
            assert_eq!(
                meaning_line(chaos, &mut rng),
                format!("{}（混乱 {chaos}）", MEANINGS[0])
            );
        }
    }

    #[test]
    fn phrase_tables_are_nonempty_and_unique() {
        for table in [EVENT_FOCUS, MEANINGS] {
            assert!(!table.is_empty());
            assert!(table.iter().all(|p| !p.trim().is_empty()));
            let mut sorted: Vec<_> = table.to_vec();
            sorted.sort();
            let n = sorted.len();
            sorted.dedup();
            assert_eq!(sorted.len(), n, "duplicate phrase");
        }
    }
}
