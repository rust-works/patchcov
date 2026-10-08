//! The `patchcov` command line.

pub(crate) mod diff;
#[cfg(test)]
mod doc_fields;
pub mod exit;
pub(crate) mod lint_markers;
pub(crate) mod merge;
pub mod warn;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

use self::exit::ErrorReport;

/// Environment variable that sets `--error-format`, when the flag is absent.
pub const ERROR_FORMAT_ENV: &str = "PATCHCOV_ERROR_FORMAT";

/// How a failure and the warnings are printed to stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum ErrorFormat {
    /// `Error: <message>: <cause>...` and `warning: <message>` on one line each.
    #[default]
    Text,
    /// One line of JSON each, see `docs/reference.md#error-output` and
    /// `docs/reference.md#warnings`.
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

    /// How a failure and the warnings are printed to stderr: `text` (an `Error:`
    /// or `warning:` line) or `json` (one machine-readable object per line).
    ///
    /// Without the flag, `PATCHCOV_ERROR_FORMAT` is used; see
    /// [`Cli::resolve_error_format`].
    #[arg(
        long = "error-format",
        global = true,
        value_enum,
        value_name = "FORMAT"
    )]
    pub error_format: Option<ErrorFormat>,
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
    /// The format failures are printed in: the flag, else `env` (the value of
    /// [`ERROR_FORMAT_ENV`]), else text.
    ///
    /// An empty variable counts as unset. One that is neither `text` nor `json` is
    /// ignored with the returned warning rather than failing every command over a
    /// purely diagnostic setting.
    pub fn resolve_error_format(&self, env: Option<&OsStr>) -> (ErrorFormat, Option<String>) {
        if let Some(format) = self.error_format {
            return (format, None);
        }
        let Some(env) = env else {
            return (ErrorFormat::Text, None);
        };
        match env.to_str() {
            Some("" | "text") => (ErrorFormat::Text, None),
            Some("json") => (ErrorFormat::Json, None),
            _ => (
                ErrorFormat::Text,
                Some(format!(
                    "ignoring {ERROR_FORMAT_ENV}={}: expected 'text' or 'json'",
                    env.to_string_lossy()
                )),
            ),
        }
    }

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
    // The error and its tips, without the usage block and the pointer to `--help`.
    let rendered = err.render().to_string();
    let end = ["\n\nUsage:", "\n\nFor more information"]
        .iter()
        .filter_map(|trailer| rendered.find(trailer))
        .min()
        .unwrap_or(rendered.len());
    let message = rendered[..end].trim();
    Some(ErrorReport::usage(
        message.strip_prefix("error: ").unwrap_or(message),
    ))
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
            error_format: None,
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
            error_format: None,
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

    fn cli_with(error_format: Option<ErrorFormat>) -> Cli {
        Cli {
            command: Commands::Merge(merge::MergeCommand {
                report: Vec::new(),
                report_format: diff::ReportFormat::Auto,
                output: PathBuf::from("out.lcov"),
                strip_prefix: None,
            }),
            repo: None,
            error_format,
        }
    }

    #[test]
    fn the_flag_wins_over_the_environment_and_text_is_the_default() {
        let env = |value: &'static str| Some(OsStr::new(value));
        let resolve = |flag, value| cli_with(flag).resolve_error_format(value);
        assert_eq!(resolve(None, None), (ErrorFormat::Text, None));
        assert_eq!(resolve(None, env("json")), (ErrorFormat::Json, None));
        assert_eq!(resolve(None, env("text")), (ErrorFormat::Text, None));
        assert_eq!(resolve(None, env("")), (ErrorFormat::Text, None));
        let flag = Some(ErrorFormat::Text);
        assert_eq!(resolve(flag, env("json")), (ErrorFormat::Text, None));
        let flag = Some(ErrorFormat::Json);
        assert_eq!(resolve(flag, env("text")), (ErrorFormat::Json, None));
        // A bad variable is ignored with a warning, unless the flag makes it moot.
        let (format, warning) = resolve(None, env("xml"));
        assert_eq!(format, ErrorFormat::Text);
        assert!(warning.unwrap().contains("PATCHCOV_ERROR_FORMAT=xml"));
        assert_eq!(resolve(flag, env("xml")), (ErrorFormat::Json, None));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let (format, warning) = resolve(None, Some(OsStr::from_bytes(b"\xff")));
            assert_eq!(format, ErrorFormat::Text);
            assert!(warning.is_some());
        }
    }

    #[test]
    fn the_flag_parses_before_or_after_the_subcommand() {
        for argv in [
            ["--error-format", "json", "lint-markers"],
            ["lint-markers", "--error-format", "json"],
        ] {
            let cli = Cli::try_parse_from(std::iter::once("patchcov").chain(argv)).unwrap();
            assert_eq!(cli.error_format, Some(ErrorFormat::Json));
        }
        let cli = Cli::try_parse_from(["patchcov", "lint-markers"]).unwrap();
        assert_eq!(cli.error_format, None);
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

    /// clap's tips are part of the message; its usage block is not.
    #[test]
    fn a_parse_failure_keeps_clap_s_tips() {
        let argv = args(&["lint-marker", "--error-format=json"]);
        let err = Cli::try_parse_from(&argv).err().unwrap();
        let report = usage_report(&err, &argv, None).unwrap();
        assert!(
            report.message.contains("lint-markers"),
            "{}",
            report.message
        );
        assert!(report.message.contains("tip:"), "{}", report.message);
        assert!(!report.message.contains("Usage:"), "{}", report.message);
        assert!(!report.message.contains("--help"), "{}", report.message);
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
