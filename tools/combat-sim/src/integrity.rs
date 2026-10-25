//! Save integrity manifest (`Saves/<slot>/integrity.yaml`), shared by
//! talkdnd and combat-sim.
//!
//! After every tool write the fingerprints of the key save files are
//! recorded. A file whose content no longer matches its fingerprint was
//! changed outside the tools. A file that was already changed by hand stays
//! flagged even if a tool writes it later, until the DM accepts it with
//! `talkdnd integrity accept --reason`.
//!
//! Every manifest read-modify-write runs under a per-save advisory lock
//! (`.talkdnd.integrity.lock`, [`lock`]), so concurrent tool processes cannot
//! overwrite each other's fingerprints with a manifest that was read before
//! the other process committed. The lock file is never fingerprinted and
//! never removed: the OS releases the lock when the process exits, so a
//! crash leaves no stale lock. While holding the lock, use the `*_locked`
//! variants — nesting the locking wrappers on the same save deadlocks.
//!
//! The fingerprint is FNV-1a 64 over the bytes. It catches casual hand edits;
//! it is not a security boundary (anyone with file access can also edit the
//! manifest).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

pub const FILE: &str = "integrity.yaml";

/// Dedicated advisory lock file for the integrity manifest
/// (`Saves/<slot>/.talkdnd.integrity.lock`). It serializes every manifest
/// read-modify-write within one save. It is distinct from checkpoint's
/// `.talkdnd.lock` and never fingerprinted: [`snapshot`] tracks save content
/// files only.
pub const LOCK: &str = ".talkdnd.integrity.lock";

/// A held integrity manifest lock for one save ([`lock`]). Released when
/// dropped — including through an error return or a panic — or when the
/// process exits, so a crashed process never leaves a stale lock.
pub struct ManifestLock {
    _file: File,
}

/// Acquire the integrity manifest lock for `save`, blocking until any other
/// holder releases it. Never nest acquisitions for the same save: a second
/// `lock()` in the same process would wait on itself. While holding the
/// lock, call the `*_locked` variants instead of the locking wrappers.
pub fn lock(save: &Path) -> Result<ManifestLock> {
    fs::create_dir_all(save).with_context(|| format!("create {}", save.display()))?;
    let p = save.join(LOCK);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&p)
        .with_context(|| format!("open integrity lock {}", p.display()))?;
    file.lock()
        .with_context(|| format!("lock integrity manifest of {}", save.display()))?;
    Ok(ManifestLock { _file: file })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Accepted {
    pub date: String,
    pub reason: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    #[serde(default)]
    pub version: u32,
    /// Relative path -> fingerprint.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    /// Reconciliation baseline maintained by talkdnd (values + log position).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<serde_norway::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted: Vec<Accepted>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Modified(String),
    Added(String),
    Removed(String),
}

impl Change {
    pub fn path(&self) -> &str {
        match self {
            Change::Modified(p) | Change::Added(p) | Change::Removed(p) => p,
        }
    }
    pub fn describe(&self) -> String {
        match self {
            Change::Modified(p) => format!("{p}: changed outside the tools"),
            Change::Added(p) => format!("{p}: new file not written by the tools"),
            Change::Removed(p) => format!("{p}: removed outside the tools"),
        }
    }
}

pub fn fingerprint(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{h:016x}:{}", bytes.len())
}

fn list(dir: &Path, ext: &str, skip_underscore: bool) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == ext))
        .filter(|p| {
            !skip_underscore
                || !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('_'))
        })
        .collect();
    v.sort();
    v
}

/// `manifest.yaml` of every checkpoint directory under `checkpoints/`.
/// Only the manifest is hashed; the checkpoint's data copies are not.
fn checkpoint_manifests(save: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(save.join("checkpoints")) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .map(|d| d.join("manifest.yaml"))
        .filter(|p| p.is_file())
        .collect();
    v.sort();
    v
}

/// Current fingerprints of the tracked files.
pub fn snapshot(save: &Path) -> BTreeMap<String, String> {
    let mut files: Vec<PathBuf> = Vec::new();
    for f in [
        "progress.yaml",
        "reveals.yaml",
        "house-rules.md",
        "save-meta.yaml",
        "original.yaml",
        // Continuity sidecars (P03/P04/P06): tracked only when present.
        "memory.yaml",
        "entities.yaml",
        "relations.yaml",
        "world-time.yaml",
    ] {
        let p = save.join(f);
        if p.is_file() {
            files.push(p);
        }
    }
    files.extend(checkpoint_manifests(save));
    files.extend(list(&save.join("Characters"), "md", true));
    files.extend(list(&save.join("NPCs"), "md", true));
    files.extend(list(&save.join("sheets"), "yaml", false));
    files.extend(list(&save.join("combat"), "yaml", false));
    // Live encounter states (combat/*.state.json): the authoritative record
    // of an ongoing fight, tracked when present so hand edits are reported.
    // A save without state files simply contributes nothing here.
    files.extend(
        list(&save.join("combat"), "json", false)
            .into_iter()
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with(crate::state::STATE_SUFFIX))
            }),
    );
    let mut out = BTreeMap::new();
    for f in files {
        if let Ok(bytes) = fs::read(&f) {
            let rel = f
                .strip_prefix(save)
                .unwrap_or(&f)
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, fingerprint(&bytes));
        }
    }
    out
}

pub fn path(save: &Path) -> PathBuf {
    save.join(FILE)
}

pub fn load(save: &Path) -> Result<Option<Manifest>> {
    let p = path(save);
    if !p.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    let manifest: Manifest =
        serde_norway::from_str(&text).with_context(|| format!("invalid {}", p.display()))?;
    if manifest.version > 1 {
        bail!(
            "{}: unsupported integrity manifest version {} (this build supports 1); update talkdnd",
            p.display(),
            manifest.version
        );
    }
    Ok(Some(manifest))
}

pub fn store(save: &Path, m: &Manifest) -> Result<()> {
    let text = format!(
        "# TalkDND save integrity manifest (written by talkdnd/combat-sim; do not edit).\n# Accept hand edits with `talkdnd integrity accept --reason \"...\"`.\n{}",
        serde_norway::to_string(m)?
    );
    crate::fsutil::atomic_write(path(save), text.as_bytes())
        .with_context(|| format!("write {}", path(save).display()))
}

/// Fresh manifest from the current state.
pub fn baseline(save: &Path) -> Manifest {
    Manifest {
        version: 1,
        files: snapshot(save),
        baseline: None,
        accepted: Vec::new(),
    }
}

/// Load the manifest, creating a baseline for saves that predate it.
/// Returns (manifest, created). Acquires the manifest lock.
pub fn ensure(save: &Path) -> Result<(Manifest, bool)> {
    let _l = lock(save)?;
    ensure_locked(save)
}

/// [`ensure`] while already holding [`lock`].
pub fn ensure_locked(save: &Path) -> Result<(Manifest, bool)> {
    if let Some(m) = load(save)? {
        return Ok((m, false));
    }
    let m = baseline(save);
    store(save, &m)?;
    Ok((m, true))
}

pub fn changes_against(m: &Manifest, now: &BTreeMap<String, String>) -> Vec<Change> {
    let mut v = Vec::new();
    for (p, h) in now {
        match m.files.get(p) {
            Some(old) if old == h => {}
            Some(_) => v.push(Change::Modified(p.clone())),
            None => v.push(Change::Added(p.clone())),
        }
    }
    for p in m.files.keys() {
        if !now.contains_key(p) {
            v.push(Change::Removed(p.clone()));
        }
    }
    v
}

pub fn changes(save: &Path) -> Result<Vec<Change>> {
    let _l = lock(save)?;
    let (m, _) = ensure_locked(save)?;
    Ok(changes_against(&m, &snapshot(save)))
}

/// Record the files a tool changed between `before` and now. Files that
/// were already out of step with the manifest before the write stay flagged.
/// Acquires the manifest lock; the manifest is read inside the lock.
pub fn after_write(save: &Path, before: &BTreeMap<String, String>) -> Result<()> {
    let _l = lock(save)?;
    let now = snapshot(save);
    after_write_with_locked(save, before, &now)
}

/// [`after_write`] with the current snapshot already taken, so callers that
/// need it anyway (audit reconciliation) parse the save only once.
/// Acquires the manifest lock; the manifest is read inside the lock.
pub fn after_write_with(
    save: &Path,
    before: &BTreeMap<String, String>,
    now: &BTreeMap<String, String>,
) -> Result<()> {
    let _l = lock(save)?;
    after_write_with_locked(save, before, now)
}

/// [`after_write_with`] while already holding [`lock`].
pub fn after_write_with_locked(
    save: &Path,
    before: &BTreeMap<String, String>,
    now: &BTreeMap<String, String>,
) -> Result<()> {
    after_write_guarded(save, before, now, None)
}

/// The manifest read-modify-write itself; the caller holds [`lock`].
/// `fault` is a test-only hook (see the `interleaved_manifest_writers`
/// regression test), always `None` in production. It runs just before the
/// manifest is read, so a competing writer's committed update is picked up
/// by the in-lock load instead of being overwritten by a stale copy.
fn after_write_guarded(
    save: &Path,
    before: &BTreeMap<String, String>,
    now: &BTreeMap<String, String>,
    fault: Option<&dyn Fn()>,
) -> Result<()> {
    if let Some(f) = fault {
        f();
    }
    let Some(mut m) = load(save)? else {
        // First tool write on an old save: the current state is the baseline.
        let m = Manifest {
            version: 1,
            files: now.clone(),
            baseline: None,
            accepted: Vec::new(),
        };
        store(save, &m)?;
        return Ok(());
    };
    let clean = |p: &String| match m.files.get(p) {
        Some(old) => before.get(p) == Some(old),
        // A file tracked by this newer tool version but absent from an older
        // manifest (save-meta.yaml, original.yaml) is adopted by the first
        // tool write. If the DM had edited it in the meantime, the tool just
        // overwrote that value, so there is nothing left to compare.
        None => true,
    };
    let mut updates = Vec::new();
    for (p, h) in now {
        if before.get(p) != Some(h) && clean(p) {
            updates.push((p.clone(), Some(h.clone())));
        }
    }
    for p in before.keys() {
        if !now.contains_key(p) && clean(p) {
            updates.push((p.clone(), None));
        }
    }
    if updates.is_empty() {
        return Ok(());
    }
    for (p, h) in updates {
        match h {
            Some(h) => m.files.insert(p, h),
            None => m.files.remove(&p),
        };
    }
    store(save, &m)
}

/// The save directory for a file inside `Saves/<slot>/combat/`, when that
/// save already has a manifest.
pub fn save_of_combat_file(file: &Path) -> Option<PathBuf> {
    let save = file.parent()?.parent()?;
    path(save).is_file().then(|| save.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("integrity-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("Characters")).unwrap();
        fs::create_dir_all(d.join("combat")).unwrap();
        d
    }

    #[test]
    fn tool_writes_are_recorded_hand_edits_stay_flagged() {
        let d = dir("flow");
        fs::write(d.join("progress.yaml"), "xp: {a: 0}\n").unwrap();
        fs::write(d.join("Characters/A.md"), "hp: 10\n").unwrap();
        fs::write(d.join("Characters/_notes.md"), "x").unwrap();
        let (_, created) = ensure(&d).unwrap();
        assert!(created);
        assert!(changes(&d).unwrap().is_empty());
        // tool write
        let before = snapshot(&d);
        fs::write(d.join("progress.yaml"), "xp: {a: 100}\n").unwrap();
        after_write(&d, &before).unwrap();
        assert!(changes(&d).unwrap().is_empty());
        // hand edit
        fs::write(d.join("Characters/A.md"), "hp: 99\n").unwrap();
        fs::write(d.join("Characters/_notes.md"), "y").unwrap();
        assert_eq!(
            changes(&d).unwrap(),
            vec![Change::Modified("Characters/A.md".into())]
        );
        // a later tool write to that file does not launder the edit
        let before = snapshot(&d);
        fs::write(d.join("Characters/A.md"), "hp: 98\n").unwrap();
        fs::write(d.join("combat/party-state.yaml"), "members: {}\n").unwrap();
        after_write(&d, &before).unwrap();
        assert_eq!(
            changes(&d).unwrap(),
            vec![Change::Modified("Characters/A.md".into())]
        );
        let _ = fs::remove_dir_all(&d);
    }

    /// Regression test for the lost-fingerprint race (issue #149): two
    /// writers whose manifest read-modify-writes interleave must both end up
    /// in the manifest. Writer A runs with writer B's complete
    /// read-modify-write injected at the test-only hook, i.e. before A reads
    /// the manifest inside its lock. The fixed code re-reads the manifest
    /// inside the lock after the injection, so B's committed fingerprint
    /// survives A's store. Before the manifest lock existed, A merged into
    /// the manifest it had loaded before B stored and dropped B's update —
    /// this test failed with `Modified("Characters/A.md")` on that code.
    #[test]
    fn interleaved_manifest_writers_do_not_lose_fingerprints() {
        let d = dir("race");
        fs::write(d.join("progress.yaml"), "xp: {a: 0}\n").unwrap();
        fs::write(d.join("Characters/A.md"), "hp: 10\n").unwrap();
        ensure(&d).unwrap();
        assert!(changes(&d).unwrap().is_empty());

        // Both writers started from the same pre-write snapshot; A rewrote
        // progress.yaml, B rewrote Characters/A.md.
        let shared_before = snapshot(&d);
        fs::write(d.join("progress.yaml"), "xp: {a: 100}\n").unwrap();
        let now_a = snapshot(&d);
        fs::write(d.join("Characters/A.md"), "hp: 20\n").unwrap();
        let now_b = snapshot(&d);

        let save = d.clone();
        let before_b = shared_before.clone();
        let writer_b = move || {
            after_write_guarded(&save, &before_b, &now_b, None).unwrap();
        };
        after_write_guarded(&d, &shared_before, &now_a, Some(&writer_b)).unwrap();

        assert!(
            !snapshot(&d).contains_key(LOCK),
            "the lock file is fingerprinted"
        );
        assert!(
            changes(&d).unwrap().is_empty(),
            "{:?}",
            changes(&d).unwrap()
        );
        let m = load(&d).unwrap().unwrap();
        assert_eq!(
            m.files.get("progress.yaml").map(String::as_str),
            Some(fingerprint(b"xp: {a: 100}\n").as_str())
        );
        assert_eq!(
            m.files.get("Characters/A.md").map(String::as_str),
            Some(fingerprint(b"hp: 20\n").as_str())
        );
        let _ = fs::remove_dir_all(&d);
    }

    /// Two real writers through the public API, serialized only by the
    /// manifest lock: whichever commits second re-reads the manifest inside
    /// the lock and keeps the first writer's fingerprint.
    ///
    /// Both writers share one pre-write baseline and publish their file with
    /// [`crate::fsutil::atomic_write`], exactly like the production writers,
    /// and neither records until both files are in place. The outcome
    /// therefore does not depend on thread scheduling: whenever an
    /// `after_write` can run, the save already holds both final files, so the
    /// first committer records both fingerprints and the second finds nothing
    /// left to update. The previous version took a per-thread `snapshot` and
    /// wrote with a bare `fs::write`; one writer could then observe the
    /// other's truncated file and commit that partial fingerprint, after
    /// which the real writer looked dirty (macOS CI reported
    /// `Modified("Characters/A.md")`).
    #[test]
    fn concurrent_public_api_writers_keep_both_fingerprints() {
        use std::sync::{Arc, Barrier};

        let d = dir("threads");
        fs::write(d.join("progress.yaml"), "xp: {a: 0}\n").unwrap();
        fs::write(d.join("Characters/A.md"), "hp: 10\n").unwrap();
        ensure(&d).unwrap();

        // One baseline for both writers, taken before either file changes.
        let before = snapshot(&d);
        // Both files land first; only then may either writer record.
        let written = Arc::new(Barrier::new(2));
        let a = {
            let (d, before, written) = (d.clone(), before.clone(), Arc::clone(&written));
            std::thread::spawn(move || {
                crate::fsutil::atomic_write(d.join("progress.yaml"), "xp: {a: 100}\n").unwrap();
                written.wait();
                after_write(&d, &before).unwrap();
            })
        };
        let b = {
            let (d, before, written) = (d.clone(), before, Arc::clone(&written));
            std::thread::spawn(move || {
                crate::fsutil::atomic_write(d.join("Characters/A.md"), "hp: 20\n").unwrap();
                written.wait();
                after_write(&d, &before).unwrap();
            })
        };
        a.join().unwrap();
        b.join().unwrap();
        assert!(
            changes(&d).unwrap().is_empty(),
            "{:?}",
            changes(&d).unwrap()
        );
        let m = load(&d).unwrap().unwrap();
        assert_eq!(
            m.files.get("progress.yaml").map(String::as_str),
            Some(fingerprint(b"xp: {a: 100}\n").as_str())
        );
        assert_eq!(
            m.files.get("Characters/A.md").map(String::as_str),
            Some(fingerprint(b"hp: 20\n").as_str())
        );
        let _ = fs::remove_dir_all(&d);
    }

    /// A failing manifest store must report the error and release the lock:
    /// the retry below would hang forever if the error path leaked it.
    #[test]
    fn a_failed_manifest_write_reports_the_error_and_releases_the_lock() {
        let d = dir("store-fail");
        fs::write(d.join("progress.yaml"), "xp: {a: 0}\n").unwrap();
        // A directory where the manifest belongs: every store fails on the
        // final rename, on every platform.
        fs::create_dir(path(&d)).unwrap();
        let before = snapshot(&d);
        fs::write(d.join("progress.yaml"), "xp: {a: 100}\n").unwrap();
        let err = after_write(&d, &before).unwrap_err();
        assert!(err.to_string().contains("integrity.yaml"), "{err}");
        // Nothing was persisted, and the lock is free again.
        assert!(load(&d).unwrap().is_none());
        fs::remove_dir(path(&d)).unwrap();
        after_write(&d, &before).unwrap();
        assert!(changes(&d).unwrap().is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn save_meta_and_original_ledger_are_tracked() {
        let d = dir("deviation");
        fs::write(
            d.join("save-meta.yaml"),
            "schema: 4\ndeviation: {scope: 0}\n",
        )
        .unwrap();
        fs::write(d.join("original.yaml"), "version: 1\nentries: []\n").unwrap();
        ensure(&d).unwrap();
        assert!(changes(&d).unwrap().is_empty());
        fs::write(
            d.join("save-meta.yaml"),
            "schema: 4\ndeviation: {scope: 2, ratio: 25}\n",
        )
        .unwrap();
        assert_eq!(
            changes(&d).unwrap(),
            vec![Change::Modified("save-meta.yaml".into())]
        );
        let before = snapshot(&d);
        fs::write(d.join("original.yaml"), "version: 1\nentries: [x]\n").unwrap();
        // The tool write records the clean ledger; the already-edited
        // save-meta stays flagged.
        after_write(&d, &before).unwrap();
        assert_eq!(
            changes(&d).unwrap(),
            vec![Change::Modified("save-meta.yaml".into())]
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_new_to_the_manifest_is_adopted_on_a_tool_write() {
        let d = dir("adopt");
        fs::write(d.join("progress.yaml"), "xp: {a: 0}\n").unwrap();
        ensure(&d).unwrap();
        // Simulate an older manifest that predates save-meta tracking.
        fs::write(
            d.join("save-meta.yaml"),
            "schema: 4\ndeviation: {scope: 0}\n",
        )
        .unwrap();
        assert!(changes(&d)
            .unwrap()
            .iter()
            .any(|c| c.path() == "save-meta.yaml"));
        let before = snapshot(&d);
        fs::write(
            d.join("save-meta.yaml"),
            "schema: 4\ndeviation: {scope: 0}\n# migrated\n",
        )
        .unwrap();
        after_write(&d, &before).unwrap();
        assert!(
            changes(&d).unwrap().is_empty(),
            "{:?}",
            changes(&d).unwrap()
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn fingerprint_is_stable() {
        assert_eq!(fingerprint(b""), "fnv1a64:cbf29ce484222325:0");
        assert_ne!(fingerprint(b"a"), fingerprint(b"b"));
    }

    #[test]
    fn a_newer_manifest_schema_is_refused() {
        let d = dir("schema");
        fs::write(path(&d), "version: 99\nfiles: {}\n").unwrap();
        let err = load(&d).unwrap_err().to_string();
        assert!(
            err.contains("integrity.yaml") && err.contains("99"),
            "{err}"
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn snapshot_ignores_internal_directories() {
        let d = dir("internal");
        fs::write(d.join("combat/a.yaml"), "x: 1\n").unwrap();
        fs::create_dir_all(d.join("_backups/combat")).unwrap();
        fs::write(d.join("_backups/combat/old.yaml"), "x: 0\n").unwrap();
        fs::create_dir_all(d.join(".cache")).unwrap();
        fs::write(d.join(".cache/skip.yaml"), "x: 0\n").unwrap();
        let files = snapshot(&d);
        assert!(files.contains_key("combat/a.yaml"));
        assert!(
            !files.keys().any(|k| k.starts_with("_backups")),
            "{files:?}"
        );
        assert!(!files.keys().any(|k| k.starts_with(".cache")), "{files:?}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn combat_state_files_are_tracked_when_present() {
        let d = dir("state-files");
        fs::write(d.join("progress.yaml"), "xp: {a: 0}\n").unwrap();
        // Absent: nothing is tracked and nothing is reported.
        ensure(&d).unwrap();
        assert!(changes(&d).unwrap().is_empty());
        assert!(
            !snapshot(&d).keys().any(|k| k.ends_with(".state.json")),
            "state file tracked while absent"
        );
        // Present: the state file is tracked; other json files in combat/ are
        // not. A new file shows up as Added, not Modified.
        fs::write(d.join("combat/encounter.state.json"), "{}\n").unwrap();
        fs::write(d.join("combat/scratch.json"), "{}\n").unwrap();
        assert_eq!(
            changes(&d).unwrap(),
            vec![Change::Added("combat/encounter.state.json".into())]
        );
        // A tool write adopts it, then it is clean.
        let before = snapshot(&d);
        fs::write(d.join("combat/encounter.state.json"), "{\"round\": 1}\n").unwrap();
        after_write(&d, &before).unwrap();
        assert!(changes(&d).unwrap().is_empty());
        // A later hand edit is reported.
        fs::write(d.join("combat/encounter.state.json"), "{\"round\": 2}\n").unwrap();
        assert_eq!(
            changes(&d).unwrap(),
            vec![Change::Modified("combat/encounter.state.json".into())]
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn continuity_sidecars_are_tracked_when_present() {
        let d = dir("sidecars");
        fs::write(d.join("progress.yaml"), "xp: {a: 0}\n").unwrap();
        // Absent: nothing enters the manifest and nothing is reported.
        let (_, created) = ensure(&d).unwrap();
        assert!(created);
        assert!(changes(&d).unwrap().is_empty());
        let snap = snapshot(&d);
        for name in [
            "memory.yaml",
            "entities.yaml",
            "relations.yaml",
            "world-time.yaml",
        ] {
            assert!(!snap.contains_key(name), "{name} tracked while absent");
        }
        assert!(
            !snap.keys().any(|k| k.starts_with("checkpoints/")),
            "{snap:?}"
        );
        // Present: a tool-style write adopts them, then they are tracked.
        let before = snapshot(&d);
        for name in [
            "memory.yaml",
            "entities.yaml",
            "relations.yaml",
            "world-time.yaml",
        ] {
            fs::write(d.join(name), format!("version: 1\nfile: {name}\n")).unwrap();
        }
        fs::create_dir_all(d.join("checkpoints/20260101-120000")).unwrap();
        fs::write(
            d.join("checkpoints/20260101-120000/manifest.yaml"),
            "version: 1\nid: 20260101-120000\n",
        )
        .unwrap();
        // Never hashed: checkpoint data copies and session logs.
        fs::write(
            d.join("checkpoints/20260101-120000/progress.yaml"),
            "xp: {a: 7}\n",
        )
        .unwrap();
        fs::create_dir_all(d.join("Sessions")).unwrap();
        fs::write(d.join("Sessions/2026-01-01.md"), "log\n").unwrap();
        after_write(&d, &before).unwrap();
        assert!(changes(&d).unwrap().is_empty());
        let snap = snapshot(&d);
        for name in [
            "memory.yaml",
            "entities.yaml",
            "relations.yaml",
            "world-time.yaml",
            "checkpoints/20260101-120000/manifest.yaml",
        ] {
            assert!(snap.contains_key(name), "{name} missing from snapshot");
        }
        assert!(
            !snap
                .keys()
                .any(|k| k.starts_with("checkpoints/") && !k.ends_with("manifest.yaml")),
            "checkpoint data copy hashed: {snap:?}"
        );
        assert!(
            !snap.keys().any(|k| k.starts_with("Sessions/")),
            "session log hashed: {snap:?}"
        );
        assert!(!snap.contains_key("integrity.yaml"), "manifest self-hashed");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn hand_edits_to_continuity_sidecars_are_reported() {
        let d = dir("sidecar-edit");
        fs::write(d.join("memory.yaml"), "version: 1\nentries: []\n").unwrap();
        fs::write(d.join("relations.yaml"), "version: 1\nrelations: []\n").unwrap();
        fs::create_dir_all(d.join("checkpoints/20260101-120000")).unwrap();
        fs::write(
            d.join("checkpoints/20260101-120000/manifest.yaml"),
            "version: 1\nid: 20260101-120000\n",
        )
        .unwrap();
        ensure(&d).unwrap();
        assert!(changes(&d).unwrap().is_empty());
        // Hand edits: one top-level sidecar and one checkpoint manifest.
        fs::write(d.join("memory.yaml"), "version: 1\nentries: [tampered]\n").unwrap();
        fs::write(
            d.join("checkpoints/20260101-120000/manifest.yaml"),
            "version: 1\nid: 20260101-120000\nreason: hand-edited\n",
        )
        .unwrap();
        let mut got: Vec<String> = changes(&d)
            .unwrap()
            .iter()
            .map(|c| c.path().to_string())
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec![
                "checkpoints/20260101-120000/manifest.yaml".to_string(),
                "memory.yaml".to_string(),
            ]
        );
        assert!(
            changes(&d)
                .unwrap()
                .iter()
                .all(|c| matches!(c, Change::Modified(_))),
            "{:?}",
            changes(&d).unwrap()
        );
        let _ = fs::remove_dir_all(&d);
    }
}
