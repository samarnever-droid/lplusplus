//! `lpp-link` — standalone CLI binary delegating to `lpp::linker`.

use std::env;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Err(e) = lpp::linker::link_cli(&args) {
        eprintln!("lpp-link error: {e}");
        std::process::exit(1);
    }
}
