//! The `patchcov` command-line entry point.

use clap::Parser;

fn main() {
    if let Err(err) = patchcov::cli::Cli::parse().execute() {
        eprintln!("Error: {err:#}");
        std::process::exit(1);
    }
}
