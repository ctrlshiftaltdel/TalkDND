fn main() -> anyhow::Result<()> {
    if combat_sim::playtest::worker_if_requested() {
        return Ok(());
    }
    combat_sim::cli::run()
}
