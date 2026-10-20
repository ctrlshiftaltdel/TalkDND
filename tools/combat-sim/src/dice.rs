//! Dice expressions and random-number plumbing.
//!
//! Expressions are parsed once (for example from CLI input or data files) and
//! can then be rolled repeatedly against any [`DiceRng`].

use std::fmt;
use std::str::FromStr;

use rand::rngs::{OsRng, StdRng};
use rand::{Rng, RngCore, SeedableRng, TryRngCore};

/// A parsed dice expression such as `2d8+1d6+3`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiceExpr {
    /// `(count, sides)` pairs in the order they are rolled.
    pub terms: Vec<(u32, u32)>,
    /// Flat modifier added once after all dice are rolled.
    pub modifier: i32,
}

impl DiceExpr {
    /// Rolls every die in the expression, in order.
    pub fn roll<R: DiceRng + ?Sized>(&self, rng: &mut R) -> RollRecord {
        self.roll_terms(&self.terms, rng)
    }

    /// Rolls the expression with every dice term doubled.
    ///
    /// This is the critical-hit form: the number of dice is doubled, not the
    /// rolled result, and the modifier is still added once. For example,
    /// `2d8+3` becomes `4d8+3`.
    pub fn roll_doubled<R: DiceRng + ?Sized>(&self, rng: &mut R) -> RollRecord {
        let doubled: Vec<(u32, u32)> = self
            .terms
            .iter()
            .map(|&(count, sides)| {
                let count = count
                    .checked_mul(2)
                    .expect("dice count overflow while doubling");
                (count, sides)
            })
            .collect();
        self.roll_terms(&doubled, rng)
    }

    /// Expected value of the expression, used by the simulation AI.
    pub fn average(&self) -> f64 {
        let dice: f64 = self
            .terms
            .iter()
            .map(|&(count, sides)| f64::from(count) * (f64::from(sides) + 1.0) / 2.0)
            .sum();
        dice + f64::from(self.modifier)
    }

    /// Expected value with every dice term doubled (critical hit).
    pub fn average_doubled(&self) -> f64 {
        let dice: f64 = self
            .terms
            .iter()
            .map(|&(count, sides)| 2.0 * f64::from(count) * (f64::from(sides) + 1.0) / 2.0)
            .sum();
        dice + f64::from(self.modifier)
    }

    /// Rolls the expression without building a record, formula or dice list.
    pub fn roll_total<R: DiceRng + ?Sized>(&self, rng: &mut R) -> i32 {
        self.roll_total_terms(&self.terms, rng)
    }

    /// Rolls the critical-hit form without allocating.
    pub fn roll_doubled_total<R: DiceRng + ?Sized>(&self, rng: &mut R) -> i32 {
        let mut sum = i64::from(self.modifier);
        for &(count, sides) in &self.terms {
            let count = count
                .checked_mul(2)
                .expect("dice count overflow while doubling");
            for _ in 0..count {
                sum += i64::from(rng.roll_die(sides));
            }
        }
        i32::try_from(sum).expect("dice total overflowed i32")
    }

    fn roll_total_terms<R: DiceRng + ?Sized>(&self, terms: &[(u32, u32)], rng: &mut R) -> i32 {
        let mut sum = i64::from(self.modifier);
        for &(count, sides) in terms {
            for _ in 0..count {
                sum += i64::from(rng.roll_die(sides));
            }
        }
        i32::try_from(sum).expect("dice total overflowed i32")
    }

    fn roll_terms<R: DiceRng + ?Sized>(&self, terms: &[(u32, u32)], rng: &mut R) -> RollRecord {
        let mut dice = Vec::new();
        let mut sum = i64::from(self.modifier);
        for &(count, sides) in terms {
            for _ in 0..count {
                let value = rng.roll_die(sides);
                let value = i32::try_from(value).expect("die result does not fit in i32");
                dice.push(value);
                sum += i64::from(value);
            }
        }
        let total = i32::try_from(sum).expect("dice total overflowed i32");
        let formula = format_formula(terms, self.modifier);
        let text = if dice.is_empty() {
            total.to_string()
        } else {
            format!(
                "{}({}){}={total}",
                format_terms(terms),
                join_numbers(&dice),
                modifier_text(self.modifier)
            )
        };
        RollRecord {
            formula,
            dice,
            modifier: self.modifier,
            total,
            text,
        }
    }
}

impl FromStr for DiceExpr {
    type Err = DiceParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let cleaned: String = input.chars().filter(|ch| !ch.is_whitespace()).collect();
        if cleaned.is_empty() {
            return Err(DiceParseError::new("dice expression is empty"));
        }

        let bytes = cleaned.as_bytes();
        let mut index = 0;
        let mut terms = Vec::new();
        let mut modifier = 0i32;
        let mut first_term = true;

        while index < bytes.len() {
            let mut sign = 1i32;
            if matches!(bytes[index], b'+' | b'-') {
                if bytes[index] == b'-' {
                    sign = -1;
                }
                index += 1;
                if index == bytes.len() {
                    return Err(DiceParseError::new(format!(
                        "trailing operator in dice expression `{cleaned}`"
                    )));
                }
            } else if !first_term {
                return Err(DiceParseError::new(format!(
                    "expected `+` or `-` between terms in `{cleaned}`"
                )));
            }

            let digits_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }

            if index < bytes.len() && matches!(bytes[index], b'd' | b'D') {
                let count = if digits_start == index {
                    1
                } else {
                    parse_dice_value(&cleaned[digits_start..index], "dice count")?
                };
                index += 1;
                let sides_start = index;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
                if sides_start == index {
                    return Err(DiceParseError::new(format!(
                        "missing die sides in dice expression `{cleaned}`"
                    )));
                }
                let sides = parse_dice_value(&cleaned[sides_start..index], "die sides")?;
                if sign < 0 {
                    return Err(DiceParseError::new(format!(
                        "dice terms cannot be negative in `{cleaned}`"
                    )));
                }
                terms.push((count, sides));
            } else {
                if digits_start == index {
                    let found = cleaned[index..].chars().next().unwrap_or('?');
                    return Err(DiceParseError::new(format!(
                        "unexpected character `{found}` in dice expression `{cleaned}`"
                    )));
                }
                let value = parse_modifier(&cleaned[digits_start..index], sign, &cleaned)?;
                modifier = modifier.checked_add(value).ok_or_else(|| {
                    DiceParseError::new(format!("modifier overflow in dice expression `{cleaned}`"))
                })?;
            }
            first_term = false;
        }

        Ok(Self { terms, modifier })
    }
}

impl fmt::Display for DiceExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format_formula(&self.terms, self.modifier))
    }
}

fn parse_dice_value(text: &str, label: &str) -> Result<u32, DiceParseError> {
    let value = text
        .parse::<u32>()
        .map_err(|_| DiceParseError::new(format!("invalid or overflowing {label} `{text}`")))?;
    if value == 0 {
        return Err(DiceParseError::new(format!("{label} must be at least 1")));
    }
    if value > i32::MAX as u32 {
        return Err(DiceParseError::new(format!(
            "{label} `{text}` is too large (maximum {})",
            i32::MAX
        )));
    }
    Ok(value)
}

fn parse_modifier(text: &str, sign: i32, full: &str) -> Result<i32, DiceParseError> {
    let magnitude = text.parse::<i64>().map_err(|_| {
        DiceParseError::new(format!("invalid modifier in dice expression `{full}`"))
    })?;
    let value = magnitude
        .checked_mul(i64::from(sign))
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| {
            DiceParseError::new(format!("modifier overflow in dice expression `{full}`"))
        })?;
    Ok(value)
}

fn format_terms(terms: &[(u32, u32)]) -> String {
    terms
        .iter()
        .map(|(count, sides)| format!("{count}d{sides}"))
        .collect::<Vec<_>>()
        .join("+")
}

fn format_formula(terms: &[(u32, u32)], modifier: i32) -> String {
    let mut formula = format_terms(terms);
    if formula.is_empty() {
        formula.push_str(&modifier.to_string());
    } else if modifier != 0 {
        formula.push_str(&modifier_text(modifier));
    }
    formula
}

fn modifier_text(modifier: i32) -> String {
    if modifier == 0 {
        String::new()
    } else {
        format!("{modifier:+}")
    }
}

fn join_numbers(values: &[i32]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// Invalid dice-expression text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiceParseError {
    message: String,
}

impl DiceParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for DiceParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DiceParseError {}

/// Minimal random-number source needed by the dice module.
///
/// The blanket implementation below gives every [`RngCore`] this interface, so
/// production code can use `StdRng`/`OsRng` and tests can use a scripted mock.
pub trait DiceRng {
    /// Rolls one die with `sides` sides, returning a value in `1..=sides`.
    fn roll_die(&mut self, sides: u32) -> u32;
}

impl<R: RngCore + ?Sized> DiceRng for R {
    fn roll_die(&mut self, sides: u32) -> u32 {
        debug_assert!(sides > 0, "die must have at least one side");
        self.random_range(1..=sides)
    }
}

/// `RngCore` adapter around the operating-system RNG.
///
/// `rand` 0.9 exposes `OsRng` through [`TryRngCore`] rather than [`RngCore`],
/// so this wrapper makes it usable with the same [`DiceRng`] blanket impl as
/// every other generator.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsDiceRng;

impl RngCore for OsDiceRng {
    fn next_u32(&mut self) -> u32 {
        OsRng
            .try_next_u32()
            .unwrap_or_else(|error| panic!("operating system RNG failed: {error}"))
    }

    fn next_u64(&mut self) -> u64 {
        OsRng
            .try_next_u64()
            .unwrap_or_else(|error| panic!("operating system RNG failed: {error}"))
    }

    fn fill_bytes(&mut self, destination: &mut [u8]) {
        OsRng
            .try_fill_bytes(destination)
            .unwrap_or_else(|error| panic!("operating system RNG failed: {error}"));
    }
}

/// RNG for step-by-step commands: fresh OS entropy for every roll.
pub fn step_rng() -> OsDiceRng {
    OsDiceRng
}

/// RNG for a simulation: a fast generator seeded from the OS once.
pub fn sim_rng() -> StdRng {
    StdRng::from_os_rng()
}

/// Reproducible RNG used only when `--seed` is explicitly supplied.
pub fn seeded_rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

/// RNG for one simulation worker.
///
/// With a base seed the stream is derived deterministically from the worker
/// index, so runs are reproducible. Without a seed every worker is seeded from
/// the operating system.
pub fn thread_rng(base_seed: Option<u64>, thread_index: usize) -> StdRng {
    match base_seed {
        Some(seed) => StdRng::seed_from_u64(derive_seed(seed, thread_index)),
        None => StdRng::from_os_rng(),
    }
}

fn derive_seed(base_seed: u64, thread_index: usize) -> u64 {
    let mut state =
        base_seed.wrapping_add((thread_index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    state = (state ^ (state >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    state = (state ^ (state >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    state ^ (state >> 31)
}

/// A completed roll, including the raw dice for auditing and logging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollRecord {
    /// Canonical expression, e.g. `4d8+3` after a critical hit.
    pub formula: String,
    /// Raw die results in roll order.
    pub dice: Vec<i32>,
    /// Flat modifier applied once.
    pub modifier: i32,
    /// Dice sum plus modifier.
    pub total: i32,
    /// Human-readable roll, e.g. `2d8(3,6)+3=12`.
    pub text: String,
}

/// Advantage/disadvantage state for a d20 roll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RollMode {
    /// Roll one d20.
    #[default]
    Normal,
    /// Roll two d20s and keep the higher.
    Advantage,
    /// Roll two d20s and keep the lower.
    Disadvantage,
}

impl RollMode {
    /// Resolves the two CLI flags. Supplying both cancels to [`RollMode::Normal`].
    pub fn from_flags(advantage: bool, disadvantage: bool) -> Self {
        match (advantage, disadvantage) {
            (true, true) | (false, false) => Self::Normal,
            (true, false) => Self::Advantage,
            (false, true) => Self::Disadvantage,
        }
    }
}

/// Result of a d20 check, retention of the raw dice and natural-roll flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D20Roll {
    /// Mode used for the roll.
    pub mode: RollMode,
    /// Raw dice in roll order: one die normally, two with advantage/disadvantage.
    pub dice: Vec<i32>,
    /// The die that was kept.
    pub kept: i32,
    /// Modifier added to the kept die.
    pub modifier: i32,
    /// Kept die plus modifier.
    pub total: i32,
    /// The kept die was a natural 20.
    pub nat20: bool,
    /// The kept die was a natural 1.
    pub nat1: bool,
}

/// Result of a d20 check without the raw-dice record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D20Total {
    /// The die that was kept.
    pub kept: i32,
    /// Kept die plus modifier.
    pub total: i32,
    /// The kept die was a natural 1.
    pub nat1: bool,
    /// The kept die was a natural 20.
    pub nat20: bool,
}

impl D20Roll {
    /// Rolls a d20 check without allocating a [`D20Roll`] record.
    pub fn roll_total<R: DiceRng + ?Sized>(modifier: i32, mode: RollMode, rng: &mut R) -> D20Total {
        let first = roll_d20_die(rng);
        let kept = match mode {
            RollMode::Normal => first,
            RollMode::Advantage => first.max(roll_d20_die(rng)),
            RollMode::Disadvantage => first.min(roll_d20_die(rng)),
        };
        let total = kept + modifier;
        D20Total {
            kept,
            total,
            nat1: kept == 1,
            nat20: kept == 20,
        }
    }

    /// Rolls a d20 check with the requested mode.
    pub fn roll<R: DiceRng + ?Sized>(modifier: i32, mode: RollMode, rng: &mut R) -> Self {
        let first = roll_d20_die(rng);
        let mut dice = vec![first];
        let kept = match mode {
            RollMode::Normal => first,
            RollMode::Advantage => {
                let second = roll_d20_die(rng);
                dice.push(second);
                first.max(second)
            }
            RollMode::Disadvantage => {
                let second = roll_d20_die(rng);
                dice.push(second);
                first.min(second)
            }
        };
        let total = kept + modifier;
        Self {
            mode,
            dice,
            kept,
            modifier,
            total,
            nat20: kept == 20,
            nat1: kept == 1,
        }
    }

    /// Base text, e.g. `1d20(3,17 adv→17)+5=22`.
    pub fn text(&self) -> String {
        let mode_suffix = match self.mode {
            RollMode::Normal => String::new(),
            RollMode::Advantage => format!(" adv→{}", self.kept),
            RollMode::Disadvantage => format!(" dis→{}", self.kept),
        };
        format!(
            "1d20({}{}){}={}",
            join_numbers(&self.dice),
            mode_suffix,
            modifier_text(self.modifier),
            self.total
        )
    }

    /// Appends an attack comparison, e.g. ` vs AC 15 HIT`.
    ///
    /// Whether the attack hits (including auto-hit/auto-miss rules) is the
    /// caller's decision; this helper only formats it.
    pub fn vs_ac(&self, ac: i32, hit: bool) -> String {
        format!(
            "{} vs AC {ac} {}",
            self.text(),
            if hit { "HIT" } else { "MISS" }
        )
    }

    /// Appends a saving-throw comparison against `dc`.
    pub fn vs_dc(&self, dc: i32) -> String {
        format!(
            "{} vs DC {dc} {}",
            self.text(),
            if self.total >= dc { "PASS" } else { "FAIL" }
        )
    }
}

fn roll_d20_die<R: DiceRng + ?Sized>(rng: &mut R) -> i32 {
    i32::try_from(rng.roll_die(20)).expect("d20 result does not fit in i32")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Returns a fixed sequence of die results and remembers which dice were asked for.
    struct SequenceRng {
        values: VecDeque<u32>,
        sides_seen: Vec<u32>,
    }

    impl SequenceRng {
        fn new(values: &[u32]) -> Self {
            Self {
                values: values.iter().copied().collect(),
                sides_seen: Vec::new(),
            }
        }
    }

    impl DiceRng for SequenceRng {
        fn roll_die(&mut self, sides: u32) -> u32 {
            self.sides_seen.push(sides);
            self.values
                .pop_front()
                .expect("mock RNG ran out of scripted values")
        }
    }

    #[test]
    fn parses_supported_forms() {
        let standard: DiceExpr = " 2d8 + 1d6 - 3 ".parse().unwrap();
        assert_eq!(standard.terms, vec![(2, 8), (1, 6)]);
        assert_eq!(standard.modifier, -3);
        assert_eq!(standard.to_string(), "2d8+1d6-3");

        let implicit_count: DiceExpr = "D20".parse().unwrap();
        assert_eq!(implicit_count.terms, vec![(1, 20)]);
        assert_eq!(implicit_count.modifier, 0);
        assert_eq!(implicit_count.to_string(), "1d20");

        let flat: DiceExpr = "-2".parse().unwrap();
        assert!(flat.terms.is_empty());
        assert_eq!(flat.modifier, -2);
        assert_eq!(flat.to_string(), "-2");

        let zero: DiceExpr = "0".parse().unwrap();
        assert!(zero.terms.is_empty());
        assert_eq!(zero.modifier, 0);
        assert_eq!(zero.to_string(), "0");
    }

    #[test]
    fn rejects_invalid_expressions() {
        for input in [
            "",
            "   ",
            "d",
            "2d",
            "0d6",
            "d0",
            "1d6+",
            "1d6++2",
            "abc",
            "-1d6",
            "1d6+-2",
            "4294967296d6",
            "1d4294967296",
            "1d6+2147483648",
        ] {
            assert!(
                input.parse::<DiceExpr>().is_err(),
                "expected `{input}` to be rejected"
            );
        }
    }

    #[test]
    fn formats_rolls_with_raw_dice() {
        let expr: DiceExpr = "2d8+3".parse().unwrap();
        let mut rng = SequenceRng::new(&[3, 6]);
        let roll = expr.roll(&mut rng);
        assert_eq!(roll.formula, "2d8+3");
        assert_eq!(roll.dice, vec![3, 6]);
        assert_eq!(roll.modifier, 3);
        assert_eq!(roll.total, 12);
        assert_eq!(roll.text, "2d8(3,6)+3=12");
        assert_eq!(rng.sides_seen, vec![8, 8]);

        let expr: DiceExpr = "1d20+6".parse().unwrap();
        let roll = expr.roll(&mut SequenceRng::new(&[14]));
        assert_eq!(roll.text, "1d20(14)+6=20");

        let expr: DiceExpr = "1d8-2".parse().unwrap();
        let roll = expr.roll(&mut SequenceRng::new(&[5]));
        assert_eq!(roll.text, "1d8(5)-2=3");
    }

    #[test]
    fn flat_modifiers_roll_without_dice() {
        let expr: DiceExpr = "7".parse().unwrap();
        let mut rng = SequenceRng::new(&[]);
        let roll = expr.roll(&mut rng);
        assert_eq!(roll.formula, "7");
        assert!(roll.dice.is_empty());
        assert_eq!(roll.modifier, 7);
        assert_eq!(roll.total, 7);
        assert_eq!(roll.text, "7");
        assert!(rng.sides_seen.is_empty());
    }

    #[test]
    fn advantage_and_disadvantage_keep_the_right_die() {
        let roll = D20Roll::roll(5, RollMode::Advantage, &mut SequenceRng::new(&[3, 17]));
        assert_eq!(roll.dice, vec![3, 17]);
        assert_eq!(roll.kept, 17);
        assert_eq!(roll.total, 22);
        assert_eq!(roll.text(), "1d20(3,17 adv→17)+5=22");

        let roll = D20Roll::roll(5, RollMode::Disadvantage, &mut SequenceRng::new(&[17, 3]));
        assert_eq!(roll.dice, vec![17, 3]);
        assert_eq!(roll.kept, 3);
        assert_eq!(roll.total, 8);
        assert_eq!(roll.text(), "1d20(17,3 dis→3)+5=8");

        let roll = D20Roll::roll(6, RollMode::Normal, &mut SequenceRng::new(&[14]));
        assert_eq!(roll.text(), "1d20(14)+6=20");
    }

    #[test]
    fn advantage_and_disadvantage_cancel() {
        assert_eq!(RollMode::from_flags(true, true), RollMode::Normal);
        assert_eq!(RollMode::from_flags(false, false), RollMode::Normal);
        assert_eq!(RollMode::from_flags(true, false), RollMode::Advantage);
        assert_eq!(RollMode::from_flags(false, true), RollMode::Disadvantage);

        let mut rng = SequenceRng::new(&[12]);
        let roll = D20Roll::roll(3, RollMode::from_flags(true, true), &mut rng);
        assert_eq!(roll.dice, vec![12]);
        assert_eq!(rng.sides_seen, vec![20]);
    }

    #[test]
    fn natural_roll_flags_follow_the_kept_die() {
        let roll = D20Roll::roll(0, RollMode::Normal, &mut SequenceRng::new(&[20]));
        assert!(roll.nat20);
        assert!(!roll.nat1);

        let roll = D20Roll::roll(0, RollMode::Normal, &mut SequenceRng::new(&[1]));
        assert!(roll.nat1);
        assert!(!roll.nat20);

        let roll = D20Roll::roll(0, RollMode::Advantage, &mut SequenceRng::new(&[20, 3]));
        assert!(roll.nat20);
        assert!(!roll.nat1);

        let roll = D20Roll::roll(0, RollMode::Disadvantage, &mut SequenceRng::new(&[20, 3]));
        assert!(!roll.nat20);
        assert!(!roll.nat1);
        assert_eq!(roll.kept, 3);
    }

    #[test]
    fn check_formatting_appends_target_results() {
        let roll = D20Roll::roll(6, RollMode::Normal, &mut SequenceRng::new(&[14]));
        assert_eq!(roll.vs_ac(15, true), "1d20(14)+6=20 vs AC 15 HIT");
        assert_eq!(roll.vs_ac(21, false), "1d20(14)+6=20 vs AC 21 MISS");
        assert_eq!(roll.vs_dc(20), "1d20(14)+6=20 vs DC 20 PASS");
        assert_eq!(roll.vs_dc(21), "1d20(14)+6=20 vs DC 21 FAIL");
    }

    #[test]
    fn critical_doubles_dice_but_not_modifier() {
        let expr: DiceExpr = "2d8+3".parse().unwrap();
        let mut rng = SequenceRng::new(&[1, 2, 3, 4]);
        let roll = expr.roll_doubled(&mut rng);
        assert_eq!(roll.formula, "4d8+3");
        assert_eq!(roll.dice, vec![1, 2, 3, 4]);
        assert_eq!(roll.modifier, 3);
        assert_eq!(roll.total, 13);
        assert_eq!(roll.text, "4d8(1,2,3,4)+3=13");
        assert_eq!(rng.sides_seen, vec![8, 8, 8, 8]);
    }

    #[test]
    fn quiet_rolls_match_recorded_totals() {
        let expr: DiceExpr = "2d8+1d6+3".parse().unwrap();
        let record = expr.roll(&mut SequenceRng::new(&[2, 5, 4]));
        let total = expr.roll_total(&mut SequenceRng::new(&[2, 5, 4]));
        assert_eq!(record.total, total);

        let doubled = expr.roll_doubled(&mut SequenceRng::new(&[1, 2, 3, 4, 5, 6]));
        let doubled_total = expr.roll_doubled_total(&mut SequenceRng::new(&[1, 2, 3, 4, 5, 6]));
        assert_eq!(doubled.total, doubled_total);

        assert_eq!(expr.average(), 2.0 * 4.5 + 3.5 + 3.0);
        assert_eq!(expr.average_doubled(), 4.0 * 4.5 + 2.0 * 3.5 + 3.0);
    }

    #[test]
    fn quiet_d20_total_matches_roll() {
        for mode in [
            RollMode::Normal,
            RollMode::Advantage,
            RollMode::Disadvantage,
        ] {
            let record = D20Roll::roll(5, mode, &mut SequenceRng::new(&[3, 17]));
            let total = D20Roll::roll_total(5, mode, &mut SequenceRng::new(&[3, 17]));
            assert_eq!(record.total, total.total);
            assert_eq!(record.kept, total.kept);
            assert_eq!(record.nat1, total.nat1);
            assert_eq!(record.nat20, total.nat20);
        }
    }

    #[test]
    fn seeded_rng_is_reproducible() {
        let expr: DiceExpr = "20d6+5".parse().unwrap();
        let mut first = seeded_rng(1234);
        let mut second = seeded_rng(1234);
        assert_eq!(expr.roll(&mut first), expr.roll(&mut second));

        let mut different = seeded_rng(4321);
        assert_ne!(expr.roll(&mut first), expr.roll(&mut different));
    }

    #[test]
    fn thread_rng_is_deterministic_per_worker() {
        let expr: DiceExpr = "10d10".parse().unwrap();
        let mut first = thread_rng(Some(7), 3);
        let mut second = thread_rng(Some(7), 3);
        assert_eq!(expr.roll(&mut first), expr.roll(&mut second));

        let mut other_worker = thread_rng(Some(7), 4);
        assert_ne!(
            expr.roll(&mut thread_rng(Some(7), 3)),
            expr.roll(&mut other_worker)
        );
    }
}
