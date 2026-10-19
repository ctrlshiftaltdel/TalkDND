//! Temporary profiling counters (removed before final commit).
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub static TURN: AtomicU64 = AtomicU64::new(0);
pub static TARGET: AtomicU64 = AtomicU64::new(0);
pub static MOVE: AtomicU64 = AtomicU64::new(0);
pub static ACTION: AtomicU64 = AtomicU64::new(0);
pub static ATTACK: AtomicU64 = AtomicU64::new(0);
pub static ABILITY: AtomicU64 = AtomicU64::new(0);
pub static NEXT_TURN: AtomicU64 = AtomicU64::new(0);
pub static INIT: AtomicU64 = AtomicU64::new(0);
pub static PLAY_OTHER: AtomicU64 = AtomicU64::new(0);
pub static SCORE: AtomicU64 = AtomicU64::new(0);
pub static EXECUTE: AtomicU64 = AtomicU64::new(0);
pub static BEST_ACTION: AtomicU64 = AtomicU64::new(0);
pub static NEAREST: AtomicU64 = AtomicU64::new(0);
pub static ADVANCE: AtomicU64 = AtomicU64::new(0);
pub static MOVE_TO: AtomicU64 = AtomicU64::new(0);
pub static RESOLVE: AtomicU64 = AtomicU64::new(0);

pub struct Prof {
    start: Instant,
    slot: &'static AtomicU64,
}

impl Prof {
    pub fn scope(slot: &'static AtomicU64) -> Self {
        Self { start: Instant::now(), slot }
    }
}

impl Drop for Prof {
    fn drop(&mut self) {
        self.slot
            .fetch_add(self.start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }
}

pub fn reset() {
    for slot in [&TURN, &TARGET, &MOVE, &ACTION, &ATTACK, &ABILITY, &NEXT_TURN, &INIT, &PLAY_OTHER, &SCORE, &EXECUTE, &BEST_ACTION, &NEAREST, &ADVANCE, &MOVE_TO, &RESOLVE] {
        slot.store(0, Ordering::Relaxed);
    }
}

pub fn dump() {
    let vals = [("turn", &TURN), ("target", &TARGET), ("move", &MOVE), ("action", &ACTION), ("attack", &ATTACK), ("ability", &ABILITY), ("next_turn", &NEXT_TURN), ("init", &INIT), ("play_other", &PLAY_OTHER), ("score", &SCORE), ("execute", &EXECUTE), ("best_act", &BEST_ACTION), ("nearest", &NEAREST), ("advance", &ADVANCE), ("move_to", &MOVE_TO), ("resolve", &RESOLVE)];
    for (name, slot) in vals {
        println!("{name:>10}: {:>10.1} ms", slot.load(Ordering::Relaxed) as f64 / 1e6);
    }
}
