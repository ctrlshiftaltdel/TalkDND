fn main() {
    if let Err(e) = talkdnd::run(std::env::args_os()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
