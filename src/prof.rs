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

pub struct Prof(Instant);
impl Prof {
    pub fn new() -> Self { Self(Instant::now()) }
    pub fn done(self, slot: &AtomicU64) {
        slot.fetch_add(self.0.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }
}
impl Drop for Prof {
    fn drop(&mut self) {}
}

pub fn dump() {
    let vals = [("turn", &TURN), ("target", &TARGET), ("move", &MOVE), ("action", &ACTION), ("attack", &ATTACK), ("ability", &ABILITY), ("next_turn", &NEXT_TURN), ("init", &INIT), ("play_other", &PLAY_OTHER)];
    for (name, slot) in vals { println!("{name:>10}: {:>10.1} ms", slot.load(Ordering::Relaxed) as f64 / 1e6); }
}
