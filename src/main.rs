//! The `patchcov` command-line entry point.

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;
use patchcov::cli::exit::{self, ErrorReport};
use patchcov::cli::{usage_report, Cli, ErrorFormat, ERROR_FORMAT_ENV};

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(err) => {
            let env = std::env::var_os(ERROR_FORMAT_ENV);
            return match usage_report(&err, &args, env.as_ref()) {
                Some(report) => {
                    eprintln!("{}", report.to_json());
                    ExitCode::from(report.code)
                }
                // Prints the message (or help) and exits with clap's own code.
                None => err.exit(),
            };
        }
    };
    let (error_format, warning) =
        cli.resolve_error_format(std::env::var_os(ERROR_FORMAT_ENV).as_deref());
    if let Some(warning) = warning {
        eprintln!("warning: {warning}");
    }
    match cli.execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            match error_format {
                ErrorFormat::Text => eprintln!("Error: {err:#}"),
                ErrorFormat::Json => eprintln!("{}", ErrorReport::new(&err).to_json()),
            }
            ExitCode::from(exit::code(&err))
        }
    }
}
