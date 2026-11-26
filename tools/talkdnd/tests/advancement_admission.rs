use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use talkdnd::advancement_admission::{inspect, Admission, ApprovedRoots, Reason};

struct Fixture {
    root: PathBuf,
    save: PathBuf,
    backup: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "talkdnd-admission-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let save = root.join("save");
        let backup = root.join("backups");
        fs::create_dir(&save).unwrap();
        fs::create_dir(&backup).unwrap();
        fs::write(save.join("character.md"), b"synthetic save").unwrap();
        Self { root, save, backup }
    }

    fn check(&self, backup: &Path, synchronized: &[&Path]) -> Admission {
        inspect(
            &self.save,
            backup,
            &ApprovedRoots {
                saves: &self.root,
                backups: &self.backup,
                synchronized,
            },
        )
    }

    fn unchanged(&self, known_entries: &[&str]) {
        let mut names: Vec<_> = fs::read_dir(&self.save)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, known_entries);
        assert_eq!(
            fs::read(self.save.join("character.md")).unwrap(),
            b"synthetic save"
        );
        assert_eq!(fs::read_dir(&self.backup).unwrap().count(), 0);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // This absolute fixture directory was created exclusively by Fixture::new.
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn actual_linux_mount_is_observed_without_mutating_the_fixture() {
    use std::io::Write;

    let fixture = Fixture::new();
    let decision = fixture.check(&fixture.backup, &[]);
    if let Ok(mount) = std::process::Command::new("findmnt")
        .args(["-n", "-o", "FSTYPE,VFS-OPTIONS", "--target"])
        .arg(&fixture.root)
        .output()
    {
        assert!(mount.status.success());
        let observed = String::from_utf8(mount.stdout).unwrap();
        let mut columns = observed.split_ascii_whitespace();
        let fs_type = columns.next().unwrap();
        let vfs = columns.next().unwrap();
        let writable = vfs.split(',').any(|flag| flag == "rw");
        // Write directly to stderr so the regular CI test command records the observation.
        writeln!(
            std::io::stderr().lock(),
            "advancement fixture mount: filesystem={fs_type}, writable={writable}"
        )
        .unwrap();
        if fs_type == "ext4" && writable {
            assert_eq!(decision, Admission::Admitted);
        } else if fs_type != "ext4" {
            assert_eq!(decision, Admission::Unsupported(Reason::Filesystem));
        } else {
            assert_eq!(decision, Admission::Unsupported(Reason::ReadOnlyMount));
        }
    } else {
        assert!(matches!(
            decision,
            Admission::Admitted | Admission::Unsupported(Reason::Filesystem)
        ));
    }
    let first_backup = fixture.root.join("first-backup");
    assert_eq!(
        inspect(
            &fixture.save,
            &fixture.root,
            &ApprovedRoots {
                saves: &fixture.root,
                backups: &fixture.root,
                synchronized: &[],
            },
        ),
        decision
    );
    assert!(!first_backup.exists());
    fixture.unchanged(&["character.md"]);
}

#[cfg(target_os = "linux")]
#[test]
fn rejects_outside_roots_known_sync_and_unresolved_paths_without_writes() {
    let fixture = Fixture::new();
    let outside = fixture.root.join("other");
    fs::create_dir(&outside).unwrap();
    assert_eq!(
        fixture.check(&outside, &[]),
        Admission::Unsupported(Reason::OutsideApprovedRoots)
    );
    assert_eq!(
        fixture.check(&fixture.backup, &[&fixture.save]),
        Admission::Unsupported(Reason::Synchronized)
    );
    assert_eq!(
        fixture.check(&fixture.backup.join("missing"), &[]),
        Admission::Unknown(Reason::Path)
    );
    fixture.unchanged(&["character.md"]);
}

#[cfg(target_os = "linux")]
#[test]
fn rejects_backup_alias_and_save_links_without_touching_user_bytes() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let alias = fixture.root.join("alias");
    symlink(&fixture.backup, &alias).unwrap();
    assert_eq!(
        fixture.check(&alias, &[]),
        Admission::Unsupported(Reason::LinkOrSpecialFile)
    );
    let link = fixture.save.join("link");
    symlink(fixture.save.join("character.md"), &link).unwrap();
    // The platform checks may reject the actual mount before reading any save entries.
    if fixture.check(&fixture.backup, &[]) != Admission::Unsupported(Reason::Filesystem) {
        assert_eq!(
            fixture.check(&fixture.backup, &[]),
            Admission::Unsupported(Reason::LinkOrSpecialFile)
        );
    }
    fixture.unchanged(&["character.md", "link"]);
}

#[cfg(not(target_os = "linux"))]
#[test]
fn non_linux_stops_before_any_fixture_write() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.check(&fixture.backup, &[]),
        Admission::Unsupported(Reason::Platform)
    );
    fixture.unchanged(&["character.md"]);
}
