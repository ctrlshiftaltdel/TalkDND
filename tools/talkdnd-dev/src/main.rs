#![forbid(unsafe_code)]

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    match dispatch(&env::args().skip(1).collect::<Vec<_>>()) {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("talkdnd-dev: {err}");
            ExitCode::from(2)
        }
    }
}

fn dispatch(args: &[String]) -> Result<u8, String> {
    match args.first().map(String::as_str) {
        Some("check-leaks") => {
            if args.len() != 1 {
                usage("usage: talkdnd-dev check-leaks");
                return Ok(2);
            }
            let root = talkdnd_dev::repo_root()?;
            let hits = talkdnd_dev::leaks::scan(&root)?;
            for hit in &hits {
                println!("{hit}");
            }
            Ok(if hits.is_empty() { 0 } else { 1 })
        }
        Some("migrate-layout") => {
            let apply = match args.get(1).map(String::as_str) {
                None => false,
                Some("--apply") if args.len() == 2 => true,
                _ => {
                    usage("usage: talkdnd-dev migrate-layout [--apply]");
                    return Ok(2);
                }
            };
            let root = talkdnd_dev::repo_root()?;
            let outcome = talkdnd_dev::migrate::run(&root, apply)?;
            print!("{}", outcome.stdout());
            Ok(0)
        }
        Some("check-letsplay") => {
            if args.len() != 1 {
                usage("usage: talkdnd-dev check-letsplay");
                return Ok(2);
            }
            let root = talkdnd_dev::repo_root()?;
            let hits = talkdnd_dev::letsplay::scan(&root)?;
            for hit in &hits {
                println!("{hit}");
            }
            Ok(if hits.is_empty() { 0 } else { 1 })
        }
        Some("fetch-binary") => talkdnd_dev::fetch::run(&args[1..]),
        Some("install-local") => talkdnd_dev::install::run(&args[1..]),
        _ => {
            usage(
                "usage: talkdnd-dev <check-leaks|check-letsplay|migrate-layout|fetch-binary|install-local>",
            );
            Ok(2)
        }
    }
}

fn usage(text: &str) {
    eprintln!("{text}");
}
