//! The entry point. It calls the composition root and nothing else. It names
//! no adapter.

use std::process::ExitCode;

fn main() -> ExitCode {
    connect_four::main_with_args(std::env::args().collect())
}
