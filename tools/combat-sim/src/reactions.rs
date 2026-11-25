//! Pending player choices for CLI encounter commands. The encounter stays at
//! its pre-command state until all choices are resolved.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::dice::DiceRng;

pub(crate) const PENDING_SCHEMA_VERSION: u32 = 2;
const WAITING_FOR_ANSWER: usize = usize::MAX;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PendingCommand {
    pub command: serde_json::Value,
    pub save: Option<PathBuf>,
    pub seed: Option<u64>,
    pub nonce: u64,
    pub choices: Vec<AnsweredReaction>,
    pub rolls: Vec<DieRoll>,
    pub window: ReactionWindow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReactionWindow {
    pub id: String,
    pub who: String,
    pub reaction: String,
    // Internal context identifies the exact event during replay. Do not put
    // it in player output, since it can name an unidentified creature.
    context: String,
    rolls_used: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReactionNotice {
    id: String,
    who: String,
    reaction: String,
}

impl ReactionWindow {
    pub fn notice(&self) -> ReactionNotice {
        ReactionNotice {
            id: self.id.clone(),
            who: self.who.clone(),
            reaction: self.reaction.clone(),
        }
    }

    pub fn prompt(&self) -> String {
        format!(
            "{} can use {}. Resolve with `reaction {} take` or `reaction {} pass`.",
            self.who, self.reaction, self.id, self.id
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnsweredReaction {
    pub window: ReactionWindow,
    pub take: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DieRoll {
    sides: u32,
    value: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct ReactionReplay {
    nonce: u64,
    answers: Vec<AnsweredReaction>,
    next_answer: usize,
    roll_count: Arc<AtomicUsize>,
    waiting: Option<ReactionWindow>,
    diverged: bool,
}

impl ReactionReplay {
    pub fn new(nonce: u64, answers: Vec<AnsweredReaction>) -> Self {
        Self {
            nonce,
            answers,
            next_answer: 0,
            roll_count: Arc::new(AtomicUsize::new(0)),
            waiting: None,
            diverged: false,
        }
    }

    pub fn dice(&self, inner: Box<dyn DiceRng>, prefix: Vec<DieRoll>, seeded: bool) -> ReplayDice {
        ReplayDice::new(inner, prefix, seeded, Arc::clone(&self.roll_count))
    }

    pub fn ask(&mut self, who: &str, reaction: &str, context: String) -> bool {
        if self.waiting.is_some() || self.diverged {
            return false;
        }
        let window = ReactionWindow {
            id: format!("{:016x}-{}", self.nonce, self.next_answer + 1),
            who: who.to_owned(),
            reaction: reaction.to_owned(),
            context,
            rolls_used: self.roll_count.load(Ordering::Relaxed),
        };
        match self.answers.get(self.next_answer) {
            Some(answer) if answer.window == window => {
                self.next_answer += 1;
                answer.take
            }
            Some(_) => {
                self.diverged = true;
                false
            }
            None => {
                self.waiting = Some(window);
                self.roll_count.store(WAITING_FOR_ANSWER, Ordering::Relaxed);
                false
            }
        }
    }

    pub fn finish(self, mut dice: ReplayDice) -> Result<Option<(ReactionWindow, Vec<DieRoll>)>> {
        if self.diverged || dice.diverged || self.next_answer != self.answers.len() {
            bail!("pending reaction no longer matches the recorded combat event; state unchanged");
        }
        if let Some(window) = self.waiting {
            // The engine can continue speculatively after asking. Preserve
            // only the rolls that occurred before the choice was offered.
            dice.rolls.truncate(window.rolls_used);
            return Ok(Some((window, dice.rolls)));
        }
        Ok(None)
    }
}

pub(crate) struct ReplayDice {
    inner: Box<dyn DiceRng>,
    rolls: Vec<DieRoll>,
    count: Arc<AtomicUsize>,
    diverged: bool,
}

impl ReplayDice {
    fn new(
        mut inner: Box<dyn DiceRng>,
        rolls: Vec<DieRoll>,
        seeded: bool,
        count: Arc<AtomicUsize>,
    ) -> Self {
        let diverged = rolls
            .iter()
            .any(|roll| roll.value == 0 || roll.value > roll.sides);
        if seeded && !diverged {
            for roll in &rolls {
                inner.roll_die(roll.sides);
            }
        }
        Self {
            inner,
            rolls,
            count,
            diverged,
        }
    }
}

impl DiceRng for ReplayDice {
    fn roll_die(&mut self, sides: u32) -> u32 {
        // Nothing after the first unanswered choice is committed. Do not
        // draw real dice while the rest of that trial is being discarded.
        if self.count.load(Ordering::Relaxed) == WAITING_FOR_ANSWER {
            return 1;
        }
        let index = self.count.fetch_add(1, Ordering::Relaxed);
        if self.diverged {
            return 1;
        }
        match self.rolls.get(index) {
            Some(roll) if roll.sides == sides => roll.value,
            Some(_) => {
                self.diverged = true;
                1
            }
            None => {
                let value = self.inner.roll_die(sides);
                self.rolls.push(DieRoll { sides, value });
                value
            }
        }
    }
}
