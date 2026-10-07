//! The `patchcov` command-line entry point.

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    match patchcov::cli::Cli::parse().execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err:#}");
            ExitCode::from(patchcov::cli::exit::code(&err))
        }
    }
}
