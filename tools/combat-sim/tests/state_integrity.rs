//! Live encounter state files (`combat/*.state.json`) and the integrity
//! manifest (TASK-0066).
//!
//! The tool's own writes (creating or updating a state file) must never show
//! up as hand edits, a missing state file must not be reported, manual edits
//! must be reported, and a failed write must not advance the baseline.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use combat_sim::integrity::{self, Change};
use combat_sim::state::EncounterState;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A throwaway save directory with a `combat/` subdirectory.
struct Save(PathBuf);

impl Save {
    fn new(tag: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!(
            "combat-sim-state-integrity-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("combat")).unwrap();
        Self(d)
    }

    fn dir(&self) -> &Path {
        &self.0
    }

    /// Seed save files and create the integrity manifest.
    fn with_manifest(&self) {
        fs::write(self.dir().join("progress.yaml"), "xp: {a: 0}\n").unwrap();
        let (_, created) = integrity::ensure(self.dir()).unwrap();
        assert!(created, "manifest should be new");
        assert!(integrity::changes(self.dir()).unwrap().is_empty());
    }
}

impl Drop for Save {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A minimal encounter state. Every field of `EncounterState` is
/// `#[serde(default)]`, so `{}` deserializes into an empty encounter.
fn empty_state() -> EncounterState {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "combat-sim-state-integrity-seed-{}-{n}.json",
        std::process::id()
    ));
    fs::write(&p, "{}").unwrap();
    let state = EncounterState::load(&p).unwrap();
    let _ = fs::remove_file(&p);
    state
}

#[test]
fn tool_writes_of_state_files_are_not_reported() {
    let save = Save::new("tool-writes");
    save.with_manifest();
    let path = save.dir().join("combat/encounter.state.json");

    // Creating a new state file: adopted by the manifest, no false positive.
    let mut state = empty_state();
    state.save(&path).unwrap();
    assert!(integrity::changes(save.dir()).unwrap().is_empty());
    let manifest = integrity::load(save.dir()).unwrap().unwrap();
    assert!(manifest.files.contains_key("combat/encounter.state.json"));

    // Updating the existing state file: baseline moves with the write.
    state.round = 2;
    state.log.push("round 2 begins".into());
    state.save(&path).unwrap();
    assert!(
        integrity::changes(save.dir()).unwrap().is_empty(),
        "{:?}",
        integrity::changes(save.dir()).unwrap()
    );
}

#[test]
fn hand_edits_to_state_files_are_reported() {
    let save = Save::new("hand-edit");
    save.with_manifest();
    let path = save.dir().join("combat/encounter.state.json");
    let mut state = empty_state();
    state.save(&path).unwrap();
    assert!(integrity::changes(save.dir()).unwrap().is_empty());

    // Manual edit outside the tools.
    fs::write(&path, "{\n  \"round\": 99\n}\n").unwrap();
    assert_eq!(
        integrity::changes(save.dir()).unwrap(),
        vec![Change::Modified("combat/encounter.state.json".into())]
    );

    // A later tool write does not launder the hand edit. The tool writes new
    // content here (round 3): the manifest keeps the old baseline, so the
    // file stays flagged. (Had the tool rewritten the exact baseline bytes,
    // the hand edit would simply be gone from disk with nothing to report.)
    state.round = 3;
    state.save(&path).unwrap();
    assert_eq!(
        integrity::changes(save.dir()).unwrap(),
        vec![Change::Modified("combat/encounter.state.json".into())]
    );
}

#[test]
fn missing_state_file_is_no_error() {
    let save = Save::new("missing");
    save.with_manifest();

    // No state file at all: nothing tracked, nothing reported.
    let snapshot = integrity::snapshot(save.dir());
    assert!(!snapshot.keys().any(|k| k.ends_with(".state.json")));
    assert!(integrity::changes(save.dir()).unwrap().is_empty());

    // A tool write elsewhere still works without inventing state entries.
    let before = integrity::snapshot(save.dir());
    fs::write(save.dir().join("progress.yaml"), "xp: {a: 10}\n").unwrap();
    integrity::after_write(save.dir(), &before).unwrap();
    let changes = integrity::changes(save.dir()).unwrap();
    assert!(changes.is_empty(), "{changes:?}");
}

#[test]
fn failed_write_does_not_advance_baseline() {
    let save = Save::new("failed-write");
    save.with_manifest();

    // A directory where the state file should be makes the rename fail.
    let path = save.dir().join("combat/blocked.state.json");
    fs::create_dir_all(&path).unwrap();
    let state = empty_state();
    assert!(state.save(&path).is_err(), "write onto a dir must fail");

    // The manifest gained no baseline entry for the state file and no
    // temporary file was left behind.
    let manifest = integrity::load(save.dir()).unwrap().unwrap();
    assert!(!manifest.files.contains_key("combat/blocked.state.json"));
    let leftovers: Vec<String> = fs::read_dir(save.dir().join("combat"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert!(integrity::changes(save.dir()).unwrap().is_empty());

    // Once the obstacle is gone, the same write succeeds and is adopted.
    fs::remove_dir_all(&path).unwrap();
    state.save(&path).unwrap();
    assert!(integrity::changes(save.dir()).unwrap().is_empty());
}

#[test]
fn state_outside_a_save_is_not_tracked() {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "combat-sim-state-integrity-scratch-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // Scratch state outside any save: written fine, no manifest created.
    let state = empty_state();
    state.save(dir.join("scratch.state.json")).unwrap();
    assert!(!dir.join("integrity.yaml").exists());

    let _ = fs::remove_dir_all(&dir);
}
