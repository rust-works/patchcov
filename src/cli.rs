//! The `patchcov` command line.

pub(crate) mod diff;
pub(crate) mod lint_markers;
pub(crate) mod merge;

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};

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
        };
        assert!(cmd.execute().is_err());
        assert!(!output.exists());
    }
}
