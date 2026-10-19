//! Expression templates with `{var}` substitution.
//!
//! Content files describe numbers and dice as text such as `"1d10+{level}"` or
//! `"8+{attack_mod}+{prof}"`. This module substitutes a context, normalises the
//! sign runs produced by negative modifiers, and parses the result with the
//! existing [`DiceExpr`] parser.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};

use crate::dice::DiceExpr;

/// The variables available to content expressions.
pub const VARIABLES: &[&str] = &[
    "str",
    "dex",
    "con",
    "int",
    "wis",
    "cha",
    "prof",
    "level",
    "spell_mod",
    "spell_attack",
    "spell_dc",
    "slot_level",
    "attack_mod",
];

/// Values used when evaluating an expression for a specific creature/action.
#[derive(Debug, Clone, PartialEq)]
pub struct ExprContext {
    pub ability_mods: BTreeMap<String, i32>,
    pub prof: i32,
    pub level: i32,
    pub spell_mod: Option<i32>,
    pub spell_attack: Option<i32>,
    pub spell_dc: Option<i32>,
    pub slot_level: Option<i32>,
    pub attack_mod: Option<i32>,
    pub params: BTreeMap<String, String>,
}

impl Default for ExprContext {
    fn default() -> Self {
        Self {
            ability_mods: BTreeMap::new(),
            prof: 0,
            level: 0,
            spell_mod: None,
            spell_attack: None,
            spell_dc: None,
            slot_level: None,
            attack_mod: None,
            params: BTreeMap::new(),
        }
    }
}

impl ExprContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fills ability modifiers from ability scores (10 => +0).
    pub fn with_ability_scores(mut self, scores: &BTreeMap<String, i32>) -> Self {
        for ability in ["str", "dex", "con", "int", "wis", "cha"] {
            let score = scores.get(ability).copied().unwrap_or(10);
            self.ability_mods.insert(ability.to_string(), (score - 10).div_euclid(2));
        }
        self
    }

    pub fn with_prof(mut self, prof: i32) -> Self {
        self.prof = prof;
        self
    }

    pub fn with_level(mut self, level: i32) -> Self {
        self.level = level;
        self
    }

    pub fn with_spellcasting(
        mut self,
        spell_mod: Option<i32>,
        spell_attack: Option<i32>,
        spell_dc: Option<i32>,
    ) -> Self {
        self.spell_mod = spell_mod;
        self.spell_attack = spell_attack;
        self.spell_dc = spell_dc;
        self
    }

    pub fn with_slot_level(mut self, slot_level: i32) -> Self {
        self.slot_level = Some(slot_level);
        self
    }

    pub fn with_attack_mod(mut self, attack_mod: i32) -> Self {
        self.attack_mod = Some(attack_mod);
        self
    }

    pub fn with_params(mut self, params: BTreeMap<String, String>) -> Self {
        self.params = params;
        self
    }

    /// Evaluates a template, returning the normalised substituted formula (for
    /// logs) and the parsed dice expression.
    pub fn evaluate(&self, template: &str) -> Result<(String, DiceExpr)> {
        let substituted = substitute(template, |name| self.variable(name))?;
        let formula = normalize_signs(&substituted);
        let expression = formula.parse::<DiceExpr>().with_context(|| {
            format!("invalid expression `{template}` (substituted to `{formula}`)")
        })?;
        Ok((formula, expression))
    }

    fn variable(&self, name: &str) -> Result<String> {
        if let Some(value) = self.ability_mods.get(name) {
            return Ok(value.to_string());
        }
        match name {
            "str" | "dex" | "con" | "int" | "wis" | "cha" => bail!(
                "missing ability modifier `{name}` in expression context"
            ),
            "prof" => Ok(self.prof.to_string()),
            "level" => Ok(self.level.to_string()),
            "spell_mod" => optional_variable(name, self.spell_mod),
            "spell_attack" => optional_variable(name, self.spell_attack),
            "spell_dc" => optional_variable(name, self.spell_dc),
            "slot_level" => optional_variable(name, self.slot_level),
            "attack_mod" => optional_variable(name, self.attack_mod),
            _ => {
                if let Some(param) = name.strip_prefix("param.") {
                    if param.is_empty() {
                        return Err(unknown_variable(name));
                    }
                    return self
                        .params
                        .get(param)
                        .cloned()
                        .ok_or_else(|| anyhow!("missing parameter `{param}` in expression context"));
                }
                Err(unknown_variable(name))
            }
        }
    }
}

fn optional_variable(name: &str, value: Option<i32>) -> Result<String> {
    value
        .map(|value| value.to_string())
        .ok_or_else(|| anyhow!("missing variable `{name}` in expression context"))
}

fn unknown_variable(name: &str) -> anyhow::Error {
    anyhow!(
        "unknown expression variable `{name}`; allowed variables: {} and `param.<name>`",
        VARIABLES.join(", ")
    )
}

/// Evaluates `template` with an explicit context.
pub fn evaluate(template: &str, context: &ExprContext) -> Result<(String, DiceExpr)> {
    context.evaluate(template)
}

/// Statically validates an expression template by substituting `0` for every
/// allowed variable and parsing the result.
pub fn validate_template(template: &str) -> Result<()> {
    let substituted = substitute(template, |name| {
        if VARIABLES.contains(&name) {
            Ok("0".to_string())
        } else if let Some(param) = name.strip_prefix("param.") {
            if param.is_empty() {
                Err(unknown_variable(name))
            } else {
                Ok("0".to_string())
            }
        } else {
            Err(unknown_variable(name))
        }
    })?;
    let formula = normalize_signs(&substituted);
    formula.parse::<DiceExpr>().with_context(|| {
        format!("invalid expression `{template}` (substituted to `{formula}`)")
    })?;
    Ok(())
}

/// Replaces `{name}` occurrences, rejecting unknown variables and unmatched
/// braces.
fn substitute<F>(template: &str, mut lookup: F) -> Result<String>
where
    F: FnMut(&str) -> Result<String>,
{
    let mut output = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let before = &rest[..open];
        if let Some(close) = before.find('}') {
            return Err(unmatched_close(template, close));
        }
        output.push_str(before);
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or_else(|| {
            anyhow!("unmatched `{{` in expression `{template}`")
        })?;
        if let Some(inner_open) = after[..close].find('{') {
            let _ = inner_open;
            return Err(anyhow!(
                "nested `{{` in expression `{template}`; braces may not nest"
            ));
        }
        let name = after[..close].trim();
        if name.is_empty() {
            return Err(anyhow!(
                "empty variable name `{{}}` in expression `{template}`"
            ));
        }
        let value = lookup(name)?;
        output.push_str(&value);
        rest = &after[close + 1..];
    }
    if let Some(close) = rest.find('}') {
        return Err(unmatched_close(template, close));
    }
    output.push_str(rest);
    Ok(output)
}

fn unmatched_close(template: &str, _offset: usize) -> anyhow::Error {
    anyhow!("unmatched `}}` in expression `{template}`")
}

/// Collapses sign runs left behind by substituting negative modifiers, and
/// removes whitespace (the dice parser ignores it anyway).
fn normalize_signs(input: &str) -> String {
    let mut compact: String = input.chars().filter(|ch| !ch.is_whitespace()).collect();
    loop {
        let next = compact
            .replace("+-", "-")
            .replace("-+", "-")
            .replace("--", "+")
            .replace("++", "+");
        if next == compact {
            return compact;
        }
        compact = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> ExprContext {
        let mut scores = BTreeMap::new();
        scores.insert("str".to_string(), 18);
        scores.insert("dex".to_string(), 8);
        ExprContext::new()
            .with_ability_scores(&scores)
            .with_prof(2)
            .with_level(5)
    }

    #[test]
    fn substitutes_dice_and_level() {
        let (formula, expression) = context().evaluate("1d10+{level}").unwrap();
        assert_eq!(formula, "1d10+5");
        assert_eq!(expression.terms, vec![(1, 10)]);
        assert_eq!(expression.modifier, 5);
    }

    #[test]
    fn substitutes_flat_sums() {
        let context = context().with_attack_mod(0);
        let (formula, expression) = context.evaluate("8+{attack_mod}+{prof}").unwrap();
        assert_eq!(formula, "8+0+2");
        assert_eq!(expression.modifier, 10);
    }

    #[test]
    fn negative_ability_modifiers_are_normalised() {
        let context = context().with_attack_mod(-1);
        let (formula, expression) = context.evaluate("8+{attack_mod}+{prof}").unwrap();
        assert_eq!(formula, "8-1+2");
        assert_eq!(expression.modifier, 9);

        let (formula, expression) = context.evaluate("1d6+{dex}").unwrap();
        assert_eq!(formula, "1d6-1");
        assert_eq!(expression.modifier, -1);

        let (formula, expression) = context.evaluate("1d6-{dex}").unwrap();
        assert_eq!(formula, "1d6+1");
        assert_eq!(expression.modifier, 1);
    }

    #[test]
    fn substitutes_parameter_dice() {
        let mut params = BTreeMap::new();
        params.insert("dice".to_string(), "2d6".to_string());
        let context = context().with_params(params);
        let (formula, expression) = context.evaluate("{param.dice}").unwrap();
        assert_eq!(formula, "2d6");
        assert_eq!(expression.terms, vec![(2, 6)]);
    }

    #[test]
    fn missing_variable_is_an_error() {
        let error = context().evaluate("2d8+{spell_mod}").unwrap_err();
        assert!(error.to_string().contains("spell_mod"), "{error}");
    }

    #[test]
    fn unknown_variable_is_an_error() {
        let error = context().evaluate("1d6+{nope}").unwrap_err();
        assert!(error.to_string().contains("unknown expression variable"), "{error}");
        validate_template("1d6+{nope}").unwrap_err();
    }

    #[test]
    fn malformed_braces_are_errors() {
        for template in ["1d6+{level", "1d6+level}", "1d6+{}", "1d6+{{level}}"] {
            assert!(
                validate_template(template).is_err(),
                "`{template}` should be rejected"
            );
        }
    }

    #[test]
    fn validates_real_content_shapes() {
        let mut params = BTreeMap::new();
        params.insert("dice".to_string(), "1d6".to_string());
        let context = context()
            .with_spellcasting(Some(3), Some(5), Some(13))
            .with_slot_level(1)
            .with_params(params);
        assert_eq!(
            context.evaluate("8+{attack_mod}+{prof}").unwrap().0,
            "8+0+2"
        );
        assert_eq!(context.evaluate("1d10+{level}").unwrap().0, "1d10+5");
        assert_eq!(context.evaluate("2d8+{spell_mod}").unwrap().0, "2d8+3");
        assert_eq!(context.evaluate("{spell_dc}").unwrap().0, "13");
        assert_eq!(context.evaluate("{param.dice}").unwrap().0, "1d6");
    }

    #[test]
    fn validates_templates_without_a_context() {
        for template in [
            "8+{attack_mod}+{prof}",
            "1d10+{level}",
            "2d8+{spell_mod}",
            "{spell_dc}",
            "{param.dice}",
            "1d4+1",
            "{attack_mod}",
            "-10",
        ] {
            validate_template(template).unwrap_or_else(|error| {
                panic!("`{template}` should validate: {error}");
            });
        }
    }
}
