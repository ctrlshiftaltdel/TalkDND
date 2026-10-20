//! The `roll` command's dice grammar.
//!
//! ```text
//! expr  := term (('+' | '-') term)*        (a leading '+' or '-' is allowed)
//! term  := dice | integer
//! dice  := [count] 'd' sides [keep]
//! sides := 2 | 3 | 4 | 6 | 8 | 10 | 12 | 20 | 100 | '%'
//! keep  := ('kh' | 'kl' | 'dh' | 'dl') [n]   (n defaults to 1)
//! ```
//!
//! Only the dice the D&D rules use are accepted: d4, d6, d8, d10, d12, d20 and
//! d100 (`d%`). d2 and d3 are also accepted because the rules call for them
//! (they are traditionally rolled as a halved d4 / d6); each is a uniform 1..=N
//! roll. Anything else (d7, d0, d1000 ...) is rejected. A dice term holds 1 to
//! 100 dice and an expression at most 20 terms.
//!
//! Output keeps every die in roll order. Dropped dice are wrapped in brackets:
//! `4d6dl1(5,3,[1],6)=14`. An advantage d20 keeps the old style:
//! `1d20(3,17 adv→17)+5=22`.

use std::fmt::Write as _;

use anyhow::{bail, Result};
use serde::Serialize;

use crate::dice::{DiceRng, RollMode};

/// Die sizes accepted by the grammar.
pub const STANDARD_SIDES: &[u32] = &[2, 3, 4, 6, 8, 10, 12, 20, 100];
/// Most dice in one term.
pub const MAX_DICE_PER_TERM: u32 = 100;
/// Most terms in one expression.
pub const MAX_TERMS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Keep {
    All,
    Highest(u32),
    Lowest(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiceTerm {
    pub negative: bool,
    pub count: u32,
    pub sides: u32,
    pub keep: Keep,
    /// Normalised notation such as `4d6dl1` (from the keep suffix as written).
    suffix: String,
    /// Set by `--adv` / `--dis` on a lone 1d20.
    mode: RollMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    Dice(DiceTerm),
    Flat(i32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollExpr {
    pub terms: Vec<Term>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TermOutcome {
    /// `dice` or `flat`.
    pub kind: &'static str,
    pub sign: i32,
    pub notation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sides: Option<u32>,
    pub rolls: Vec<u32>,
    pub kept: Vec<bool>,
    pub value: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RollOutcome {
    pub expression: String,
    pub formula: String,
    pub terms: Vec<TermOutcome>,
    pub total: i32,
    /// Kept natural d20 when the expression has exactly one kept d20.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub natural_d20: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dc: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    pub crit: bool,
    pub text: String,
}

impl RollExpr {
    pub fn parse(input: &str) -> Result<Self> {
        let s: String = input
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        if s.is_empty() {
            bail!("dice expression is empty");
        }
        let b = s.as_bytes();
        let mut i = 0;
        let mut terms = Vec::new();
        while i < b.len() {
            let mut negative = false;
            if b[i] == b'+' || b[i] == b'-' {
                negative = b[i] == b'-';
                i += 1;
            } else if !terms.is_empty() {
                bail!("expected `+` or `-` between terms in `{s}`");
            }
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let digits = &s[start..i];
            if i < b.len() && b[i] == b'd' {
                i += 1;
                let count = if digits.is_empty() {
                    1
                } else {
                    number(digits, "dice count")?
                };
                if count == 0 || count > MAX_DICE_PER_TERM {
                    bail!("dice count must be 1..={MAX_DICE_PER_TERM} in `{s}`");
                }
                let sides = if i < b.len() && b[i] == b'%' {
                    i += 1;
                    100
                } else {
                    let st = i;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                    if st == i {
                        bail!("missing die sides in `{s}`");
                    }
                    number(&s[st..i], "die sides")?
                };
                if !STANDARD_SIDES.contains(&sides) {
                    bail!(
                        "d{sides} is not a standard D&D die; use d4, d6, d8, d10, d12, d20, d100 (d%), or d2/d3"
                    );
                }
                let mut keep = Keep::All;
                let mut suffix = String::new();
                if i + 1 < b.len() && matches!(&s[i..i + 2], "kh" | "kl" | "dh" | "dl") {
                    let op = s[i..i + 2].to_string();
                    i += 2;
                    let st = i;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                    let n = if st == i {
                        1
                    } else {
                        number(&s[st..i], "keep/drop count")?
                    };
                    keep = match op.as_str() {
                        "kh" | "kl" if n == 0 || n > count => {
                            bail!("{op}{n}: must keep 1..={count} of {count}d{sides}")
                        }
                        "dh" | "dl" if n == 0 || n >= count => {
                            bail!("{op}{n}: must drop 1..={} of {count}d{sides}", count - 1)
                        }
                        "kh" => Keep::Highest(n),
                        "kl" => Keep::Lowest(n),
                        "dl" => Keep::Highest(count - n),
                        _ => Keep::Lowest(count - n),
                    };
                    suffix = format!("{op}{n}");
                }
                terms.push(Term::Dice(DiceTerm {
                    negative,
                    count,
                    sides,
                    keep,
                    suffix,
                    mode: RollMode::Normal,
                }));
            } else {
                if digits.is_empty() {
                    let found = s[i..]
                        .chars()
                        .next()
                        .map_or("end".to_string(), String::from);
                    bail!("unexpected `{found}` in dice expression `{s}`");
                }
                let v: i32 = digits
                    .parse()
                    .map_err(|_| anyhow::anyhow!("modifier `{digits}` is too large"))?;
                terms.push(Term::Flat(if negative { -v } else { v }));
            }
            if terms.len() > MAX_TERMS {
                bail!("too many terms (maximum {MAX_TERMS})");
            }
        }
        Ok(Self { terms })
    }

    /// Applies advantage/disadvantage to the expression's single plain 1d20.
    pub fn with_mode(mut self, mode: RollMode) -> Result<Self> {
        if mode == RollMode::Normal {
            return Ok(self);
        }
        let d20s: Vec<usize> = self
            .terms
            .iter()
            .enumerate()
            .filter_map(|(i, t)| match t {
                Term::Dice(d) if d.sides == 20 => Some(i),
                _ => None,
            })
            .collect();
        let ok = d20s.len() == 1
            && matches!(&self.terms[d20s[0]], Term::Dice(d) if d.count == 1 && d.keep == Keep::All && !d.negative)
            && self
                .terms
                .iter()
                .all(|t| !matches!(t, Term::Dice(d) if d.sides != 20));
        if !ok {
            bail!("--adv/--dis apply only to a d20 test: exactly one 1d20 plus flat modifiers (e.g. 1d20+5)");
        }
        if let Term::Dice(d) = &mut self.terms[d20s[0]] {
            d.count = 2;
            d.keep = if mode == RollMode::Advantage {
                Keep::Highest(1)
            } else {
                Keep::Lowest(1)
            };
            d.mode = mode;
        }
        Ok(self)
    }

    /// Critical hit: doubles the number of every damage die; modifiers once.
    pub fn critical(mut self) -> Result<Self> {
        for t in &mut self.terms {
            if let Term::Dice(d) = t {
                if d.sides == 20 {
                    bail!("--crit doubles damage dice; a d20 test is not doubled");
                }
                if d.keep != Keep::All {
                    bail!("--crit cannot double a keep/drop term");
                }
                d.count *= 2;
                if d.count > MAX_DICE_PER_TERM * 2 {
                    bail!("too many dice");
                }
            }
        }
        if !self.terms.iter().any(|t| matches!(t, Term::Dice(_))) {
            bail!("--crit needs at least one damage die");
        }
        Ok(self)
    }

    pub fn roll(
        &self,
        expression: &str,
        dc: Option<i32>,
        crit: bool,
        rng: &mut dyn DiceRng,
    ) -> Result<RollOutcome> {
        let mut terms = Vec::new();
        let mut total: i64 = 0;
        let mut text = String::new();
        let mut formula = String::new();
        let mut kept_d20 = Vec::new();
        for (index, term) in self.terms.iter().enumerate() {
            match term {
                Term::Flat(v) => {
                    let piece = if index == 0 {
                        v.to_string()
                    } else {
                        format!("{v:+}")
                    };
                    text.push_str(&piece);
                    formula.push_str(&piece);
                    total += i64::from(*v);
                    terms.push(TermOutcome {
                        kind: "flat",
                        sign: if *v < 0 { -1 } else { 1 },
                        notation: v.to_string(),
                        sides: None,
                        rolls: Vec::new(),
                        kept: Vec::new(),
                        value: i64::from(*v),
                    });
                }
                Term::Dice(d) => {
                    let rolls: Vec<u32> = (0..d.count).map(|_| rng.roll_die(d.sides)).collect();
                    let kept = keep_flags(&rolls, d.keep);
                    let sum: i64 = rolls
                        .iter()
                        .zip(&kept)
                        .filter(|(_, k)| **k)
                        .map(|(r, _)| i64::from(*r))
                        .sum();
                    let value = if d.negative { -sum } else { sum };
                    total += value;
                    if d.sides == 20 {
                        kept_d20.extend(
                            rolls
                                .iter()
                                .zip(&kept)
                                .filter(|(_, k)| **k)
                                .map(|(r, _)| *r),
                        );
                    }
                    let notation = match d.mode {
                        RollMode::Normal => format!("{}d{}{}", d.count, d.sides, d.suffix),
                        _ => "1d20".to_string(),
                    };
                    let sign = if d.negative {
                        "-"
                    } else if index == 0 {
                        ""
                    } else {
                        "+"
                    };
                    let body = match d.mode {
                        RollMode::Normal => rolls
                            .iter()
                            .zip(&kept)
                            .map(|(r, k)| if *k { r.to_string() } else { format!("[{r}]") })
                            .collect::<Vec<_>>()
                            .join(","),
                        mode => {
                            let tag = if mode == RollMode::Advantage {
                                "adv"
                            } else {
                                "dis"
                            };
                            let keptv = rolls
                                .iter()
                                .zip(&kept)
                                .find(|(_, k)| **k)
                                .map(|(r, _)| *r)
                                .unwrap_or(0);
                            format!("{},{} {tag}→{keptv}", rolls[0], rolls[1])
                        }
                    };
                    let _ = write!(text, "{sign}{notation}({body})");
                    let _ = write!(
                        formula,
                        "{sign}{}",
                        match d.mode {
                            RollMode::Advantage => "2d20kh1".to_string(),
                            RollMode::Disadvantage => "2d20kl1".to_string(),
                            RollMode::Normal => notation.clone(),
                        }
                    );
                    terms.push(TermOutcome {
                        kind: "dice",
                        sign: if d.negative { -1 } else { 1 },
                        notation,
                        sides: Some(d.sides),
                        rolls,
                        kept,
                        value,
                    });
                }
            }
        }
        let total = i32::try_from(total).map_err(|_| anyhow::anyhow!("total overflow"))?;
        let only_dice_flat = terms.iter().filter(|t| t.kind == "dice").count();
        if only_dice_flat > 0 || self.terms.len() > 1 {
            let _ = write!(text, "={total}");
        }
        let success = dc.map(|dc| total >= dc);
        if let (Some(dc), Some(ok)) = (dc, success) {
            let _ = write!(text, " vs DC {dc} {}", if ok { "PASS" } else { "FAIL" });
        }
        Ok(RollOutcome {
            expression: expression.to_string(),
            formula,
            terms,
            total,
            natural_d20: (kept_d20.len() == 1).then(|| kept_d20[0]),
            dc,
            success,
            crit,
            text,
        })
    }
}

fn number(text: &str, label: &str) -> Result<u32> {
    text.parse::<u32>()
        .map_err(|_| anyhow::anyhow!("invalid {label} `{text}`"))
}

/// Marks which dice count. Ties keep the earlier die.
fn keep_flags(rolls: &[u32], keep: Keep) -> Vec<bool> {
    let n = match keep {
        Keep::All => return vec![true; rolls.len()],
        Keep::Highest(n) | Keep::Lowest(n) => n as usize,
    };
    let mut order: Vec<usize> = (0..rolls.len()).collect();
    match keep {
        Keep::Highest(_) => order.sort_by(|a, b| rolls[*b].cmp(&rolls[*a]).then(a.cmp(b))),
        _ => order.sort_by(|a, b| rolls[*a].cmp(&rolls[*b]).then(a.cmp(b))),
    }
    let mut flags = vec![false; rolls.len()];
    for &i in order.iter().take(n) {
        flags[i] = true;
    }
    flags
}

/// Parses, applies modes, rolls. Used by the CLI.
pub fn roll_command(
    expression: &str,
    mode: RollMode,
    dc: Option<i32>,
    crit: bool,
    rng: &mut dyn DiceRng,
) -> Result<RollOutcome> {
    let mut expr = RollExpr::parse(expression)?;
    if crit {
        if mode != RollMode::Normal {
            bail!("--crit is for damage; it cannot be combined with --adv/--dis");
        }
        expr = expr.critical()?;
    }
    let expr = expr.with_mode(mode)?;
    expr.roll(expression, dc, crit, rng)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct Seq(VecDeque<u32>, Vec<u32>);
    impl DiceRng for Seq {
        fn roll_die(&mut self, sides: u32) -> u32 {
            self.1.push(sides);
            let v = self.0.pop_front().expect("script ran out");
            assert!((1..=sides).contains(&v));
            v
        }
    }
    fn seq(v: &[u32]) -> Seq {
        Seq(v.iter().copied().collect(), Vec::new())
    }
    fn text(e: &str, mode: RollMode, dc: Option<i32>, crit: bool, v: &[u32]) -> String {
        roll_command(e, mode, dc, crit, &mut seq(v)).unwrap().text
    }
    const N: RollMode = RollMode::Normal;

    #[test]
    fn plain_expressions_keep_the_old_format() {
        assert_eq!(text("1d20+5", N, None, false, &[9]), "1d20(9)+5=14");
        assert_eq!(text("2d8+3", N, None, false, &[3, 6]), "2d8(3,6)+3=12");
        assert_eq!(text("1d8-2", N, None, false, &[5]), "1d8(5)-2=3");
        assert_eq!(text("7", N, None, false, &[]), "7");
        assert_eq!(text("D20", N, None, false, &[4]), "1d20(4)=4");
    }

    #[test]
    fn keep_and_drop() {
        assert_eq!(
            text("2d20kh1", N, None, false, &[3, 17]),
            "2d20kh1([3],17)=17"
        );
        assert_eq!(
            text("2d20kl1+2", N, None, false, &[3, 17]),
            "2d20kl1(3,[17])+2=5"
        );
        assert_eq!(
            text("4d6dl1", N, None, false, &[5, 3, 1, 6]),
            "4d6dl1(5,3,[1],6)=14"
        );
        assert_eq!(
            text("4d6dh1", N, None, false, &[5, 3, 1, 6]),
            "4d6dh1(5,3,1,[6])=9"
        );
        assert_eq!(
            text("4d6kh3", N, None, false, &[2, 2, 2, 6]),
            "4d6kh3(2,2,[2],6)=10"
        );
        assert_eq!(
            text("4d6dl", N, None, false, &[4, 4, 4, 4]),
            "4d6dl1(4,4,4,[4])=12"
        );
    }

    #[test]
    fn multiple_terms_and_negative_dice() {
        assert_eq!(
            text("1d20+1d4+5", N, None, false, &[14, 2]),
            "1d20(14)+1d4(2)+5=21"
        );
        assert_eq!(
            text("1d20-1d4+5", N, None, false, &[14, 2]),
            "1d20(14)-1d4(2)+5=17"
        );
        assert_eq!(
            text("2d6+1d8+2-1", N, None, false, &[1, 2, 3]),
            "2d6(1,2)+1d8(3)+2-1=7"
        );
        assert_eq!(text("3+2", N, None, false, &[]), "3+2=5");
    }

    #[test]
    fn percentile_and_small_dice() {
        let mut r = seq(&[73]);
        let o = roll_command("d%", N, None, false, &mut r).unwrap();
        assert_eq!(o.text, "1d100(73)=73");
        assert_eq!(r.1, vec![100]);
        assert_eq!(text("1d100", N, None, false, &[100]), "1d100(100)=100");
        assert_eq!(text("1d3+1d2", N, None, false, &[3, 2]), "1d3(3)+1d2(2)=5");
    }

    #[test]
    fn advantage_and_dc() {
        assert_eq!(
            text("1d20+5", RollMode::Advantage, None, false, &[3, 17]),
            "1d20(3,17 adv→17)+5=22"
        );
        assert_eq!(
            text("1d20+5", RollMode::Disadvantage, Some(12), false, &[17, 3]),
            "1d20(17,3 dis→3)+5=8 vs DC 12 FAIL"
        );
        let o = roll_command(
            "1d20+2",
            RollMode::Advantage,
            Some(10),
            false,
            &mut seq(&[20, 1]),
        )
        .unwrap();
        assert_eq!(o.natural_d20, Some(20));
        assert_eq!(o.formula, "2d20kh1+2");
        assert_eq!(o.success, Some(true));
        assert_eq!(
            text("1d20+3", N, Some(13), false, &[10]),
            "1d20(10)+3=13 vs DC 13 PASS"
        );
        for bad in ["2d6", "1d20+1d4", "2d20", "1d20kh1"] {
            assert!(
                roll_command(bad, RollMode::Advantage, None, false, &mut seq(&[1, 1, 1])).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn crit_doubles_dice_not_modifiers() {
        let mut r = seq(&[1, 2, 3, 4, 5, 6]);
        let o = roll_command("2d8+1d6+3", N, None, true, &mut r).unwrap();
        assert_eq!(o.text, "4d8(1,2,3,4)+2d6(5,6)+3=24");
        assert_eq!(o.formula, "4d8+2d6+3");
        assert_eq!(r.1, vec![8, 8, 8, 8, 6, 6]);
        assert!(o.crit);
        assert!(roll_command("1d20+5", N, None, true, &mut seq(&[1])).is_err());
        assert!(roll_command("4d6dl1", N, None, true, &mut seq(&[1])).is_err());
        assert!(roll_command("5", N, None, true, &mut seq(&[])).is_err());
        assert!(roll_command("1d6", RollMode::Advantage, None, true, &mut seq(&[1])).is_err());
    }

    #[test]
    fn rejects_invalid_and_nonstandard() {
        for bad in [
            "",
            "d",
            "2d",
            "0d6",
            "d0",
            "1d7",
            "1d1",
            "1d5",
            "1d1000",
            "1d6+",
            "1d6++2",
            "abc",
            "101d6",
            "2d20kh3",
            "2d20kh0",
            "2d20dl2",
            "4d6dl0",
            "1d6x",
            "1d6+2147483648",
            "1d%%",
        ] {
            assert!(RollExpr::parse(bad).is_err(), "`{bad}` should be rejected");
        }
    }

    #[test]
    fn seeded_rolls_are_reproducible() {
        let mut a = crate::dice::seeded_rng(42);
        let mut b = crate::dice::seeded_rng(42);
        let x = roll_command("4d6dl1+1d20+3", N, None, false, &mut a).unwrap();
        let y = roll_command("4d6dl1+1d20+3", N, None, false, &mut b).unwrap();
        assert_eq!(x, y);
        let kept: usize = x.terms[0].kept.iter().filter(|k| **k).count();
        assert_eq!(kept, 3);
    }
}
