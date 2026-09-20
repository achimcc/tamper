use std::process::ExitCode;

use tamper::verdict::Verdict;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("rules") => {
            for v in Verdict::ALL {
                println!("{:<14} {}", v.slug(), v.explain());
            }
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("tamper: unknown command {other:?}; try `tamper rules`");
            ExitCode::from(2)
        }
    }
}
