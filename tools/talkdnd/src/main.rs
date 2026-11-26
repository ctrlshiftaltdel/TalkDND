fn main() {
    if combat_sim::playtest::worker_if_requested() {
        return;
    }
    if let Err(e) = talkdnd::run(std::env::args_os()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
