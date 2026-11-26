use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Barrier;

use super::github::{Creation, Transport};
use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "talkdnd-playtest-{}-{}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join(".talkdnd-root"), []).unwrap();
        Self(path)
    }

    fn queue(&self) -> Queue {
        Queue::open(&self.0).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn observation(stage: Stage, outcome: Outcome) -> Event {
    Event::new(stage, outcome, Some(Ruleset::FiveESrd), Duration::ZERO)
}

fn immediate() -> Event {
    observation(Stage::CheckpointRestore, Outcome::RollbackFailed)
}

fn eligible_queue(fixture: &Fixture) -> Queue {
    let queue = fixture.queue();
    queue.update(|state| state.record(immediate())).unwrap();
    queue
}

#[test]
fn technical_threshold_uses_two_recent_sessions_and_retains_local_totals() {
    let mut state = QueueState::default();
    let start = observation(Stage::Resume, Outcome::Completed);
    let fault = observation(Stage::CheckpointSnapshot, Outcome::LockUnavailable);
    state.record(start);
    state.record(fault);
    state.record(start);
    state.record(start);
    state.record(fault);
    state.record(fault);
    assert!(state.reports.is_empty(), "old session must not qualify");
    state.record(start);
    state.record(fault);
    assert_eq!(state.reports.len(), 1);
    assert_eq!(state.reports[0].event, fault);
    assert_eq!(
        state
            .totals
            .iter()
            .find(|count| count.event == fault)
            .unwrap()
            .count,
        4
    );
    state.record(fault);
    assert_eq!(state.reports.len(), 1);
}

#[test]
fn rollback_is_immediate_and_normal_results_are_only_counted() {
    let mut state = QueueState::default();
    for stage in [Stage::Reaction, Stage::NewGame, Stage::CheckpointRestore] {
        for outcome in [
            Outcome::Waiting,
            Outcome::Refused,
            Outcome::Unclassified,
            Outcome::Completed,
        ] {
            for _ in 0..4 {
                state.record(observation(stage, outcome));
            }
        }
    }
    assert!(!state.has_work());
    assert_eq!(state.totals.len(), 12);
    state.record(observation(Stage::Reaction, Outcome::RollbackFailed));
    assert!(!state.has_work());
    state.record(immediate());
    assert_eq!(state.reports.len(), 1);
    assert_eq!(state.reports[0].delivery, Delivery::Ready);
}

struct Fake {
    remote: Cell<Result<Option<u64>, ()>>,
    result: RefCell<Creation>,
    created: Cell<usize>,
    looked_up: Cell<usize>,
}

impl Fake {
    fn new(result: Creation) -> Self {
        Self {
            remote: Cell::new(Ok(None)),
            result: RefCell::new(result),
            created: Cell::new(0),
            looked_up: Cell::new(0),
        }
    }
}

impl Transport for Fake {
    fn find(&self, _: &str) -> Result<Option<u64>, ()> {
        self.looked_up.set(self.looked_up.get() + 1);
        self.remote.get()
    }

    fn create(&self, _: Event, _: &str) -> Creation {
        self.created.set(self.created.get() + 1);
        self.result.replace(Creation::Uncertain)
    }
}

#[test]
fn offline_and_definitely_unstarted_requests_retry() {
    let fixture = Fixture::new();
    let queue = eligible_queue(&fixture);
    let offline = Fake::new(Creation::Created(91));
    offline.remote.set(Err(()));
    deliver(&queue, &offline).unwrap();
    assert_eq!(offline.created.get(), 0);
    assert_eq!(
        fixture.queue().read().unwrap().reports[0].delivery,
        Delivery::Ready
    );
    let unavailable = Fake::new(Creation::NotStarted);
    deliver(&fixture.queue(), &unavailable).unwrap();
    assert_eq!(
        fixture.queue().read().unwrap().reports[0].delivery,
        Delivery::Ready
    );
    let online = Fake::new(Creation::Created(91));
    deliver(&fixture.queue(), &online).unwrap();
    assert_eq!(online.created.get(), 1);
    assert_eq!(
        fixture.queue().read().unwrap().reports[0].delivery,
        Delivery::Linked(91)
    );
}

#[test]
fn a_lost_response_reconciles_across_restarts_without_reposting() {
    let fixture = Fixture::new();
    let queue = eligible_queue(&fixture);
    let offline_after_post = Fake::new(Creation::Uncertain);
    deliver(&queue, &offline_after_post).unwrap();
    assert_eq!(offline_after_post.created.get(), 1);
    assert_eq!(
        fixture.queue().read().unwrap().reports[0].delivery,
        Delivery::Uncertain
    );
    let later = Fake::new(Creation::Created(99));
    deliver(&fixture.queue(), &later).unwrap();
    assert_eq!(later.looked_up.get(), 1);
    assert_eq!(
        later.created.get(),
        0,
        "absence after ambiguity is not permission to create"
    );
    later.remote.set(Ok(Some(86)));
    deliver(&fixture.queue(), &later).unwrap();
    assert_eq!(later.created.get(), 0);
    assert_eq!(
        fixture.queue().read().unwrap().reports[0].delivery,
        Delivery::Linked(86)
    );
    deliver(&fixture.queue(), &later).unwrap();
    assert_eq!(
        later.looked_up.get(),
        2,
        "linked incidents stay entirely local"
    );
}

#[test]
fn known_remote_fingerprint_skips_creation_even_without_local_link() {
    let fixture = Fixture::new();
    let queue = eligible_queue(&fixture);
    let remote = Fake::new(Creation::Created(88));
    remote.remote.set(Ok(Some(77)));
    deliver(&queue, &remote).unwrap();
    assert_eq!(remote.created.get(), 0);
    assert_eq!(
        queue.read().unwrap().reports[0].delivery,
        Delivery::Linked(77)
    );
}

#[cfg(unix)]
#[test]
fn worker_releases_lock_even_if_a_duplicate_descriptor_stays_open() {
    let fixture = Fixture::new();
    let queue = fixture.queue();
    let worker = queue.lock("worker.lock", 1).unwrap();
    // A forked child can retain the same open file description until exec.
    let inherited = worker.try_clone().unwrap();
    let contention = queue.lock("worker.lock", 1).unwrap_err();
    assert!(matches!(
        contention.downcast_ref::<fs::TryLockError>(),
        Some(fs::TryLockError::WouldBlock)
    ));
    drop(worker);

    let transport = Fake::new(Creation::Created(76));
    deliver(&queue, &transport).unwrap();
    drop(inherited);
}

#[test]
fn concurrent_worker_is_excluded_but_recording_continues_during_request() {
    struct WaitingTransport<'a> {
        entered: &'a Barrier,
        release: &'a Barrier,
        posts: &'a AtomicUsize,
    }

    impl Transport for WaitingTransport<'_> {
        fn find(&self, _: &str) -> Result<Option<u64>, ()> {
            self.entered.wait();
            self.release.wait();
            Ok(None)
        }

        fn create(&self, _: Event, _: &str) -> Creation {
            self.posts.fetch_add(1, Ordering::Relaxed);
            Creation::Created(76)
        }
    }

    let fixture = Fixture::new();
    let queue = eligible_queue(&fixture);
    let entered = Barrier::new(2);
    let release = Barrier::new(2);
    let posts = AtomicUsize::new(0);
    let transport = WaitingTransport {
        entered: &entered,
        release: &release,
        posts: &posts,
    };

    std::thread::scope(|scope| {
        let worker = scope.spawn(|| deliver(&queue, &transport));
        entered.wait();
        let other = Fake::new(Creation::Created(75));
        let duplicate = deliver(&fixture.queue(), &other);
        let record =
            queue.update(|state| state.record(observation(Stage::Reaction, Outcome::Waiting)));
        release.wait();
        worker.join().unwrap().unwrap();
        assert!(duplicate.is_err());
        assert_eq!(other.looked_up.get(), 0);
        record.unwrap();
    });
    assert_eq!(posts.load(Ordering::Relaxed), 1);
    assert_eq!(queue.read().unwrap().totals.len(), 2);
}

#[test]
fn report_uses_only_the_allowlist_even_if_operational_error_contains_secrets() {
    let synthetic =
        "fake-name /fake-slot fake-dm-secret fake-rule-text fake-log fake-token".to_string();
    let error =
        classified_error(Outcome::RollbackFailed, synthetic.clone()).context("fake-context");
    assert_eq!(error.root_cause().to_string(), synthetic);
    let outcome = classify(&Err(error), false);
    let event = observation(Stage::CheckpointRestore, outcome);
    let marker = event.marker().unwrap();
    let text = String::from_utf8(github::request(event, &marker).unwrap()).unwrap();
    for fragment in synthetic.split(' ').chain(["fake-context"]) {
        assert!(
            !text.contains(fragment),
            "unexpected input escaped the field boundary"
        );
    }
    let request: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(request.as_object().unwrap().len(), 2);
    assert_eq!(
        request["title"],
        "[playtest] checkpoint_restore: rollback_failed"
    );
    assert!(request["body"].as_str().unwrap().starts_with(&marker));
    assert!(text.contains("5e-srd"));
    assert!(github::request(event, "fake-injected-marker").is_none());
    assert_eq!(classify(&Ok(()), true), Outcome::Waiting);
    assert_eq!(
        classify(&Err(anyhow::anyhow!("looks like rollback failed")), false),
        Outcome::Unclassified
    );
}

#[test]
fn local_time_bucket_and_queue_size_stay_bounded() {
    let timed = Event::new(
        Stage::Resume,
        Outcome::Completed,
        None,
        Duration::from_secs(2),
    );
    assert_eq!(timed.elapsed, Elapsed::UnderTenSeconds);
    assert_eq!(
        Elapsed::from(Duration::from_secs(10)),
        Elapsed::TenSecondsOrMore
    );

    let fixture = Fixture::new();
    let queue = fixture.queue();
    let state = queue.0.join("state.json");
    let oversized = vec![b' '; MAX_STATE_BYTES as usize + 1];
    fs::write(&state, &oversized).unwrap();
    assert!(queue.update(|state| state.record(timed)).is_err());
    assert_eq!(fs::read(state).unwrap(), oversized);
}

#[test]
fn only_the_bound_play_root_publishes_on_a_shared_host() {
    let allowed = Fixture::new();
    let allowed_queue = eligible_queue(&allowed);
    let other = Fixture::new();
    let other_queue = eligible_queue(&other);
    let authorized_calls = Cell::new(0);
    let factory = |_: &Queue| {
        authorized_calls.set(authorized_calls.get() + 1);
        Some(Fake::new(Creation::Created(97)))
    };

    run_publisher(&other.0, &allowed.0, factory);
    run_publisher(&allowed.0, Path::new("."), factory);
    run_publisher(&allowed.0, &allowed.0.join("missing"), factory);
    assert_eq!(authorized_calls.get(), 0);
    assert_eq!(
        other_queue.read().unwrap().reports[0].delivery,
        Delivery::Ready
    );

    run_publisher(&allowed.0, &allowed.0, factory);
    assert_eq!(authorized_calls.get(), 1);
    assert_eq!(
        allowed_queue.read().unwrap().reports[0].delivery,
        Delivery::Linked(97)
    );
}

#[test]
fn ruleset_observation_reads_regular_manifests_and_rejects_directories() {
    let fixture = Fixture::new();
    let combat = fixture.0.join("combat");
    fs::create_dir(&combat).unwrap();
    let manifest = combat.join("campaign.yaml");
    fs::create_dir(&manifest).unwrap();
    assert_eq!(ruleset_for_save(&fixture.0), None);
    fs::remove_dir(&manifest).unwrap();
    fs::write(&manifest, "ruleset: 5e-srd\n").unwrap();
    assert_eq!(ruleset_for_save(&fixture.0), Some(Ruleset::FiveESrd));
}

#[cfg(unix)]
#[test]
fn feedback_foreground_rejects_fifo_and_symlinks() {
    let fixture = Fixture::new();
    let combat = fixture.0.join("combat");
    fs::create_dir(&combat).unwrap();
    let manifest = combat.join("campaign.yaml");
    let unrelated = fixture.0.join("unrelated.yaml");
    fs::write(&unrelated, "ruleset: 5e-srd\n").unwrap();
    std::os::unix::fs::symlink(&unrelated, &manifest).unwrap();
    assert_eq!(ruleset_for_save(&fixture.0), None);
    fs::remove_file(&manifest).unwrap();

    let queue = fixture.queue();
    std::os::unix::fs::symlink(&unrelated, queue.0.join("record.lock")).unwrap();
    assert!(queue.lock("record.lock", 1).is_err());
    std::os::unix::fs::symlink(&unrelated, queue.0.join("state.json")).unwrap();
    assert!(queue.read().is_err());

    assert!(std::process::Command::new("mkfifo")
        .arg(&manifest)
        .status()
        .unwrap()
        .success());
    let (sender, receiver) = std::sync::mpsc::channel();
    let save = fixture.0.clone();
    std::thread::spawn(move || sender.send(ruleset_for_save(&save)).unwrap());
    assert_eq!(receiver.recv_timeout(Duration::from_secs(2)).unwrap(), None);
}
