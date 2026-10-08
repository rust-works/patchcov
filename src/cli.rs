//! The `patchcov` command line.

pub(crate) mod diff;
pub mod exit;
pub(crate) mod lint_markers;
pub(crate) mod merge;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};

use self::exit::ErrorReport;

/// Environment variable that sets `--error-format`.
pub const ERROR_FORMAT_ENV: &str = "PATCHCOV_ERROR_FORMAT";

/// How a failure is printed to stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorFormat {
    /// `Error: <message>: <cause>...` on one line.
    #[default]
    Text,
    /// One line of JSON, see `docs/reference.md#error-output`.
    Json,
}

/// Patch coverage for git diffs.
#[derive(Parser)]
#[command(name = "patchcov", version, about, long_about = None)]
pub struct Cli {
    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,

    /// Run as if patchcov was started in `<PATH>` instead of the current
    /// working directory. Mirrors `git -C`.
    #[arg(long = "repo", short = 'C', global = true, value_name = "PATH")]
    pub repo: Option<PathBuf>,

    /// How a failure is printed to stderr: `text` (an `Error:` line) or `json`
    /// (one machine-readable object).
    #[arg(
        long = "error-format",
        global = true,
        value_name = "text|json",
        value_parser = parse_error_format,
        env = ERROR_FORMAT_ENV,
        default_value = "text"
    )]
    pub error_format: ErrorFormat,
}

/// Subcommands.
#[derive(Subcommand)]
pub enum Commands {
    /// Analyses diff/patch coverage from a per-line report and a git diff.
    Diff(Box<diff::DiffCommand>),
    /// Checks source coverage markers without generating a coverage report.
    LintMarkers(lint_markers::LintMarkersCommand),
    /// Merges the per-shard reports of a sharded run into one lcov file.
    Merge(merge::MergeCommand),
}

impl Cli {
    /// Executes the command.
    ///
    /// `-C/--repo` is resolved here (`None` = current working directory) and
    /// threaded explicitly to the leaf.
    pub fn execute(self) -> Result<()> {
        let repo: Option<&Path> = self.repo.as_deref();
        match self.command {
            Commands::Diff(cmd) => cmd.execute(repo),
            Commands::LintMarkers(cmd) => cmd.execute(repo),
            Commands::Merge(cmd) => cmd.execute(repo),
        }
    }
}

/// Parses `--error-format`. An empty value is the default, so an empty
/// [`ERROR_FORMAT_ENV`] counts as unset, like the other environment variables.
fn parse_error_format(value: &str) -> Result<ErrorFormat, String> {
    match value {
        "" | "text" => Ok(ErrorFormat::Text),
        "json" => Ok(ErrorFormat::Json),
        _ => Err("expected 'text' or 'json'".to_string()),
    }
}

/// The JSON report for a failure of the argument parser, when JSON was asked for.
///
/// Parsing failed, so `--error-format` was never parsed: look for it in `args`
/// (the last occurrence wins, and `--` ends the options) and fall back to `env`,
/// the value of [`ERROR_FORMAT_ENV`]. `None` for `--help` and `--version`, which
/// are not failures, and when the text format is wanted.
pub fn usage_report(
    err: &clap::Error,
    args: &[OsString],
    env: Option<&OsString>,
) -> Option<ErrorReport> {
    if !err.use_stderr() || !json_requested(args, env) {
        return None;
    }
    let rendered = err.render().to_string();
    let first = rendered.split("\n\n").next().unwrap_or_default();
    let message = first.strip_prefix("error: ").unwrap_or(first);
    Some(ErrorReport::usage(message.trim()))
}

/// Whether `args` (or, without a flag, `env`) ask for `--error-format json`.
fn json_requested(args: &[OsString], env: Option<&OsString>) -> bool {
    let mut value = env.map(OsString::as_os_str);
    let mut args = args.iter().skip(1).map(OsString::as_os_str);
    while let Some(arg) = args.next() {
        let Some(arg) = arg.to_str() else { continue };
        if arg == "--" {
            break;
        }
        if let Some(inline) = arg.strip_prefix("--error-format=") {
            value = Some(inline.as_ref());
        } else if arg == "--error-format" {
            value = args.next().or(value);
        }
    }
    value == Some("json".as_ref())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// The CLI dispatches to `diff`; a missing report makes the
    /// leaf command error, which exercises the dispatch path end-to-end.
    #[test]
    fn dispatches_to_diff() {
        let cmd = Cli {
            command: Commands::Diff(Box::new(diff::DiffCommand {
                report: vec![std::path::PathBuf::from("/nonexistent/report.lcov")],
                report_format: diff::ReportFormat::Auto,
                branch_coverage: false,
                base_ref: Some("HEAD".to_string()),
                head_ref: None,
                baseline_report: None,
                baseline_report_format: diff::ReportFormat::Auto,
                output: diff::OutputFormatArg::Markdown,
                no_explanation: false,
                format: None,
                fail_under_patch: None,
                fail_on_unmeasured: Vec::new(),
                fail_under_lines: None,
                allow_path_mismatch: false,
                fail_on_path_mismatch: false,
                strip_prefix: None,
                ignore_filename_regex: Vec::new(),
                config_dir: None,
                collapse_ranges: false,
                all_files: false,
                artifact_url: None,
                run_url: None,
                base_sha: None,
                head_sha: None,
                commit_url: None,
            })),
            repo: None,
            error_format: ErrorFormat::Text,
        };
        // Reaches the leaf command and fails on the missing report file.
        assert!(cmd.execute().is_err());
    }

    /// The same for `merge`: a missing shard fails the leaf, and the output is
    /// never created.
    #[test]
    fn dispatches_to_merge() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("merged.lcov");
        let cmd = Cli {
            command: Commands::Merge(merge::MergeCommand {
                report: vec![dir.path().join("missing.lcov")],
                report_format: diff::ReportFormat::Auto,
                output: output.clone(),
                strip_prefix: None,
            }),
            repo: None,
            error_format: ErrorFormat::Text,
        };
        assert!(cmd.execute().is_err());
        assert!(!output.exists());
    }

    fn args(list: &[&str]) -> Vec<OsString> {
        std::iter::once("patchcov")
            .chain(list.iter().copied())
            .map(OsString::from)
            .collect()
    }

    #[test]
    fn error_formats_parse() {
        assert_eq!(parse_error_format("text"), Ok(ErrorFormat::Text));
        assert_eq!(parse_error_format("json"), Ok(ErrorFormat::Json));
        assert_eq!(parse_error_format(""), Ok(ErrorFormat::Text));
        assert!(parse_error_format("JSON").is_err());
        assert_eq!(ErrorFormat::default(), ErrorFormat::Text);
    }

    #[test]
    fn json_is_found_in_the_arguments_or_the_environment() {
        let json = OsString::from("json");
        let text = OsString::from("text");
        assert!(!json_requested(&args(&["diff"]), None));
        assert!(json_requested(&args(&["--error-format", "json"]), None));
        assert!(json_requested(
            &args(&["diff", "--error-format=json"]),
            None
        ));
        assert!(json_requested(&args(&["diff"]), Some(&json)));
        // The flag wins over the variable, and the last flag wins.
        assert!(!json_requested(
            &args(&["--error-format=text"]),
            Some(&json)
        ));
        assert!(json_requested(
            &args(&["--error-format=text", "--error-format", "json"]),
            None
        ));
        assert!(!json_requested(
            &args(&["--error-format=json", "--error-format=text"]),
            None
        ));
        assert!(!json_requested(&args(&["diff"]), Some(&text)));
        // A flag with no value falls back to the variable; `--` ends the options.
        assert!(json_requested(&args(&["--error-format"]), Some(&json)));
        assert!(!json_requested(&args(&["--", "--error-format=json"]), None));
    }

    #[test]
    fn a_parse_failure_is_reported_when_json_is_asked_for() {
        let argv = args(&["diff", "--error-format=json"]);
        let err = Cli::try_parse_from(&argv).err().unwrap();
        let report = usage_report(&err, &argv, None).unwrap();
        assert_eq!((report.code, report.kind), (2, "usage"));
        assert!(report.message.contains("--report"), "{}", report.message);
        assert!(!report.message.contains("Usage:"), "{}", report.message);
        assert!(!report.message.starts_with("error:"), "{}", report.message);
        assert!(report.chain.is_empty());
        // Not asked for: clap prints it.
        assert!(usage_report(&err, &args(&["diff"]), None).is_none());
    }

    #[test]
    fn help_and_version_are_not_failures() {
        for flag in ["--help", "--version"] {
            let argv = args(&[flag, "--error-format=json"]);
            let err = Cli::try_parse_from(&argv).err().unwrap();
            assert!(usage_report(&err, &argv, None).is_none(), "{flag}");
        }
    }
}
