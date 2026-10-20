use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SEQ: AtomicU64 = AtomicU64::new(0);

pub struct TempRepo {
    pub path: PathBuf,
}

impl TempRepo {
    pub fn new() -> Self {
        let repo = Self::dir_only();
        git(&repo.path, &["init", "-q"]);
        repo
    }

    pub fn dir_only() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "talkdnd-dev-{tick}-{n}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub fn write(&self, rel: &str, body: &str) {
        self.write_bytes(rel, body.as_bytes());
    }

    pub fn write_bytes(&self, rel: &str, body: &[u8]) {
        let path = self.path.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    pub fn symlink(&self, rel: &str, target: &str) {
        let path = self.path.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        std::os::unix::fs::symlink(target, path).unwrap();
    }

    #[allow(dead_code)] // leak tests stage paths; migrate tests do not
    pub fn add(&self, rels: &[&str]) {
        let mut args = vec!["add", "-f", "--"];
        args.extend_from_slice(rels);
        git(&self.path, &args);
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn bin() -> PathBuf {
    // The test harness lives in target/<profile>/deps/. The dev binary is the
    // sibling of deps/. This cargo does not export CARGO_BIN_EXE_* to tests.
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    path.pop();
    path.push("talkdnd-dev");
    path
}

pub fn run(dir: &Path, args: &[&str]) -> Output {
    let program = bin();
    let output = Command::new(&program)
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap_or_else(|err| panic!("failed to run {}: {err}", program.display()));
    output
}

pub fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}
