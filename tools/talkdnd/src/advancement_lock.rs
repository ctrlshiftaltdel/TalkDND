//! Re-export of the shared SAVE lock primitive (`tools/advancement-lock`).
//!
//! `ADV-P1-1b` moved the v1 implementation out of this crate so `talkdnd` and
//! `combat-sim` share one lock. Group A writers keep using this module path
//! (`open` + `check_ordinary_write`); the behaviour is unchanged. A D commit
//! must still use [`AdvancementLock::check_writable`]. This crate does not
//! implement the D stage machine or auto-init.

pub use advancement_lock::save_lock::Result;
pub use advancement_lock::{
    init, is_save_lock_held, lock_path, open, reseal, state_path, AcquireOptions, AdvancementLock,
    CompletionEvidence, Error, ErrorKind, InitHooks, InitOptions, InitOutcome, LockHeader,
    LockIdentity, OpenOptions, Pending, Published, Stage, State, StateRecord, COPY_LIMITATION,
    DEFAULT_ACQUIRE_TIMEOUT, LOCK_FILE, SCHEMA, STATE_FILE,
};
