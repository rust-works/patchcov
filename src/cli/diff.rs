//! `patchcov diff` — diff/patch coverage analysis.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use git2::Repository;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use regex::RegexSet;

use super::exit::{Classify, ExitKind};
use super::warn::{warn, WarningKind};
use crate::analysis::{analyze_with_markers, ExcludedFiles, Markers};
use crate::config::{load_config_content, resolve_config_dir_at};
use crate::format::resolve as resolve_format;
use crate::markers::{self, FileMarkers};
use crate::merge::require_executable_lines;
use crate::paths::{self, PathMapping};
use crate::{
    default_base_ref, go_coverprofile, parse, render, CoverageReport, DiffModel, DiffScope,
    FileCoverage, Format, OutputFormat, RenderOptions,
};

/// Config file (under the discovered `.patchcov/` dir) that declares persistent
/// `patchcov diff` settings, unioned with the CLI flags. Missing ⇒ no-op.
const COVERAGE_CONFIG_FILE: &str = "config.yaml";

/// Coverage report format selector (CLI mirror of [`Format`] plus auto-detect).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum ReportFormat {
    /// Auto-detect from report content.
    Auto,
    /// lcov trace file.
    Lcov,
    /// llvm-cov JSON export (`cargo llvm-cov report --json`).
    LlvmCovJson,
    /// Cobertura XML.
    Cobertura,
    /// JaCoCo XML.
    Jacoco,
    /// Go `go test -coverprofile` output.
    GoCoverprofile,
}

impl ReportFormat {
    /// Converts to a concrete [`Format`], or `None` for auto-detection.
    fn into_format(self) -> Option<Format> {
        match self {
            Self::Auto => None,
            Self::Lcov => Some(Format::Lcov),
            Self::LlvmCovJson => Some(Format::LlvmCovJson),
            Self::Cobertura => Some(Format::Cobertura),
            Self::Jacoco => Some(Format::Jacoco),
            Self::GoCoverprofile => Some(Format::GoCoverprofile),
        }
    }
}

/// Output format selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum OutputFormatArg {
    /// Markdown PR comment (default).
    Markdown,
    /// YAML structured output.
    Yaml,
    /// JSON output.
    Json,
}

impl From<OutputFormatArg> for OutputFormat {
    fn from(arg: OutputFormatArg) -> Self {
        match arg {
            OutputFormatArg::Markdown => Self::Markdown,
            OutputFormatArg::Yaml => Self::Yaml,
            OutputFormatArg::Json => Self::Json,
        }
    }
}

/// Analyses diff/patch coverage from a per-line report and a git diff.
#[derive(Parser)]
pub struct DiffCommand {
    /// Head coverage report (lcov / llvm-cov-json / cobertura / jacoco / go-coverprofile);
    /// repeat once per shard to merge a sharded run.
    ///
    /// Pass one `--report` per shard of a sharded coverage run and
    /// they are merged before anything is computed. The merge is a union of the
    /// files and of each file's executable lines, taking the larger hit count
    /// for a line present in several — so a line any shard covered is covered,
    /// and a line only one shard instrumented is judged by that shard alone.
    /// With more than one `--report`, a shard with no executable lines fails the
    /// run (it would otherwise lower the result unnoticed). Every nonempty
    /// report with no tracked-file matches after normalization fails the run
    /// (or only warns with --allow-path-mismatch). Shard order does not affect output.
    #[arg(long, value_name = "PATH", required = true)]
    pub report: Vec<PathBuf>,

    /// Format of every `--report` (auto-detected by default).
    #[arg(long, value_enum, default_value_t = ReportFormat::Auto)]
    pub report_format: ReportFormat,

    /// Treat lines with missed branches as uncovered (lcov and Cobertura only).
    /// Applies to head, baseline, totals and both gates. Missing branch records
    /// retain line-only semantics. Use original reports: `merge` drops branches.
    #[arg(long)]
    pub branch_coverage: bool,

    /// Base revision to diff against (default: merge-base of the default branch and `HEAD`:
    /// `origin/HEAD`, else `origin/main`, `main`, `origin/master`, `master`).
    #[arg(long, value_name = "REV")]
    pub base_ref: Option<String>,

    /// Head revision the report was measured at (default: `HEAD`).
    #[arg(long, value_name = "REV")]
    pub head_ref: Option<String>,

    /// Optional baseline coverage report; enables project deltas and indirect-change detection.
    ///
    /// Takes one report. A baseline from a sharded run is one file: make it with
    /// `patchcov merge`.
    #[arg(long, value_name = "PATH")]
    pub baseline_report: Option<PathBuf>,

    /// Format of `--baseline-report` (auto-detected by default).
    #[arg(long, value_enum, default_value_t = ReportFormat::Auto)]
    pub baseline_report_format: ReportFormat,

    /// Output format.
    #[arg(short = 'o', long, value_enum, default_value_t = OutputFormatArg::Markdown)]
    pub output: OutputFormatArg,

    /// Omit the explanation block from JSON/YAML output; Markdown is unchanged.
    #[arg(long)]
    pub no_explanation: bool,

    /// Deprecated: use `-o`/`--output` instead.
    #[arg(long = "format", hide = true)]
    pub format: Option<OutputFormatArg>,

    /// Fail (non-zero exit) when patch coverage is below this percentage.
    #[arg(long, value_name = "PCT")]
    pub fail_under_patch: Option<f64>,

    /// Fail when a touched file absent from every report matches this repo-relative glob.
    /// Repeatable; unioned with config.yaml diff.require-measured. Use ** for directories.
    #[arg(long, value_name = "GLOB")]
    pub fail_on_unmeasured: Vec<String>,

    /// Fail (non-zero exit) when overall line coverage is below this percentage.
    ///
    /// Gates on the headline `Total` — covered over executable lines across the
    /// head report after `--ignore-filename-regex` and `ignore` markers — so it
    /// moves with the same exclusions the patch gate sees. The figure is derived
    /// from the report's per-line records, so it can differ slightly from
    /// `cargo llvm-cov report --summary-only`. A report with no executable lines
    /// fails the gate: there is nothing to measure, and passing would let an
    /// empty report slip through.
    #[arg(long, value_name = "PCT")]
    pub fail_under_lines: Option<f64>,

    /// Warn, instead of failing, when a nonempty head, shard or baseline report
    /// has no path matching a tracked file after normalization.
    ///
    /// Such a report cannot be joined to the diff, so the patch looks empty and
    /// every gate passes vacuously: that is an error by default. Use this only
    /// where it is legitimate. Unioned with config.yaml diff.allow-path-mismatch.
    #[arg(long)]
    pub allow_path_mismatch: bool,

    /// Deprecated and ignored: a path mismatch is an error by default, unless
    /// --allow-path-mismatch or diff.allow-path-mismatch says otherwise.
    #[arg(long, hide = true)]
    pub fail_on_path_mismatch: bool,

    /// Override the path prefix stripped from report file paths to make them
    /// repo-relative (default: the repository working directory).
    #[arg(long, value_name = "PATH")]
    pub strip_prefix: Option<PathBuf>,

    /// Exclude files whose repo-relative path matches any of these regexes
    /// from BOTH the head and baseline reports before computing the diff.
    ///
    /// Repeatable, or comma-separated. Matching is unanchored (partial), the
    /// same semantics as `cargo llvm-cov --ignore-filename-regex`, and is
    /// applied after `--strip-prefix` normalisation so the pattern matches the
    /// repo-relative path. Filtering both sides symmetrically keeps the total,
    /// per-file deltas, patch coverage, and indirect-change list — and the
    /// `--fail-under-patch` gate — computed over the same denominator, even
    /// when the baseline predates the exclusion.
    #[arg(long, value_name = "REGEX", value_delimiter = ',')]
    pub ignore_filename_regex: Vec<String>,

    /// Path to the config directory searched for `config.yaml` (defaults to
    /// the discovered `.patchcov/`, honoring `PATCHCOV_CONFIG_DIR`).
    ///
    /// `config.yaml`'s `diff.ignore-filename-regex` list is unioned with any
    /// `--ignore-filename-regex` passed here, so a repo can declare its
    /// CPU-conditional / run-to-run-nondeterministic files once in version
    /// control instead of threading the flag through every invocation.
    #[arg(long, value_name = "PATH")]
    pub config_dir: Option<PathBuf>,

    /// Collapse consecutive uncovered new lines into ranges (e.g. `9-11`).
    #[arg(long)]
    pub collapse_ranges: bool,

    /// Report per-file deltas and indirect changes for ALL files, not just the
    /// ones this diff touches.
    ///
    /// By default the project-delta and indirect-change sections are scoped to
    /// files the diff modifies, because coverage is measured by two independent
    /// test runs and lines in untouched files flip purely from run-to-run
    /// variance. This flag restores the unscoped (noisier) report.
    #[arg(long)]
    pub all_files: bool,

    /// Link to the full coverage-summary artifact (markdown footer).
    #[arg(long, value_name = "URL")]
    pub artifact_url: Option<String>,

    /// Link to the CI run (markdown footer).
    #[arg(long, value_name = "URL")]
    pub run_url: Option<String>,

    /// Base (merge-base) commit SHA shown in the markdown `Comparing` line.
    #[arg(long, value_name = "SHA")]
    pub base_sha: Option<String>,

    /// Head commit SHA shown in the markdown `Comparing` line.
    #[arg(long, value_name = "SHA")]
    pub head_sha: Option<String>,

    /// Commit-URL prefix for linking SHAs (e.g. `https://…/<repo>/commit`).
    #[arg(long, value_name = "URL")]
    pub commit_url: Option<String>,
}

/// Which revision's source to scan for markers.
#[derive(Debug, Clone, Copy)]
enum Revision<'a> {
    /// The head side. `Some(rev)` when `--head-ref` named one explicitly.
    Head(Option<&'a str>),
    /// The base side, always an explicit revision.
    Base(&'a str),
}

/// Reads source files from one revision.
///
/// The head side prefers the **working tree** when `--head-ref` was not given:
/// that is the source the report was measured from, and it lets a marker take
/// effect while its author is still writing it, before any commit. An explicit
/// `--head-ref` (or a repository with no working tree) reads that revision's
/// tree instead, so the scan always matches the revision the report describes.
enum RevisionSource<'repo> {
    /// Files on disk, under this working directory.
    Workdir(PathBuf),
    /// Blobs from a revision's tree, with the repository they belong to.
    Tree(&'repo Repository, git2::Tree<'repo>),
}

impl<'repo> RevisionSource<'repo> {
    /// Resolves the source for `revision`.
    fn open(repo: &'repo Repository, revision: Revision<'_>) -> Result<Self> {
        let rev = match revision {
            Revision::Head(None) => match repo.workdir() {
                Some(workdir) => return Ok(Self::Workdir(workdir.to_path_buf())),
                None => "HEAD",
            },
            Revision::Head(Some(rev)) | Revision::Base(rev) => rev,
        };
        let tree = repo
            .revparse_single(rev)
            .with_context(|| format!("could not resolve ref `{rev}` to scan for coverage markers"))
            .classify(ExitKind::Git)?
            .peel_to_tree()
            .with_context(|| format!("ref `{rev}` is not a tree-ish"))
            .classify(ExitKind::Git)?;
        Ok(Self::Tree(repo, tree))
    }

    /// Reads `path`, or `None` when the revision has no readable text there.
    fn read(&self, path: &str) -> Result<Option<String>> {
        match self {
            Self::Workdir(workdir) => match std::fs::read(workdir.join(path)) {
                Ok(bytes) => Ok(String::from_utf8(bytes).ok()),
                // Generated code and out-of-tree paths appear in a report
                // without existing in the checkout.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(anyhow::Error::new(e).context(format!(
                    "could not read {} to scan for coverage markers",
                    workdir.join(path).display()
                )))
                .classify(ExitKind::Other),
            },
            Self::Tree(repo, tree) => {
                let Ok(entry) = tree.get_path(Path::new(path)) else {
                    return Ok(None);
                };
                let object = entry
                    .to_object(repo)
                    .with_context(|| {
                        format!(
                            "could not read `{path}` from the tree to scan for coverage markers"
                        )
                    })
                    .classify(ExitKind::Git)?;
                let Some(blob) = object.as_blob() else {
                    return Ok(None);
                };
                Ok(String::from_utf8(blob.content().to_vec()).ok())
            }
        }
    }
}

/// Removes every `ignore`d line from `report`, and any file left empty.
///
/// This is what makes `ignore` the scoped twin of `ignore-filename-regex`:
/// applied to each side from its own revision's source, before any delta is
/// computed, so the lines simply do not exist for the rest of the pipeline.
fn apply_ignored(report: &mut CoverageReport, markers: &BTreeMap<String, FileMarkers>) {
    if markers.values().all(|m| m.ignored.is_empty()) {
        return;
    }
    report.retain_lines(|path, line| markers.get(path).is_none_or(|m| !m.ignored.contains(&line)));
}

/// Resolves a relative report `path` against `repo_root`, so the report and the
/// git repository always anchor to the same root; an absolute `path` is kept.
pub(super) fn anchor(path: &Path, repo_root: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    }
}

/// Reads and parses the report at `path`.
///
/// Paths are left as the tool wrote them, except in a Go coverprofile, whose file
/// names are import paths: those lose the leading module path declared by the
/// `go.mod` in `repo_root`, which makes them repo-relative. A profile of a module
/// that is not at the repository root, or with no readable `go.mod`, is left
/// as written; `--strip-prefix <module path>` maps those.
pub(super) fn read_report(
    path: &Path,
    format: ReportFormat,
    repo_root: &Path,
) -> Result<CoverageReport> {
    read_report_mode(path, format, repo_root, false)
}

/// Reads a report, optionally retaining branch evidence for supported formats.
fn read_report_mode(
    path: &Path,
    format: ReportFormat,
    repo_root: &Path,
    branch_coverage: bool,
) -> Result<CoverageReport> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("could not read coverage report {}", path.display()))
        .classify(ExitKind::Report)?;
    let resolved = resolve_format(&content, format.into_format())
        .with_context(|| format!("could not parse coverage report {}", path.display()))
        .classify(ExitKind::Report)?;
    let parsed = if branch_coverage {
        match resolved {
            Format::Lcov => crate::lcov::parse_with_branches(&content),
            Format::Cobertura => crate::cobertura::parse_with_branches(&content),
            _ => {
                return Err(ExitKind::Usage.error(format!(
                    "--branch-coverage supports only lcov and Cobertura reports: {}",
                    path.display()
                )))
            }
        }
    } else {
        parse(&content, Some(resolved))
    };
    let mut report = parsed
        .with_context(|| format!("could not parse coverage report {}", path.display()))
        .classify(ExitKind::Report)?;
    if resolved == Format::GoCoverprofile {
        if let Some(module) = std::fs::read_to_string(repo_root.join("go.mod"))
            .ok()
            .and_then(|go_mod| go_coverprofile::module_path(&go_mod))
        {
            report.strip_prefix(Path::new(&module));
        }
    }
    Ok(report)
}

/// Checks normalized paths against tracked files at the report's revision.
struct ReportPathCheck<'a> {
    repo: &'a Repository,
    root: &'a Path,
    revision: Revision<'a>,
    strict: bool,
    warnings: &'a mut Vec<String>,
}

impl ReportPathCheck<'_> {
    fn check(&mut self, label: &Path, report: &CoverageReport) -> Result<()> {
        if report.files.is_empty() {
            return Ok(());
        }
        let matches = match self.revision {
            Revision::Head(None) if self.repo.workdir().is_some() => {
                let index = self.repo.index().classify(ExitKind::Git)?;
                // get_path panics on Windows prefixes and other invalid repo
                // paths. Compare stored git paths directly: unmatched report
                // paths are diagnostic input, not paths safe for that API.
                index.iter().any(|entry| {
                    std::str::from_utf8(&entry.path)
                        .is_ok_and(|path| report.files.contains_key(path))
                })
            }
            revision => {
                let rev = match revision {
                    Revision::Head(rev) => rev.unwrap_or("HEAD"),
                    Revision::Base(rev) => rev,
                };
                let tree = self
                    .repo
                    .revparse_single(rev)
                    .and_then(|object| object.peel_to_tree())
                    .classify(ExitKind::Git)?;
                report.files.keys().any(|path| {
                    tree.get_path(Path::new(path))
                        .is_ok_and(|entry| entry.kind() == Some(git2::ObjectType::Blob))
                })
            }
        };
        if !matches {
            let sample = report
                .files
                .keys()
                .take(3)
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(", ");
            let message = format!(
                "coverage report {}: none of its {} file path(s) matches a tracked file in the repository; unmatched normalized paths: {sample}; use --strip-prefix or diff.path-mappings to make paths repo-relative",
                label.display(), report.files.len()
            );
            if self.strict {
                return Err(ExitKind::PathMismatch.error(format!(
                    "{message} (or pass --allow-path-mismatch / set diff.allow-path-mismatch to warn instead)"
                )));
            }
            self.warnings.push(message);
        }
        Ok(())
    }
}

/// Makes `report`'s paths repo-relative, then drops the files `ignore` excludes,
/// returning the dropped files so the caller can report them.
fn normalise_report(
    report: &mut CoverageReport,
    strip_prefix: Option<&Path>,
    ignore: Option<&RegexSet>,
    label: &Path,
    check: &mut ReportPathCheck<'_>,
) -> Result<Vec<FileCoverage>> {
    if let Some(prefix) = strip_prefix {
        report.strip_prefix(prefix);
    }
    check.check(label, report)?;
    // Match on the repo-relative path (post strip-prefix), so the same
    // pattern applies identically to head and baseline.
    Ok(match ignore {
        Some(ignore) => report.retain_paths(|path| !ignore.is_match(path)),
        None => Vec::new(),
    })
}

/// Persistent `patchcov diff` settings read from `.patchcov/config.yaml`.
///
/// Forward-compatible: unknown top-level keys are ignored, so a newer schema
/// stays readable by an older binary. A missing `diff` block defaults to empty.
#[derive(Debug, Default, serde::Deserialize)]
pub(super) struct CoverageConfig {
    /// Settings for the `diff` subcommand.
    #[serde(default)]
    diff: CoverageDiffConfig,
    /// Settings for the `lint-markers` subcommand.
    #[serde(default, rename = "lint-markers")]
    pub(super) lint_markers: CoverageLintMarkersConfig,
}

/// The `lint-markers:` block of `config.yaml`.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) struct CoverageLintMarkersConfig {
    /// Repo-relative globs narrowing which tracked files `lint-markers` scans
    /// when it is given no paths. Replaced by any `--include` on the command line.
    #[serde(default)]
    pub(super) include: Vec<String>,
}

/// The `diff:` block of `config.yaml`.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct CoverageDiffConfig {
    /// Explicit report directory prefixes mapped before strip-prefix and filters.
    #[serde(default)]
    path_mappings: Vec<PathMapping>,
    /// Globs requiring touched files to appear in at least one report.
    #[serde(default)]
    require_measured: Vec<String>,
    /// Downgrades the no-tracked-file-match error to a warning, like
    /// `--allow-path-mismatch`; either one enables it.
    #[serde(default)]
    allow_path_mismatch: bool,
    /// Repo-relative path regexes excluded from both head and baseline reports,
    /// unioned with `--ignore-filename-regex` and applied after `--strip-prefix`
    /// normalisation. Same unanchored semantics as the flag.
    #[serde(default)]
    ignore_filename_regex: Vec<String>,
}

/// Loads and parses `config.yaml` from a resolved config directory.
///
/// A missing file is the default (empty) config; a present-but-malformed one is
/// a hard error, so a typo in a *value* fails loudly instead of letting excluded
/// noise (or a wider lint scan) back in. Shared by every `coverage` subcommand
/// that reads the file.
pub(super) fn load_coverage_config(config_dir: &Path) -> Result<CoverageConfig> {
    let Some(content) =
        load_config_content(config_dir, COVERAGE_CONFIG_FILE).classify(ExitKind::Config)?
    else {
        return Ok(CoverageConfig::default());
    };
    serde_yaml::from_str(&content)
        .with_context(|| {
            format!(
                "could not parse coverage config {}/{COVERAGE_CONFIG_FILE}",
                config_dir.display()
            )
        })
        .classify(ExitKind::Config)
}

/// The result of running `patchcov diff`, separated from printing so it can be
/// exercised by tests and reused programmatically.
pub struct DiffOutcome {
    /// The rendered report in the requested format.
    pub rendered: String,
    /// Project-wide patch coverage percentage (`None` when no lines were added).
    pub patch_percent: Option<f64>,
    /// Whether `--fail-under-patch` was set and patch coverage fell below it.
    pub below_gate: bool,
    /// Unmeasured paths matching the strict CLI/config policy.
    pub unmeasured_failures: Vec<String>,
    /// Overall line coverage percentage of the head report (`None` when it has
    /// no executable lines).
    pub line_percent: Option<f64>,
    /// Whether `--fail-under-lines` was set and overall line coverage fell below
    /// it — or could not be measured at all.
    pub below_line_gate: bool,
    /// Non-fatal problems found while loading the reports (for example a shard
    /// measured under a different workspace root). [`DiffCommand::execute`]
    /// prints them to stderr.
    pub warnings: Vec<String>,
}

impl DiffCommand {
    /// Executes the command: prints the report and applies the coverage gates.
    ///
    /// `repo` is the repository location resolved at the CLI boundary
    /// (`None` = current working directory).
    pub fn execute(mut self, repo: Option<&Path>) -> Result<()> {
        if let Some(format) = self.format.take() {
            warn(
                WarningKind::Deprecated,
                "--format is deprecated; use -o/--output instead",
            );
            self.output = format;
        }
        if self.fail_on_path_mismatch {
            warn(
                WarningKind::Deprecated,
                "--fail-on-path-mismatch is deprecated and has no effect; a path mismatch is an error unless --allow-path-mismatch or diff.allow-path-mismatch is set",
            );
        }
        let outcome = self.run(repo)?;
        for warning in &outcome.warnings {
            warn(WarningKind::PathMismatch, warning);
        }
        println!("{}", outcome.rendered);
        let failures = self.gate_failures(&outcome);
        if !failures.is_empty() {
            return Err(ExitKind::Gate.error(failures.join("; ")));
        }
        Ok(())
    }

    /// One message per coverage gate `outcome` failed, empty when all pass.
    ///
    /// Every failed gate is reported, so fixing one does not just reveal the next.
    fn gate_failures(&self, outcome: &DiffOutcome) -> Vec<String> {
        let mut failures = Vec::new();
        if outcome.below_gate {
            failures.push(format!(
                "patch coverage {:.2}% is below the --fail-under-patch threshold of {:.2}%",
                outcome.patch_percent.unwrap_or(0.0),
                self.fail_under_patch.unwrap_or_default()
            ));
        }
        if outcome.below_line_gate {
            let threshold = self.fail_under_lines.unwrap_or_default();
            failures.push(match outcome.line_percent {
                Some(pct) => format!(
                    "line coverage {pct:.2}% is below the --fail-under-lines threshold of {threshold:.2}%"
                ),
                None => format!(
                    "the report has no executable lines, so the --fail-under-lines threshold of {threshold:.2}% cannot be met"
                ),
            });
        }
        if !outcome.unmeasured_failures.is_empty() {
            failures.push(format!("touched files absent from every coverage report (--fail-on-unmeasured / diff.require-measured): {}", outcome.unmeasured_failures.join(", ")));
        }
        failures
    }

    /// Runs the analysis and renders the output without printing.
    ///
    /// `repo_root` is the repository to analyze (`None` defaults to `.`, which
    /// preserves the CI invocation that runs from the repo root). Relative
    /// `--report`/`--baseline-report` paths are anchored to it so the git repo
    /// and the coverage reports always resolve against the same root.
    pub fn run(&self, repo_root: Option<&Path>) -> Result<DiffOutcome> {
        let repo_path = repo_root.map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let repo = Repository::open(&repo_path)
            .with_context(|| format!("could not open git repository at {}", repo_path.display()))
            .classify(ExitKind::Git)?;

        // Resolve the base ref (default: merge-base of the default branch and HEAD).
        let base_ref = match &self.base_ref {
            Some(r) => r.clone(),
            None => default_base_ref(&repo).classify(ExitKind::Git)?,
        };

        // Determine the prefix stripped from report paths to make them repo-relative.
        let strip_prefix = self
            .strip_prefix
            .clone()
            .or_else(|| repo.workdir().map(std::path::Path::to_path_buf));

        // Union the repo-config ignore-list with the CLI flag, then compile once
        // so an invalid pattern is a single up-front error rather than failing
        // separately per report.
        let config_dir = resolve_config_dir_at(self.config_dir.as_deref(), &repo_path);
        let config = load_coverage_config(&config_dir)?.diff;
        paths::validate(&config.path_mappings).classify(ExitKind::Config)?;
        let ignore = self.compile_ignore(&config.ignore_filename_regex)?;

        let require_measured = self.compile_require_measured(&config.require_measured)?;
        let mut warnings = Vec::new();
        let mut check = ReportPathCheck {
            repo: &repo,
            root: &repo_path,
            revision: Revision::Head(self.head_ref.as_deref()),
            strict: !(self.allow_path_mismatch || config.allow_path_mismatch),
            warnings: &mut warnings,
        };
        let mut excluded = ExcludedFiles::default();
        let head = self.load_head(
            &config.path_mappings,
            strip_prefix.as_deref(),
            ignore.as_ref(),
            &mut check,
            &mut excluded,
        )?;
        check.revision = Revision::Base(&base_ref);
        let baseline = match &self.baseline_report {
            Some(path) => Some(self.load_baseline(
                path,
                &config.path_mappings,
                strip_prefix.as_deref(),
                ignore.as_ref(),
                &mut check,
                &mut excluded,
            )?),
            None => None,
        };

        let diff = DiffModel::between(&repo, &base_ref, self.head_ref.as_deref())
            .classify(ExitKind::Git)?;
        // Preserve report presence before source markers can remove every line.
        let unmeasured_files = crate::analysis::unmeasured_files(&head, &diff, baseline.as_ref())
            .into_iter()
            .filter(|path| {
                ignore.as_ref().is_none_or(|set| {
                    !set.is_match(path)
                        && diff
                            .files
                            .get(path)
                            .and_then(|file| file.old_path.as_ref())
                            .is_none_or(|old| !set.is_match(old))
                })
            })
            .collect::<Vec<_>>();
        let unmeasured_failures = unmeasured_files
            .iter()
            .filter(|path| {
                require_measured
                    .as_ref()
                    .is_some_and(|set| set.is_match(path))
            })
            .cloned()
            .collect();

        // Source markers are read from each revision's *own* source, so no line
        // number is ever stored and a region that moved between base and head
        // needs no reconciliation. This runs after `strip_prefix` and the
        // filename ignore-list, so a file excluded there is never even read —
        // file-level exclusion wins, and markers inside it never raise errors.
        let mut markers = Markers {
            head: self.scan_markers(&repo, Revision::Head(self.head_ref.as_deref()), &head)?,
            base: match &baseline {
                Some(baseline) => self.scan_markers(&repo, Revision::Base(&base_ref), baseline)?,
                None => BTreeMap::new(),
            },
        };
        let mut head = head;
        let mut baseline = baseline;
        if self.branch_coverage {
            head.apply_branch_coverage();
            if let Some(baseline) = baseline.as_mut() {
                baseline.apply_branch_coverage();
            }
        }
        apply_ignored(&mut head, &markers.head);
        if let Some(baseline) = baseline.as_mut() {
            apply_ignored(baseline, &markers.base);
        }
        // An ignored file may have left the report entirely; its markers would
        // then be reported as applied without having applied to anything.
        markers.head.retain(|path, _| head.files.contains_key(path));

        let scope = if self.all_files {
            DiffScope::All
        } else {
            DiffScope::DiffOnly
        };
        let mut result = analyze_with_markers(&head, &diff, baseline.as_ref(), scope, &markers);
        excluded.resolve(&diff);
        result.excluded = excluded;
        result.unmeasured_files = unmeasured_files;

        let opts = self.render_options();
        let rendered = render(&result, &opts, self.output.into()).classify(ExitKind::Other)?;

        let patch_percent = result.patch.percent();
        let below_gate = match self.fail_under_patch {
            // No added lines ⇒ nothing to gate on; treat as a pass.
            Some(threshold) => patch_percent.is_some_and(|p| p < threshold),
            None => false,
        };

        // `total_after` is the headline total, so the gate and the rendered
        // `Total` can never disagree. Unlike the patch gate, an unmeasurable
        // total fails: "no added lines" is a property of the change, but "no
        // executable lines" means the report is empty or fully excluded.
        let line_percent = result.total_after;
        let below_line_gate = self
            .fail_under_lines
            .is_some_and(|threshold| line_percent.is_none_or(|p| p < threshold));

        Ok(DiffOutcome {
            rendered,
            patch_percent,
            below_gate,
            unmeasured_failures,
            line_percent,
            below_line_gate,
            warnings,
        })
    }

    /// Loads the head report: one `--report`, or the merge of every shard.
    ///
    /// Each shard is read, mapped, and stripped before checking tracked paths,
    /// then filtered and merged. An empty shard fails before filtering. The
    /// merge is a union, so shard order cannot change the result.
    fn load_head(
        &self,
        mappings: &[PathMapping],
        strip_prefix: Option<&Path>,
        ignore: Option<&RegexSet>,
        check: &mut ReportPathCheck<'_>,
        excluded: &mut ExcludedFiles,
    ) -> Result<CoverageReport> {
        if self.report.is_empty() {
            return Err(ExitKind::Usage.error("at least one coverage report is required"));
        }
        let sharded = self.report.len() > 1;
        let mut merged = CoverageReport::new();
        for path in &self.report {
            let path = anchor(path, check.root);
            let mut report =
                read_report_mode(&path, self.report_format, check.root, self.branch_coverage)?;
            report.map_paths(mappings).classify(ExitKind::Config)?;
            if sharded {
                require_executable_lines(&path.display().to_string(), &report)
                    .classify(ExitKind::Report)?;
            }
            excluded.record_head(normalise_report(
                &mut report,
                strip_prefix,
                ignore,
                &path,
                check,
            )?);
            merged.merge(report);
        }
        Ok(merged)
    }

    /// Reads and parses the baseline report, normalising paths to be repo-relative
    /// and recording the files `ignore` removed in `excluded`.
    ///
    /// A relative `path` is resolved against `repo_root` so the report and the
    /// git repository always anchor to the same root; an absolute `path` is
    /// used as-is.
    fn load_baseline(
        &self,
        path: &Path,
        mappings: &[PathMapping],
        strip_prefix: Option<&Path>,
        ignore: Option<&RegexSet>,
        check: &mut ReportPathCheck<'_>,
        excluded: &mut ExcludedFiles,
    ) -> Result<CoverageReport> {
        let path = anchor(path, check.root);
        let mut report = read_report_mode(
            &path,
            self.baseline_report_format,
            check.root,
            self.branch_coverage,
        )?;
        report.map_paths(mappings).classify(ExitKind::Config)?;
        excluded.record_baseline(normalise_report(
            &mut report,
            strip_prefix,
            ignore,
            &path,
            check,
        )?);
        Ok(report)
    }

    /// Scans one revision's source for coverage markers.
    ///
    /// Only paths present in `report` are read, so the cost is bounded by the
    /// report rather than by the repository, and a file the coverage run never
    /// mentioned is never opened.
    ///
    /// A path missing from the revision is **skipped silently**: generated code
    /// and out-of-tree paths legitimately appear in a report without existing in
    /// the tree. Any other read failure is a hard error — except a non-UTF-8
    /// file, which is skipped, since it can only ever *fail to find* a marker
    /// and so errs toward reporting more coverage movement, never less.
    fn scan_markers(
        &self,
        repo: &Repository,
        revision: Revision<'_>,
        report: &CoverageReport,
    ) -> Result<BTreeMap<String, FileMarkers>> {
        let mut found = BTreeMap::new();
        let source = RevisionSource::open(repo, revision)?;
        for path in report.files.keys() {
            let Some(text) = source.read(path)? else {
                continue;
            };
            let regions = markers::scan(path, &text).classify(ExitKind::Marker)?;
            if !regions.is_empty() {
                found.insert(path.clone(), FileMarkers::new(regions));
            }
        }
        Ok(found)
    }

    /// Compiles the union of `--ignore-filename-regex` and the repo-config
    /// ignore-list into a [`RegexSet`], or `None` when neither supplied any
    /// pattern. A file is excluded when it matches *any* pattern.
    ///
    /// Empty patterns are dropped: comma-splitting a trailing or doubled comma
    /// (`foo,` / `a,,b`) yields an empty element, and an empty regex matches
    /// *every* path — which would silently exclude the whole report (and make a
    /// `--fail-under-patch` gate pass vacuously). Treating it as a no-op is the
    /// safe reading of an obvious typo.
    fn compile_ignore(&self, config_patterns: &[String]) -> Result<Option<RegexSet>> {
        let patterns: Vec<&str> = self
            .ignore_filename_regex
            .iter()
            .chain(config_patterns)
            .map(String::as_str)
            .filter(|p| !p.is_empty())
            .collect();
        if patterns.is_empty() {
            return Ok(None);
        }
        let set = RegexSet::new(patterns)
            .context(
                "invalid ignore-filename-regex pattern (--ignore-filename-regex or config.yaml)",
            )
            .classify(ExitKind::Config)?;
        Ok(Some(set))
    }

    /// Compiles the union of CLI and config policies; invalid/empty globs fail loudly.
    fn compile_require_measured(&self, config: &[String]) -> Result<Option<GlobSet>> {
        let mut builder = GlobSetBuilder::new();
        if self.fail_on_unmeasured.is_empty() && config.is_empty() {
            return Ok(None);
        }
        for pattern in self.fail_on_unmeasured.iter().chain(config) {
            if pattern.is_empty() {
                return Err(ExitKind::Config
                    .error("empty glob in --fail-on-unmeasured / diff.require-measured"));
            }
            builder.add(GlobBuilder::new(pattern).literal_separator(true).build()
                .with_context(|| format!("invalid glob `{pattern}` in --fail-on-unmeasured / diff.require-measured"))
                .classify(ExitKind::Config)?);
        }
        Ok(Some(
            builder
                .build()
                .context("could not compile require-measured globs")
                .classify(ExitKind::Config)?,
        ))
    }

    /// Builds the render options, falling back to the `COVERAGE_*` environment
    /// variables CI sets when a flag is not supplied.
    fn render_options(&self) -> RenderOptions {
        fn or_env(flag: &Option<String>, var: &str) -> Option<String> {
            flag.clone()
                .or_else(|| std::env::var(var).ok())
                .filter(|s| !s.is_empty())
        }
        RenderOptions {
            artifact_url: or_env(&self.artifact_url, "COVERAGE_ARTIFACT_URL"),
            run_url: or_env(&self.run_url, "COVERAGE_RUN_URL"),
            base_sha: or_env(&self.base_sha, "COVERAGE_BASE_SHA"),
            head_sha: or_env(&self.head_sha, "COVERAGE_HEAD_SHA"),
            commit_url: or_env(&self.commit_url, "COVERAGE_COMMIT_URL"),
            collapse_ranges: self.collapse_ranges,
            no_explanation: self.no_explanation,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use git2::{Repository, Signature};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Creates a temp repo with a base commit (`a.rs`) and a head commit that
    /// adds `b.rs` with three lines. Returns the dir, repo path, and base SHA.
    fn repo_with_added_file() -> (TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let repo = Repository::init(&path).unwrap();
        {
            let mut cfg = repo.config().unwrap();
            cfg.set_str("user.name", "Test").unwrap();
            cfg.set_str("user.email", "test@example.com").unwrap();
        }

        let commit = |repo: &Repository, files: &[(&str, &str)], parent: Option<git2::Oid>| {
            let mut index = repo.index().unwrap();
            index.clear().unwrap();
            for (name, content) in files {
                fs::write(path.join(name), content).unwrap();
                index.add_path(Path::new(name)).unwrap();
            }
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let sig = Signature::now("Test", "test@example.com").unwrap();
            let parent = parent.map(|id| repo.find_commit(id).unwrap());
            let parents: Vec<&git2::Commit> = parent.as_ref().into_iter().collect();
            repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, &parents)
                .unwrap()
        };

        let base = commit(&repo, &[("a.rs", "fn a() {}\n")], None);
        commit(
            &repo,
            &[("a.rs", "fn a() {}\n"), ("b.rs", "one\ntwo\nthree\n")],
            Some(base),
        );
        // Return git2's canonical workdir: on macOS the tempdir `/var/...` is a
        // symlink to `/private/var/...`, and `Repository::open` resolves to the
        // latter. Using it for both the repo path and the report's `SF:` path
        // keeps `strip_prefix` (which defaults to the workdir) consistent.
        let workdir = repo.workdir().unwrap().to_path_buf();
        (dir, workdir, base.to_string())
    }

    /// Writes an lcov report for `b.rs` (line 1 & 3 covered, line 2 uncovered).
    fn write_head_lcov(repo_path: &Path) -> PathBuf {
        let lcov = format!(
            "SF:{}\nDA:1,1\nDA:2,0\nDA:3,4\nend_of_record\n",
            repo_path.join("b.rs").display()
        );
        let report = repo_path.join("head.lcov");
        fs::write(&report, lcov).unwrap();
        report
    }

    /// Builds a `DiffCommand` with defaults pointed at the given report. The
    /// repository root is supplied separately to `run`/`execute`.
    fn command(report: PathBuf, base_ref: &str) -> DiffCommand {
        DiffCommand {
            report: vec![report],
            report_format: ReportFormat::Auto,
            branch_coverage: false,
            base_ref: Some(base_ref.to_string()),
            head_ref: None,
            baseline_report: None,
            baseline_report_format: ReportFormat::Auto,
            output: OutputFormatArg::Markdown,
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
        }
    }

    /// Writes `<repo>/.patchcov/config.yaml` declaring `diff.ignore-filename-regex`
    /// and returns the config-dir path to pass as `config_dir`. An empty
    /// `patterns` emits an explicit `[]` (a bare `key:` would deserialize as
    /// `null`, not an empty sequence).
    fn write_coverage_config(repo: &Path, patterns: &[&str]) -> PathBuf {
        use std::fmt::Write as _;
        let config_dir = repo.join(".patchcov");
        fs::create_dir_all(&config_dir).unwrap();
        let mut body = String::from("diff:\n");
        if patterns.is_empty() {
            body.push_str("  ignore-filename-regex: []\n");
        } else {
            body.push_str("  ignore-filename-regex:\n");
            for p in patterns {
                let _ = writeln!(body, "    - '{p}'");
            }
        }
        fs::write(config_dir.join("config.yaml"), body).unwrap();
        config_dir
    }

    #[test]
    fn unmeasured_files_render_and_gate_only_matching_policy() {
        let (_dir, repo, base) = repo_with_added_file();
        let report = repo.join("head.lcov");
        fs::write(&report, "SF:a.rs\nDA:1,1\nend_of_record\n").unwrap();
        let mut cmd = command(report, &base);
        cmd.fail_under_patch = Some(80.0);
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert!(!outcome.below_gate);
        assert!(cmd.gate_failures(&outcome).is_empty());
        assert!(outcome
            .rendered
            .contains("Touched files absent from every coverage report (1)"));
        assert!(outcome.rendered.contains("`b.rs`"));
        for format in [OutputFormatArg::Json, OutputFormatArg::Yaml] {
            cmd.output = format;
            let outcome = cmd.run(Some(&repo)).unwrap();
            let value: serde_json::Value = if format == OutputFormatArg::Json {
                serde_json::from_str(&outcome.rendered).unwrap()
            } else {
                serde_yaml::from_str(&outcome.rendered).unwrap()
            };
            assert_eq!(value["unmeasured_files"], serde_json::json!(["b.rs"]));
        }
        cmd.fail_on_unmeasured = vec!["docs/**".into()];
        assert!(cmd.run(Some(&repo)).unwrap().unmeasured_failures.is_empty());
        cmd.fail_on_unmeasured.push("**/*.rs".into());
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.unmeasured_failures, ["b.rs"]);
        assert!(cmd.gate_failures(&outcome)[0].contains("b.rs"));
        assert!(cmd.execute(Some(&repo)).is_err());
    }

    #[test]
    fn require_measured_config_unions_cli_and_validates_before_loading() {
        let (_dir, repo, base) = repo_with_added_file();
        let dir = repo.join(".patchcov");
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("config.yaml"),
            "diff:\n  require-measured: ['**/*.rs']\n",
        )
        .unwrap();
        let report = repo.join("head.lcov");
        fs::write(&report, "SF:a.rs\nDA:1,1\nend_of_record\n").unwrap();
        let mut cmd = command(report, &base);
        cmd.config_dir = Some(dir);
        cmd.fail_on_unmeasured = vec!["docs/**".into()];
        assert_eq!(cmd.run(Some(&repo)).unwrap().unmeasured_failures, ["b.rs"]);
        cmd.report = vec![repo.join("missing.lcov")];
        for pattern in ["[", ""] {
            cmd.fail_on_unmeasured = vec![pattern.into()];
            assert!(cmd
                .run(Some(&repo))
                .err()
                .unwrap()
                .to_string()
                .contains("glob"));
        }
        let set = cmd.compile_require_measured(&["src/**/*.rs".into()]);
        assert!(set.is_err()); // invalid CLI policy still takes precedence
        cmd.fail_on_unmeasured.clear();
        let set = cmd
            .compile_require_measured(&["src/**/*.rs".into()])
            .unwrap()
            .unwrap();
        assert!(set.is_match("src/a.rs"));
        assert!(set.is_match("src/nested/a.rs"));
        assert!(!set.is_match("docs/a.rs"));
        assert!(!set.is_match("src/a.md"));
    }

    #[test]
    fn unmeasured_exclusions_baseline_shards_and_markers() {
        let (_dir, repo, base) = repo_with_added_file();
        let report = repo.join("head.lcov");
        fs::write(&report, "SF:a.rs\nDA:1,1\nend_of_record\n").unwrap();
        let mut cmd = command(report, &base);
        cmd.fail_on_unmeasured = vec!["**/*.rs".into()];
        cmd.ignore_filename_regex = vec!["^b\\.rs$".into()];
        assert!(cmd.run(Some(&repo)).unwrap().unmeasured_failures.is_empty());
        cmd.ignore_filename_regex.clear();
        let measured = write_head_lcov(&repo);
        // write_head_lcov overwrites head.lcov, so restore unrelated first report.
        let unrelated = repo.join("other.lcov");
        fs::write(&unrelated, "SF:a.rs\nDA:1,1\nend_of_record\n").unwrap();
        cmd.report = vec![unrelated];
        cmd.baseline_report = Some(measured.clone());
        // `b.rs` is not in the base tree, so this baseline matches no tracked file.
        cmd.allow_path_mismatch = true;
        assert!(cmd.run(Some(&repo)).unwrap().unmeasured_failures.is_empty());
        cmd.allow_path_mismatch = false;
        cmd.baseline_report = None;
        cmd.report.push(measured);
        assert!(cmd.run(Some(&repo)).unwrap().unmeasured_failures.is_empty());
        fs::write(
            repo.join("b.rs"),
            format!(
                "// {} coverage ignore reason=\"generated\"\none\n// {} coverage end\n",
                "patchcov:", "patchcov:"
            ),
        )
        .unwrap();
        assert!(cmd.run(Some(&repo)).unwrap().unmeasured_failures.is_empty());
    }

    #[test]
    fn fail_on_unmeasured_parses_repeatable_globs() {
        let cmd = DiffCommand::try_parse_from([
            "diff",
            "--report",
            "r.lcov",
            "--fail-on-unmeasured",
            "src/**/*.rs",
            "--fail-on-unmeasured",
            "lib/**/*.py",
        ])
        .unwrap();
        assert_eq!(cmd.fail_on_unmeasured, ["src/**/*.rs", "lib/**/*.py"]);
    }

    #[test]
    fn report_format_into_format() {
        assert_eq!(ReportFormat::Auto.into_format(), None);
        assert_eq!(ReportFormat::Jacoco.into_format(), Some(Format::Jacoco));
        assert_eq!(ReportFormat::Lcov.into_format(), Some(Format::Lcov));
        assert_eq!(
            ReportFormat::LlvmCovJson.into_format(),
            Some(Format::LlvmCovJson)
        );
        assert_eq!(
            ReportFormat::Cobertura.into_format(),
            Some(Format::Cobertura)
        );
        assert_eq!(
            ReportFormat::GoCoverprofile.into_format(),
            Some(Format::GoCoverprofile)
        );
    }

    #[test]
    fn output_format_arg_conversion() {
        assert_eq!(
            OutputFormat::from(OutputFormatArg::Markdown),
            OutputFormat::Markdown
        );
        assert_eq!(
            OutputFormat::from(OutputFormatArg::Yaml),
            OutputFormat::Yaml
        );
        assert_eq!(
            OutputFormat::from(OutputFormatArg::Json),
            OutputFormat::Json
        );
    }

    #[test]
    fn run_markdown_reports_patch_coverage() {
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        let outcome = command(report, &base).run(Some(&repo)).unwrap();
        // 3 added lines, 2 covered.
        assert_eq!(outcome.patch_percent, Some(2.0 / 3.0 * 100.0));
        assert!(!outcome.below_gate);
        assert!(outcome.rendered.contains("### Patch coverage"));
        assert!(outcome.rendered.contains("`b.rs:2`"));
    }

    #[test]
    fn run_yaml_and_json_formats() {
        let (_dir, repo, base) = repo_with_added_file();
        for format in [OutputFormatArg::Yaml, OutputFormatArg::Json] {
            let report = write_head_lcov(&repo);
            let mut cmd = command(report, &base);
            cmd.output = format;
            let outcome = cmd.run(Some(&repo)).unwrap();
            assert!(outcome.rendered.contains("patch_coverage"));
        }
    }

    #[test]
    fn no_explanation_removes_only_explanation_in_structured_output() {
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        for format in ["json", "yaml"] {
            for baseline in [false, true] {
                let mut cmd = DiffCommand::try_parse_from([
                    "diff",
                    "--report",
                    report.to_str().unwrap(),
                    "--base-ref",
                    &base,
                    "-o",
                    format,
                    "--fail-under-patch",
                    "99",
                ])
                .unwrap();
                assert!(!cmd.no_explanation);
                if baseline {
                    cmd.baseline_report = Some(report.clone());
                    // `b.rs` does not exist in the base tree, which this test is not about.
                    cmd.allow_path_mismatch = true;
                }
                let default = cmd.run(Some(&repo)).unwrap();
                let flagged = DiffCommand::try_parse_from([
                    "diff",
                    "--report",
                    report.to_str().unwrap(),
                    "--no-explanation",
                ])
                .unwrap();
                cmd.no_explanation = flagged.no_explanation;
                let compact = cmd.run(Some(&repo)).unwrap();
                let parse = |s: &str| -> serde_json::Value {
                    if format == "json" {
                        serde_json::from_str(s).unwrap()
                    } else {
                        serde_yaml::from_str(s).unwrap()
                    }
                };
                let mut expected = parse(&default.rendered);
                assert!(expected
                    .as_object_mut()
                    .unwrap()
                    .remove("explanation")
                    .is_some());
                let actual = parse(&compact.rendered);
                assert!(!actual.as_object().unwrap().contains_key("explanation"));
                assert_eq!(actual, expected, "format={format}, baseline={baseline}");
                assert_eq!(compact.patch_percent, default.patch_percent);
                assert!(default.below_gate);
                assert_eq!(compact.below_gate, default.below_gate);
                assert_eq!(compact.warnings, default.warnings);
            }
        }
    }

    #[test]
    fn no_explanation_leaves_markdown_unchanged() {
        let (_dir, repo, base) = repo_with_added_file();
        let mut cmd = command(write_head_lcov(&repo), &base);
        let default = cmd.run(Some(&repo)).unwrap().rendered;
        cmd.no_explanation = true;
        assert_eq!(cmd.run(Some(&repo)).unwrap().rendered, default);
    }

    /// Builds a marker line. The introducer is assembled at runtime so this
    /// file's own fixtures are not themselves markers when patchcov scans its
    /// own source — see `crate::markers::INTRODUCER`.
    fn marked(kind: &str, reason: &str) -> String {
        format!(
            "// {} {kind} reason=\"{reason}\"",
            crate::markers::INTRODUCER
        )
    }

    /// Builds a repo whose `src/gated.rs` carries a marked region sitting at
    /// **different line numbers on each side** — the property the whole design
    /// exists for. Base has the region at lines 1-4; head prepends two lines,
    /// moving it to 3-6. `kind` of `None` builds the unmarked control.
    ///
    /// The gated lines are covered at base and uncovered at head: exactly the
    /// cross-runner-CPU flip the markers exist to silence.
    fn repo_with_moved_marker_region(
        kind: Option<&str>,
    ) -> (TempDir, PathBuf, String, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let repo = Repository::init(&path).unwrap();
        {
            let mut cfg = repo.config().unwrap();
            cfg.set_str("user.name", "Test").unwrap();
            cfg.set_str("user.email", "test@example.com").unwrap();
        }

        let (open, close) = match kind {
            Some(kind) => (
                marked(kind, "CPU-gated dispatch"),
                format!("// {} end", crate::markers::INTRODUCER),
            ),
            None => (
                "// an ordinary comment".to_string(),
                "// another".to_string(),
            ),
        };
        let base_src =
            format!("{open}\nfn gated() {{}}\nfn also_gated() {{}}\n{close}\nfn plain() {{}}\n");
        // Two extra lines above shift the whole region down by two.
        let head_src = format!(
            "// a new comment\n// and another\n{open}\nfn gated() {{}}\nfn also_gated() {{}}\n{close}\nfn plain() {{}}\n"
        );

        let commit = |files: &[(&str, &str)], parent: Option<git2::Oid>| {
            let mut index = repo.index().unwrap();
            index.clear().unwrap();
            for (name, content) in files {
                let file = path.join(name);
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(&file, content).unwrap();
                index.add_path(Path::new(name)).unwrap();
            }
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let sig = Signature::now("Test", "test@example.com").unwrap();
            let parent = parent.map(|id| repo.find_commit(id).unwrap());
            let parents: Vec<&git2::Commit> = parent.as_ref().into_iter().collect();
            repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, &parents)
                .unwrap()
        };

        let base = commit(&[("src/gated.rs", &base_src)], None);
        commit(&[("src/gated.rs", &head_src)], Some(base));

        let workdir = repo.workdir().unwrap().to_path_buf();
        let gated = workdir.join("src/gated.rs");

        // Base: the two gated lines (2, 3) covered, `plain` (5) covered.
        let base_lcov = workdir.join("base.lcov");
        fs::write(
            &base_lcov,
            format!(
                "SF:{}\nDA:2,4\nDA:3,4\nDA:5,1\nend_of_record\n",
                gated.display()
            ),
        )
        .unwrap();
        // Head: the same two gated lines (now 4, 5) uncovered — a different
        // runner CPU — and `plain` (now 7) still covered.
        let head_lcov = workdir.join("head.lcov");
        fs::write(
            &head_lcov,
            format!(
                "SF:{}\nDA:4,0\nDA:5,0\nDA:7,1\nend_of_record\n",
                gated.display()
            ),
        )
        .unwrap();

        (dir, workdir, base.to_string(), head_lcov, base_lcov)
    }

    /// End-to-end: a `tolerate` region read from each revision's own source, at
    /// different line numbers on each side, masks the flip everywhere — headline
    /// and per-file row — while the reported percentage stays the real measured
    /// value, and the region is reported once, not once per revision.
    #[test]
    fn tolerate_marker_masks_a_flip_across_a_moved_region() {
        let (_dir, repo, base, head_lcov, base_lcov) =
            repo_with_moved_marker_region(Some("tolerate"));
        let mut cmd = command(head_lcov, &base);
        cmd.baseline_report = Some(base_lcov);
        let rendered = cmd.run(Some(&repo)).unwrap().rendered;

        assert!(
            rendered.contains("Total: **33.33%**"),
            "the displayed percentage must be the real one: {rendered}"
        );
        assert!(
            rendered.contains("\u{26aa} 0 pp vs `main`"),
            "the masked flip must not move the headline: {rendered}"
        );
        assert!(
            !rendered.contains("\u{1f534}"),
            "nothing should be painted red: {rendered}"
        );
        assert!(
            rendered.contains("0 ignored region(s), 1 tolerated region(s)"),
            "a region that merely moved is one region, not two: {rendered}"
        );
        assert!(
            rendered.contains("| `src/gated.rs` | `tolerate` | 3-6 | both |"),
            "the span reported is the one observed at head: {rendered}"
        );
        assert!(
            rendered.contains("CPU-gated dispatch"),
            "the reason must be shown: {rendered}"
        );
    }

    /// The control that proves the test above measures the marker and not a
    /// coincidence: the same flip, unmarked, still moves the report.
    #[test]
    fn the_same_flip_moves_the_report_without_a_marker() {
        let (_dir, repo, base, head_lcov, base_lcov) = repo_with_moved_marker_region(None);
        let mut cmd = command(head_lcov, &base);
        cmd.baseline_report = Some(base_lcov);
        let rendered = cmd.run(Some(&repo)).unwrap().rendered;
        assert!(
            rendered.contains("\u{1f534}"),
            "an unmarked flip must still be reported: {rendered}"
        );
        assert!(!rendered.contains("region(s)"), "{rendered}");
    }

    /// `ignore` removes the lines from both reports, so they leave the
    /// denominator entirely rather than being scored against the baseline.
    #[test]
    fn ignore_marker_removes_lines_from_both_reports() {
        let (_dir, repo, base, head_lcov, base_lcov) =
            repo_with_moved_marker_region(Some("ignore"));
        let mut cmd = command(head_lcov, &base);
        cmd.baseline_report = Some(base_lcov);
        let rendered = cmd.run(Some(&repo)).unwrap().rendered;
        assert!(
            rendered.contains("Total: **100%**"),
            "only the covered `plain` line should remain in the denominator: {rendered}"
        );
        assert!(rendered.contains("1 ignored region(s)"), "{rendered}");
    }

    /// A malformed marker fails the run loudly, naming the file and line: a
    /// marker whose author believes it is silencing noise must never be a silent
    /// no-op.
    #[test]
    fn a_malformed_marker_is_a_hard_error() {
        let (_dir, repo, base, head_lcov, base_lcov) =
            repo_with_moved_marker_region(Some("tolerate"));
        // Strip the closing marker from the head worktree, leaving it open.
        let gated = repo.join("src/gated.rs");
        let source = fs::read_to_string(&gated).unwrap();
        let end = format!("// {} end", crate::markers::INTRODUCER);
        fs::write(&gated, source.replace(&end, "// not the end")).unwrap();

        let mut cmd = command(head_lcov, &base);
        cmd.baseline_report = Some(base_lcov);
        let message = match cmd.run(Some(&repo)) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("an unterminated region must fail the run"),
        };
        assert!(message.contains("src/gated.rs:3"), "{message}");
        assert!(message.contains("unterminated"), "{message}");
    }

    #[test]
    fn run_with_baseline_enables_delta() {
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        // Baseline only knows a.rs at 100%.
        let baseline = repo.join("base.lcov");
        fs::write(
            &baseline,
            format!("SF:{}/a.rs\nDA:1,1\nend_of_record\n", repo.display()),
        )
        .unwrap();
        let mut cmd = command(report, &base);
        cmd.baseline_report = Some(baseline);
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert!(outcome.rendered.contains("vs `main`"));
    }

    #[test]
    fn fail_under_patch_gate() {
        let (_dir, repo, base) = repo_with_added_file();
        // Patch coverage is ~66.7%.
        let report = write_head_lcov(&repo);
        let mut cmd = command(report, &base);
        cmd.fail_under_patch = Some(90.0);
        assert!(
            cmd.run(Some(&repo)).unwrap().below_gate,
            "66.7% < 90% should fail"
        );

        let report = write_head_lcov(&repo);
        let mut cmd = command(report, &base);
        cmd.fail_under_patch = Some(50.0);
        assert!(
            !cmd.run(Some(&repo)).unwrap().below_gate,
            "66.7% >= 50% should pass"
        );
    }

    /// Writes a head report covering `b.rs` (2 of 3 lines) and `a.rs` (0 of 2),
    /// so overall line coverage is 2/5 = 40% and 2/3 once `a.rs` is excluded.
    fn write_two_file_head_lcov(repo: &Path) -> PathBuf {
        let lcov = format!(
            "SF:{a}\nDA:1,0\nDA:2,0\nend_of_record\nSF:{b}\nDA:1,1\nDA:2,0\nDA:3,4\nend_of_record\n",
            a = repo.join("a.rs").display(),
            b = repo.join("b.rs").display(),
        );
        let report = repo.join("head.lcov");
        fs::write(&report, lcov).unwrap();
        report
    }

    #[test]
    fn fail_under_lines_gate() {
        let (_dir, repo, base) = repo_with_added_file();
        // Overall line coverage is 40%.
        let report = write_two_file_head_lcov(&repo);
        let mut cmd = command(report.clone(), &base);
        cmd.fail_under_lines = Some(50.0);
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert!(outcome.below_line_gate, "40% < 50% should fail");
        assert_eq!(outcome.line_percent, Some(40.0));
        assert!(!outcome.below_gate, "the patch gate is independent");

        let mut cmd = command(report, &base);
        cmd.fail_under_lines = Some(40.0);
        assert!(
            !cmd.run(Some(&repo)).unwrap().below_line_gate,
            "a total exactly at the threshold passes"
        );
    }

    #[test]
    fn line_gate_is_off_unless_requested() {
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_two_file_head_lcov(&repo);
        let outcome = command(report, &base).run(Some(&repo)).unwrap();
        assert_eq!(outcome.line_percent, Some(40.0));
        assert!(!outcome.below_line_gate);
    }

    #[test]
    fn fail_under_lines_sees_the_post_ignore_total() {
        // The gate reads the same total the report prints, so excluding the
        // uncovered `a.rs` lifts 40% to 66.7% and turns a failing gate into a pass.
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_two_file_head_lcov(&repo);
        let mut cmd = command(report, &base);
        cmd.fail_under_lines = Some(50.0);
        cmd.ignore_filename_regex = vec![r"a\.rs".to_string()];
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.line_percent, Some(2.0 / 3.0 * 100.0));
        assert!(!outcome.below_line_gate);
    }

    #[test]
    fn fail_under_lines_fails_when_nothing_is_measurable() {
        // Excluding every file leaves no executable lines. Passing vacuously
        // would let an empty (or fully excluded) report through the gate.
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        let mut cmd = command(report, &base);
        cmd.fail_under_lines = Some(1.0);
        cmd.ignore_filename_regex = vec![r"b\.rs".to_string()];
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.line_percent, None);
        assert!(outcome.below_line_gate);
        let failures = cmd.gate_failures(&outcome);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("no executable lines"), "{failures:?}");
    }

    #[test]
    fn gate_failures_reports_every_failed_gate() {
        let (_dir, repo, base) = repo_with_added_file();
        // Patch coverage is 66.7% and overall line coverage is 40%.
        let report = write_two_file_head_lcov(&repo);
        let mut cmd = command(report, &base);
        cmd.fail_under_patch = Some(90.0);
        cmd.fail_under_lines = Some(90.0);
        let outcome = cmd.run(Some(&repo)).unwrap();
        let failures = cmd.gate_failures(&outcome);
        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(
            failures[0].contains("patch coverage 66.67%"),
            "{failures:?}"
        );
        assert!(failures[1].contains("line coverage 40.00%"), "{failures:?}");
        assert!(failures[1].contains("--fail-under-lines threshold of 90.00%"));
    }

    #[test]
    fn execute_bails_on_the_line_gate_only() {
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_two_file_head_lcov(&repo);
        let mut cmd = command(report, &base);
        cmd.fail_under_lines = Some(90.0);
        let message = cmd.execute(Some(&repo)).unwrap_err().to_string();
        assert!(message.contains("--fail-under-lines"), "{message}");
        assert!(!message.contains("--fail-under-patch"), "{message}");
    }

    #[test]
    fn fail_under_lines_parses() {
        use clap::Parser;
        let cmd =
            DiffCommand::try_parse_from(["diff", "--report", "r.lcov", "--fail-under-lines", "80"])
                .unwrap();
        assert_eq!(cmd.fail_under_lines, Some(80.0));
        assert_eq!(cmd.fail_under_patch, None);
    }

    // ── sharded reports ──────────────────────────────────────────

    /// Writes an lcov shard under `repo`: `a.rs` covered at line 1, and `b.rs`
    /// with the given `(line, hits)` records.
    fn write_shard(repo: &Path, name: &str, b_lines: &[(u32, u64)]) -> PathBuf {
        use std::fmt::Write as _;
        let mut lcov = format!(
            "SF:{}\nDA:1,1\nend_of_record\nSF:{}\n",
            repo.join("a.rs").display(),
            repo.join("b.rs").display()
        );
        for (line, hits) in b_lines {
            let _ = writeln!(lcov, "DA:{line},{hits}");
        }
        lcov.push_str("end_of_record\n");
        let path = repo.join(name);
        fs::write(&path, lcov).unwrap();
        path
    }

    /// A command reading several shard reports.
    fn sharded(reports: Vec<PathBuf>, base: &str) -> DiffCommand {
        let mut cmd = command(PathBuf::new(), base);
        cmd.report = reports;
        cmd
    }

    #[test]
    fn shards_are_merged_before_the_patch_is_computed() {
        // The shards cover different lines of `b.rs`: each alone leaves two of
        // its three added lines uncovered, but together only line 3 is.
        let (_dir, repo, base) = repo_with_added_file();
        let one = write_shard(&repo, "one.lcov", &[(1, 1), (2, 0), (3, 0)]);
        let two = write_shard(&repo, "two.lcov", &[(1, 0), (2, 1), (3, 0)]);

        for alone in [&one, &two] {
            let outcome = sharded(vec![alone.clone()], &base)
                .run(Some(&repo))
                .unwrap();
            assert_eq!(outcome.patch_percent, Some(1.0 / 3.0 * 100.0));
        }
        let outcome = sharded(vec![one, two], &base).run(Some(&repo)).unwrap();
        assert_eq!(outcome.patch_percent, Some(2.0 / 3.0 * 100.0));
        // `a.rs` line 1, and `b.rs` lines 1 and 2, are covered; `b.rs` line 3 is not.
        assert_eq!(outcome.line_percent, Some(75.0));
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    }

    #[test]
    fn shard_order_does_not_change_the_output() {
        let (_dir, repo, base) = repo_with_added_file();
        let one = write_shard(&repo, "one.lcov", &[(1, 1), (2, 0), (3, 0)]);
        let two = write_shard(&repo, "two.lcov", &[(1, 0), (2, 1), (3, 0)]);
        let forward = sharded(vec![one.clone(), two.clone()], &base)
            .run(Some(&repo))
            .unwrap();
        let reverse = sharded(vec![two, one], &base).run(Some(&repo)).unwrap();
        assert_eq!(forward.rendered, reverse.rendered);
        assert_eq!(forward.line_percent, reverse.line_percent);
    }

    #[test]
    fn shards_of_different_formats_are_merged() {
        // An lcov shard and an llvm-cov JSON shard, each auto-detected. The JSON
        // region is uncovered over `b.rs` lines 2-4 (line 4 exists only there).
        // Merged: line 2 stays covered (max of 1 and 0), line 4 is added
        // uncovered, so 3 of 5 lines are covered — a figure neither shard alone
        // (3/4 and 0/3) nor a "last shard wins" replacement (1/4) would give.
        let (_dir, repo, base) = repo_with_added_file();
        let lcov = write_shard(&repo, "one.lcov", &[(1, 1), (2, 1), (3, 0)]);
        let json = repo.join("two.json");
        fs::write(
            &json,
            serde_json::json!({
                "data": [{ "files": [{
                    "filename": repo.join("b.rs").display().to_string(),
                    "segments": [
                        [2, 1, 0, true, true, false],
                        [4, 1, 0, false, false, false]
                    ]
                }]}],
                "type": "llvm.coverage.json.export",
                "version": "2.0.1"
            })
            .to_string(),
        )
        .unwrap();
        let outcome = sharded(vec![lcov, json], &base).run(Some(&repo)).unwrap();
        assert_eq!(outcome.line_percent, Some(60.0));
        assert_eq!(outcome.patch_percent, Some(2.0 / 3.0 * 100.0));
    }

    #[test]
    fn a_shard_with_no_executable_lines_fails_the_run_by_name() {
        let (_dir, repo, base) = repo_with_added_file();
        let one = write_shard(&repo, "one.lcov", &[(1, 1), (2, 1), (3, 1)]);
        // A parseable lcov whose blocks list no lines: the shape of a failed run.
        let two = repo.join("two.lcov");
        fs::write(&two, "TN:\nSF:b.rs\nend_of_record\n").unwrap();
        let message = sharded(vec![one, two], &base)
            .run(Some(&repo))
            .err()
            .expect("an empty shard must fail the run")
            .to_string();
        assert!(message.contains("two.lcov"), "{message}");
        assert!(message.contains("no executable lines"), "{message}");
    }

    #[test]
    fn an_unparseable_shard_fails_the_run_by_name() {
        let (_dir, repo, base) = repo_with_added_file();
        let one = write_shard(&repo, "one.lcov", &[(1, 1), (2, 1), (3, 1)]);
        let two = repo.join("two.lcov");
        fs::write(&two, "").unwrap();
        let message = format!(
            "{:#}",
            sharded(vec![one, two], &base)
                .run(Some(&repo))
                .err()
                .expect("an empty file must fail the run")
        );
        assert!(message.contains("two.lcov"), "{message}");
    }

    #[test]
    fn a_missing_shard_fails_the_run() {
        let (_dir, repo, base) = repo_with_added_file();
        let one = write_shard(&repo, "one.lcov", &[(1, 1), (2, 1), (3, 1)]);
        let message = sharded(vec![one, repo.join("shard-2.lcov")], &base)
            .run(Some(&repo))
            .err()
            .expect("a shard that never uploaded must fail the run")
            .to_string();
        assert!(message.contains("shard-2.lcov"), "{message}");
    }

    #[test]
    fn a_lone_report_with_no_lines_is_still_accepted() {
        // The empty-shard check applies to a *set* of shards. A single report
        // keeps its pre-sharding behaviour (`--fail-under-lines` is what rejects
        // an empty one), so no existing invocation starts failing.
        let (_dir, repo, base) = repo_with_added_file();
        let empty = repo.join("empty.lcov");
        fs::write(&empty, "TN:\nSF:b.rs\nend_of_record\n").unwrap();
        let outcome = sharded(vec![empty], &base).run(Some(&repo)).unwrap();
        assert_eq!(outcome.line_percent, None);
    }

    #[test]
    fn a_shard_whose_files_are_all_ignored_is_not_empty() {
        // The check runs on the parsed shard, before `--ignore-filename-regex`:
        // a shard that only covers excluded files is healthy, not failed.
        let (_dir, repo, base) = repo_with_added_file();
        let one = write_shard(&repo, "one.lcov", &[(1, 1), (2, 1), (3, 1)]);
        let two = repo.join("two.lcov");
        fs::write(
            &two,
            format!(
                "SF:{}\nDA:1,1\nend_of_record\n",
                repo.join("a.rs").display()
            ),
        )
        .unwrap();
        let mut cmd = sharded(vec![one, two], &base);
        cmd.ignore_filename_regex = vec![r"a\.rs".to_string()];
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.line_percent, Some(100.0));
    }

    #[test]
    fn a_shard_under_another_root_fails_unless_allowed() {
        let (_dir, repo, base) = repo_with_added_file();
        let one = write_shard(&repo, "one.lcov", &[(1, 1), (2, 1), (3, 1)]);
        let two = repo.join("two.lcov");
        fs::write(
            &two,
            "SF:/some/other/runner/b.rs\nDA:1,1\nDA:2,1\nend_of_record\n",
        )
        .unwrap();
        let mut cmd = sharded(vec![one, two], &base);
        let message = cmd.run(Some(&repo)).err().unwrap().to_string();
        assert!(message.contains("two.lcov"), "{message}");
        assert!(message.contains("--strip-prefix"), "{message}");
        cmd.allow_path_mismatch = true;
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.warnings.len(), 1, "{:?}", outcome.warnings);
        assert!(outcome.warnings[0].contains("two.lcov"));
        assert!(outcome.warnings[0].contains("--strip-prefix"));
    }

    #[test]
    fn execute_reports_allowed_shard_warnings_without_failing() {
        let (_dir, repo, base) = repo_with_added_file();
        let one = write_shard(&repo, "one.lcov", &[(1, 1), (2, 1), (3, 1)]);
        let two = repo.join("two.lcov");
        fs::write(&two, "SF:/some/other/runner/b.rs\nDA:1,1\nend_of_record\n").unwrap();
        let mut cmd = sharded(vec![one, two], &base);
        assert!(cmd.run(Some(&repo)).is_err());
        // With the opt-out the warning goes to stderr and the run still succeeds.
        cmd.allow_path_mismatch = true;
        cmd.execute(Some(&repo)).unwrap();
    }

    #[test]
    fn a_lone_report_under_another_root_fails_unless_allowed() {
        // A single report can also silently miss every patch line.
        let (_dir, repo, base) = repo_with_added_file();
        let only = repo.join("only.lcov");
        fs::write(&only, "SF:/some/other/runner/b.rs\nDA:1,1\nend_of_record\n").unwrap();
        let mut cmd = sharded(vec![only], &base);
        assert!(cmd.run(Some(&repo)).is_err());
        cmd.allow_path_mismatch = true;
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("only.lcov"));
    }

    #[test]
    fn an_empty_report_list_is_an_error() {
        // clap requires one `--report`; a programmatic caller (MCP) is checked here.
        let (_dir, repo, base) = repo_with_added_file();
        let message = sharded(Vec::new(), &base)
            .run(Some(&repo))
            .err()
            .expect("no report at all must fail")
            .to_string();
        assert!(
            message.contains("at least one coverage report"),
            "{message}"
        );
    }

    #[test]
    fn report_is_repeatable_and_still_required() {
        use clap::Parser;
        let cmd = DiffCommand::try_parse_from([
            "diff",
            "--report",
            "one.lcov",
            "--report",
            "two.lcov",
            "--report",
            "three.json",
        ])
        .unwrap();
        assert_eq!(
            cmd.report,
            vec![
                PathBuf::from("one.lcov"),
                PathBuf::from("two.lcov"),
                PathBuf::from("three.json")
            ]
        );
        assert!(DiffCommand::try_parse_from(["diff"]).is_err());
    }

    #[test]
    fn all_files_scope_surfaces_untouched_file() {
        // `a.rs` is unchanged between base and head, so the diff never touches
        // it. Its coverage moves by a single line (2/4 → 3/4): a small enough
        // net move that DiffOnly drops it as noise, but a 25 pp shift that the
        // delta table renders once `--all-files` widens the scope to `All`.
        let (_dir, repo, base) = repo_with_added_file();
        let head = format!(
            "SF:{a}\nDA:1,1\nDA:2,1\nDA:3,1\nDA:4,0\nend_of_record\n\
             SF:{b}\nDA:1,1\nDA:2,0\nDA:3,4\nend_of_record\n",
            a = repo.join("a.rs").display(),
            b = repo.join("b.rs").display(),
        );
        let report = repo.join("head.lcov");
        fs::write(&report, head).unwrap();
        let baseline = repo.join("base.lcov");
        fs::write(
            &baseline,
            format!(
                "SF:{}\nDA:1,1\nDA:2,1\nDA:3,0\nDA:4,0\nend_of_record\n",
                repo.join("a.rs").display()
            ),
        )
        .unwrap();

        // Default (DiffOnly): the untouched `a.rs` row is filtered out as noise.
        let mut scoped = command(report.clone(), &base);
        scoped.baseline_report = Some(baseline.clone());
        let scoped_md = scoped.run(Some(&repo)).unwrap().rendered;
        assert!(
            !scoped_md.contains("`a.rs`"),
            "DiffOnly must hide untouched a.rs"
        );

        // --all-files (All): the untouched `a.rs` row is now surfaced.
        let mut all = command(report, &base);
        all.baseline_report = Some(baseline);
        all.all_files = true;
        let all_md = all.run(Some(&repo)).unwrap().rendered;
        assert!(
            all_md.contains("`a.rs`"),
            "All scope must surface untouched a.rs"
        );
    }

    #[test]
    fn ignore_filename_regex_excludes_file_from_patch() {
        // The only diff-added file is `b.rs`; ignoring it removes it from the
        // head report, so `head.hits` misses and no added lines remain to
        // measure — patch coverage becomes undefined rather than 66.7%.
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        let mut cmd = command(report, &base);
        cmd.ignore_filename_regex = vec![r"b\.rs".to_string()];
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.patch_percent, None);
        assert!(!outcome.rendered.contains("`b.rs:2`"));
    }

    #[test]
    fn excluding_the_only_touched_file_is_visible_and_not_an_empty_diff() {
        // The diff's one file is excluded. The comment must say so, and
        // must not read like a diff that added no code.
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        let mut cmd = command(report, &base);
        cmd.ignore_filename_regex = vec![r"b\.rs".to_string()];
        let md = cmd.run(Some(&repo)).unwrap().rendered;
        assert!(
            md.contains(
                "_Excluded by ignore-filename-regex: 1 file (1 of them touched by this diff, adding 3 executable lines)._"
            ),
            "{md}"
        );
        assert!(md.contains("- `b.rs`"), "{md}");
        assert!(
            md.contains("3 new executable lines are in files excluded by ignore-filename-regex"),
            "{md}"
        );
        assert!(!md.contains("_No new executable lines added by this diff._"));

        cmd.output = OutputFormatArg::Json;
        let json: serde_json::Value =
            serde_json::from_str(&cmd.run(Some(&repo)).unwrap().rendered).unwrap();
        let excluded = &json["excluded_files"];
        assert_eq!(excluded["count"], 1);
        assert_eq!(excluded["touched_count"], 1);
        assert_eq!(excluded["new_executable_lines"], 3);
        assert_eq!(excluded["paths"], serde_json::json!(["b.rs"]));
        assert_eq!(excluded["touched"], serde_json::json!(["b.rs"]));
    }

    #[test]
    fn a_filter_matching_no_file_leaves_the_output_unchanged() {
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        let plain = command(report.clone(), &base)
            .run(Some(&repo))
            .unwrap()
            .rendered;
        let mut cmd = command(report, &base);
        cmd.ignore_filename_regex = vec![r"nomatch\.rs".to_string()];
        let filtered = cmd.run(Some(&repo)).unwrap().rendered;
        assert_eq!(filtered, plain);
        assert!(!filtered.contains("Excluded by"));
        for format in [OutputFormatArg::Json, OutputFormatArg::Yaml] {
            cmd.output = format;
            assert!(!cmd
                .run(Some(&repo))
                .unwrap()
                .rendered
                .contains("excluded_files:"));
            assert!(!cmd
                .run(Some(&repo))
                .unwrap()
                .rendered
                .contains("\"excluded_files\""));
        }
    }

    #[test]
    fn excluding_an_untouched_file_says_the_diff_did_not_touch_it() {
        // `a.rs` is in the report but the diff only adds `b.rs`: the note counts
        // it, says nothing touched it, and the patch is measured as before.
        let (_dir, repo, base) = repo_with_added_file();
        let head = format!(
            "SF:{a}\nDA:1,1\nend_of_record\nSF:{b}\nDA:1,1\nDA:2,0\nDA:3,4\nend_of_record\n",
            a = repo.join("a.rs").display(),
            b = repo.join("b.rs").display(),
        );
        let report = repo.join("head.lcov");
        fs::write(&report, head).unwrap();
        let mut cmd = command(report, &base);
        cmd.ignore_filename_regex = vec![r"a\.rs".to_string()];
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert!(
            outcome.rendered.contains(
                "_Excluded by ignore-filename-regex: 1 file (none of them touched by this diff)._"
            ),
            "{}",
            outcome.rendered
        );
        assert!(!outcome.rendered.contains("Excluded files touched"));
        assert!(outcome.patch_percent.is_some());
    }

    #[test]
    fn a_diff_that_added_no_executable_code_reads_as_before_beside_the_note() {
        // The diff's file is not in the report at all, so the filter took no new
        // executable line out of the patch: the empty-patch sentence is unchanged.
        let (_dir, repo, base) = repo_with_added_file();
        let report = repo.join("head.lcov");
        fs::write(
            &report,
            format!(
                "SF:{}\nDA:1,1\nend_of_record\n",
                repo.join("a.rs").display()
            ),
        )
        .unwrap();
        let mut cmd = command(report, &base);
        cmd.ignore_filename_regex = vec![r"a\.rs".to_string()];
        let md = cmd.run(Some(&repo)).unwrap().rendered;
        assert!(
            md.contains("_No new executable lines added by this diff._"),
            "{md}"
        );
        assert!(
            md.contains("Excluded by ignore-filename-regex: 1 file"),
            "{md}"
        );
    }

    #[test]
    fn a_file_only_the_baseline_held_counts_as_excluded() {
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        let baseline = repo.join("base.lcov");
        fs::write(
            &baseline,
            format!(
                "SF:{}\nDA:1,1\nend_of_record\n",
                repo.join("a.rs").display()
            ),
        )
        .unwrap();
        let mut cmd = command(report, &base);
        cmd.baseline_report = Some(baseline);
        cmd.ignore_filename_regex = vec![r"a\.rs".to_string()];
        let md = cmd.run(Some(&repo)).unwrap().rendered;
        assert!(
            md.contains(
                "_Excluded by ignore-filename-regex: 1 file (none of them touched by this diff)._"
            ),
            "{md}"
        );
    }

    #[test]
    fn ignore_filename_regex_drops_file_from_both_reports() {
        // `a.rs` is unchanged but has different coverage in head vs baseline, so
        // `--all-files` would normally surface it in the delta table (it is read
        // from both `head.files` and `baseline.files`). The regex must remove it
        // from both sides symmetrically so it disappears entirely.
        let (_dir, repo, base) = repo_with_added_file();
        let head = format!(
            "SF:{a}\nDA:1,1\nDA:2,1\nDA:3,1\nDA:4,0\nend_of_record\n\
             SF:{b}\nDA:1,1\nDA:2,0\nDA:3,4\nend_of_record\n",
            a = repo.join("a.rs").display(),
            b = repo.join("b.rs").display(),
        );
        let report = repo.join("head.lcov");
        fs::write(&report, head).unwrap();
        let baseline = repo.join("base.lcov");
        fs::write(
            &baseline,
            format!(
                "SF:{}\nDA:1,1\nDA:2,1\nDA:3,0\nDA:4,0\nend_of_record\n",
                repo.join("a.rs").display()
            ),
        )
        .unwrap();

        let mut cmd = command(report, &base);
        cmd.baseline_report = Some(baseline);
        cmd.all_files = true;
        cmd.ignore_filename_regex = vec![r"a\.rs".to_string()];
        let md = cmd.run(Some(&repo)).unwrap().rendered;
        assert!(
            !md.contains("`a.rs`"),
            "ignored a.rs must not appear in the delta table"
        );
        // The un-ignored added file is still reported.
        assert!(md.contains("`b.rs"));
    }

    #[test]
    fn ignore_filename_regex_parses_repeatable_and_comma() {
        use clap::Parser;
        let cmd = DiffCommand::try_parse_from([
            "diff",
            "--report",
            "r.lcov",
            "--ignore-filename-regex",
            "foo,bar",
            "--ignore-filename-regex",
            "baz",
        ])
        .unwrap();
        assert_eq!(cmd.ignore_filename_regex, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn ignore_filename_regex_treats_empty_pattern_as_noop() {
        // A trailing/doubled comma splits to an empty element; an empty regex
        // matches every path and would silently wipe the whole report (and pass
        // a --fail-under-patch gate vacuously). It must be dropped instead.
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        let mut cmd = command(report, &base);
        cmd.ignore_filename_regex = vec![String::new(), r"nomatch\.rs".to_string()];
        let outcome = cmd.run(Some(&repo)).unwrap();
        // b.rs is still measured: 2/3, not wiped to nothing.
        assert_eq!(outcome.patch_percent, Some(2.0 / 3.0 * 100.0));
        assert!(outcome.rendered.contains("`b.rs:2`"));
    }

    #[test]
    fn invalid_ignore_pattern_errors() {
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        let mut cmd = command(report, &base);
        cmd.ignore_filename_regex = vec!["(unclosed".to_string()];
        assert!(cmd.run(Some(&repo)).is_err());
    }

    // ── .patchcov/config.yaml ignore-list ──────────────────────

    #[test]
    fn path_mappings_apply_to_shards_baseline_filters_and_markers() {
        let (_dir, repo, base) = repo_with_added_file();
        let config_dir = repo.join(".patchcov");
        fs::create_dir(&config_dir).unwrap();
        fs::write(config_dir.join("config.yaml"),
            "diff:\n  path-mappings:\n    - from: /ci/package\n      to: ''\n    - from: module\n      to: ''\n").unwrap();
        let report = repo.join("head.lcov");
        fs::write(
            &report,
            "SF:/ci/package/b.rs\nDA:1,1\nDA:2,0\nDA:3,1\nend_of_record\nSF:/sdk/runtime.rs\nDA:1,1\nend_of_record\nSF:module/a.rs\nDA:1,0\nend_of_record\n",
        )
        .unwrap();
        let shard = repo.join("shard.lcov");
        fs::write(&shard, "SF:module/b.rs\nDA:2,1\nend_of_record\n").unwrap();
        let baseline = repo.join("base.lcov");
        fs::write(&baseline, "SF:module/a.rs\nDA:1,1\nend_of_record\n").unwrap();
        let mut cmd = command(report, &base);
        cmd.config_dir = Some(config_dir);
        cmd.report.push(shard);
        cmd.baseline_report = Some(baseline);
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.patch_percent, Some(100.0));
        assert!(outcome.warnings.is_empty());

        cmd.all_files = true;
        cmd.output = OutputFormatArg::Json;
        let outcome = cmd.run(Some(&repo)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&outcome.rendered).unwrap();
        let deltas = json["project_delta"]["files"].as_array().unwrap();
        assert!(deltas
            .iter()
            .any(|d| d["path"] == "a.rs" && d["before"] == 100.0));
        cmd.output = OutputFormatArg::Markdown;
        cmd.ignore_filename_regex = vec!["^b\\.rs$".into()];
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.patch_percent, None);
        assert!(outcome
            .rendered
            .contains("Excluded by ignore-filename-regex"));
        cmd.ignore_filename_regex.clear();
        // A malformed marker proves the mapped path is used to read source.
        fs::write(
            repo.join("b.rs"),
            format!("// {} coverage ignore-line\none\n", "patchcov:"),
        )
        .unwrap();
        let error = cmd.run(Some(&repo)).err().unwrap().to_string();
        assert!(error.contains("b.rs"), "{error}");
    }

    #[test]
    fn invalid_path_mapping_config_fails_before_report_loading() {
        let (_dir, repo, base) = repo_with_added_file();
        let dir = repo.join(".patchcov");
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("config.yaml"),
            "diff:\n  path-mappings:\n    - from: src\n      to: ../outside\n",
        )
        .unwrap();
        let mut cmd = command(repo.join("missing.lcov"), &base);
        cmd.config_dir = Some(dir);
        assert!(cmd
            .run(Some(&repo))
            .err()
            .unwrap()
            .to_string()
            .contains("repo-relative"));
    }

    #[test]
    fn coverage_config_parses_ignore_list() {
        let yaml = "diff:\n  ignore-filename-regex:\n    - 'src/bits/popcount\\.rs'\n    - 'src/dsv/simd/.*'\n";
        let config: CoverageConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.diff.ignore_filename_regex,
            vec![
                r"src/bits/popcount\.rs".to_string(),
                "src/dsv/simd/.*".to_string()
            ]
        );
    }

    #[test]
    fn coverage_config_ignores_unknown_keys() {
        // Forward-compat: a newer top-level or `diff` key is ignored, not an
        // error, so a newer schema stays readable by an older binary.
        let yaml =
            "future-top: 1\ndiff:\n  future-diff-key: true\n  ignore-filename-regex:\n    - 'x'\n";
        let config: CoverageConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.diff.ignore_filename_regex, vec!["x".to_string()]);
    }

    #[test]
    fn coverage_config_defaults_to_empty() {
        let config: CoverageConfig = serde_yaml::from_str("{}").unwrap();
        assert!(config.diff.ignore_filename_regex.is_empty());
        assert!(config.lint_markers.include.is_empty());
    }

    #[test]
    fn coverage_config_parses_lint_markers_include() {
        let yaml =
            "lint-markers:\n  future-key: 1\n  include:\n    - 'src/**/*.py'\n    - '**/*.rs'\n";
        let config: CoverageConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.lint_markers.include,
            vec!["src/**/*.py".to_string(), "**/*.rs".to_string()]
        );
        assert!(config.diff.ignore_filename_regex.is_empty());
    }

    #[test]
    fn config_ignore_drops_file_from_both_reports() {
        // Acceptance criterion: a file matched ONLY via `.patchcov/config.yaml`
        // is excluded from BOTH head and baseline, so it cannot surface in the
        // delta table nor the "unchanged files also moved" note. Config-only
        // analogue of `ignore_filename_regex_drops_file_from_both_reports`.
        let (_dir, repo, base) = repo_with_added_file();
        let head = format!(
            "SF:{a}\nDA:1,1\nDA:2,1\nDA:3,1\nDA:4,0\nend_of_record\n\
             SF:{b}\nDA:1,1\nDA:2,0\nDA:3,4\nend_of_record\n",
            a = repo.join("a.rs").display(),
            b = repo.join("b.rs").display(),
        );
        let report = repo.join("head.lcov");
        fs::write(&report, head).unwrap();
        let baseline = repo.join("base.lcov");
        fs::write(
            &baseline,
            format!(
                "SF:{}\nDA:1,1\nDA:2,1\nDA:3,0\nDA:4,0\nend_of_record\n",
                repo.join("a.rs").display()
            ),
        )
        .unwrap();

        let config_dir = write_coverage_config(&repo, &[r"a\.rs"]);
        let mut cmd = command(report, &base);
        cmd.baseline_report = Some(baseline);
        cmd.all_files = true;
        cmd.config_dir = Some(config_dir);
        let md = cmd.run(Some(&repo)).unwrap().rendered;
        assert!(
            !md.contains("`a.rs`"),
            "config-ignored a.rs must not appear in the delta table"
        );
        assert!(md.contains("`b.rs"), "un-ignored added file still reported");
    }

    #[test]
    fn config_and_cli_ignore_lists_union() {
        // The config ignores `a.rs`; the CLI flag ignores `b.rs`. Both take
        // effect — proving the two sources are set-unioned, neither replacing
        // the other.
        let (_dir, repo, base) = repo_with_added_file();
        let head = format!(
            "SF:{a}\nDA:1,1\nDA:2,1\nDA:3,1\nDA:4,0\nend_of_record\n\
             SF:{b}\nDA:1,1\nDA:2,0\nDA:3,4\nend_of_record\n",
            a = repo.join("a.rs").display(),
            b = repo.join("b.rs").display(),
        );
        let report = repo.join("head.lcov");
        fs::write(&report, head).unwrap();
        let baseline = repo.join("base.lcov");
        fs::write(
            &baseline,
            format!(
                "SF:{}\nDA:1,1\nDA:2,1\nDA:3,0\nDA:4,0\nend_of_record\n",
                repo.join("a.rs").display()
            ),
        )
        .unwrap();

        let config_dir = write_coverage_config(&repo, &[r"a\.rs"]);
        let mut cmd = command(report, &base);
        cmd.baseline_report = Some(baseline);
        cmd.all_files = true;
        cmd.config_dir = Some(config_dir);
        cmd.ignore_filename_regex = vec![r"b\.rs".to_string()];
        let outcome = cmd.run(Some(&repo)).unwrap();
        // `b.rs` (the only added file) dropped via the CLI flag ⇒ no patch lines.
        assert_eq!(outcome.patch_percent, None);
        // `a.rs` dropped via config ⇒ absent from the delta table.
        assert!(!outcome.rendered.contains("`a.rs`"));
    }

    #[test]
    fn empty_config_ignore_is_noop() {
        // An empty `diff.ignore-filename-regex: []` changes nothing: b.rs is
        // still measured at 2/3, exactly as with no config at all.
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        let config_dir = write_coverage_config(&repo, &[]);
        let mut cmd = command(report, &base);
        cmd.config_dir = Some(config_dir);
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.patch_percent, Some(2.0 / 3.0 * 100.0));
        assert!(outcome.rendered.contains("`b.rs:2`"));
    }

    #[test]
    fn missing_config_is_noop() {
        // A config dir with no `config.yaml` leaves behavior unchanged.
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        let empty_dir = repo.join(".patchcov-empty");
        fs::create_dir_all(&empty_dir).unwrap();
        let mut cmd = command(report, &base);
        cmd.config_dir = Some(empty_dir);
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.patch_percent, Some(2.0 / 3.0 * 100.0));
        assert!(outcome.rendered.contains("`b.rs:2`"));
    }

    #[test]
    fn malformed_config_errors() {
        // A present-but-malformed `config.yaml` (scalar where a list is
        // expected) is a hard error — fail loudly rather than silently letting
        // the excluded noise back in.
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        let config_dir = repo.join(".patchcov");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.yaml"),
            "diff:\n  ignore-filename-regex: 'not-a-list'\n",
        )
        .unwrap();
        let mut cmd = command(report, &base);
        cmd.config_dir = Some(config_dir);
        assert!(cmd.run(Some(&repo)).is_err());
    }

    #[test]
    fn invalid_config_pattern_errors() {
        // An unclosed group from config is an up-front error, same as the flag.
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        let config_dir = write_coverage_config(&repo, &["(unclosed"]);
        let mut cmd = command(report, &base);
        cmd.config_dir = Some(config_dir);
        assert!(cmd.run(Some(&repo)).is_err());
    }

    #[test]
    fn path_mismatch_checks_each_report_before_exclusions() {
        let (_dir, repo, base) = repo_with_added_file();
        let bad = repo.join("bad.lcov");
        fs::write(&bad, "SF:wrong/b.rs\nDA:1,1\nend_of_record\n").unwrap();
        let mut cmd = command(bad.clone(), &base);
        cmd.ignore_filename_regex = vec![".*".into()];
        assert!(cmd.run(Some(&repo)).is_err());
        cmd.allow_path_mismatch = true;
        assert_eq!(cmd.run(Some(&repo)).unwrap().warnings.len(), 1);
        cmd.allow_path_mismatch = false;
        cmd.ignore_filename_regex.clear();
        cmd.report = vec![write_head_lcov(&repo), bad.clone()];
        assert!(cmd.run(Some(&repo)).is_err());
        cmd.report.pop();
        cmd.baseline_report = Some(bad);
        assert!(cmd
            .run(Some(&repo))
            .err()
            .unwrap()
            .to_string()
            .contains("bad.lcov"));
    }

    #[test]
    fn path_mismatch_error_names_the_opt_out() {
        let (_dir, repo, base) = repo_with_added_file();
        let bad = repo.join("bad.lcov");
        fs::write(&bad, "SF:wrong/b.rs\nDA:1,1\nend_of_record\n").unwrap();
        let error = command(bad, &base).run(Some(&repo)).err().unwrap();
        let message = error.to_string();
        for text in [
            "none of its 1 file path(s)",
            "`wrong/b.rs`",
            "--strip-prefix",
            "--allow-path-mismatch",
            "diff.allow-path-mismatch",
        ] {
            assert!(message.contains(text), "{message}");
        }
    }

    #[test]
    fn path_mismatch_config_opt_out_downgrades_to_a_warning() {
        let (_dir, repo, base) = repo_with_added_file();
        let bad = repo.join("bad.lcov");
        fs::write(&bad, "SF:wrong/b.rs\nDA:1,1\nend_of_record\n").unwrap();
        let config_dir = repo.join(".patchcov");
        fs::create_dir_all(&config_dir).unwrap();
        let mut cmd = command(bad, &base);
        cmd.config_dir = Some(config_dir.clone());
        for (yaml, allowed) in [
            ("diff:\n  allow-path-mismatch: true\n", true),
            ("diff:\n  allow-path-mismatch: false\n", false),
            ("diff: {}\n", false),
        ] {
            fs::write(config_dir.join("config.yaml"), yaml).unwrap();
            let result = cmd.run(Some(&repo));
            if allowed {
                assert_eq!(result.unwrap().warnings.len(), 1, "{yaml}");
            } else {
                assert!(result.is_err(), "{yaml}");
            }
        }
        // The flag and the config key are a union: either one is enough.
        fs::write(config_dir.join("config.yaml"), "diff: {}\n").unwrap();
        cmd.allow_path_mismatch = true;
        assert_eq!(cmd.run(Some(&repo)).unwrap().warnings.len(), 1);
    }

    #[test]
    fn path_mismatch_gate_cannot_pass_vacuously_by_default() {
        let (_dir, repo, base) = repo_with_added_file();
        let bad = repo.join("bad.lcov");
        fs::write(&bad, "SF:/other/root/b.rs\nDA:1,1\nend_of_record\n").unwrap();
        let mut cmd = command(bad, &base);
        cmd.fail_under_patch = Some(80.0);
        assert!(cmd.run(Some(&repo)).is_err());
        cmd.allow_path_mismatch = true;
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.patch_percent, None);
        assert!(!outcome.below_gate);
    }

    #[test]
    fn deprecated_fail_on_path_mismatch_is_accepted_and_ignored() {
        let (_dir, repo, base) = repo_with_added_file();
        let mut cmd = command(write_head_lcov(&repo), &base);
        cmd.fail_on_path_mismatch = true;
        assert!(cmd.run(Some(&repo)).is_ok());
        // It cannot re-strengthen a mismatch the caller allowed.
        let bad = repo.join("bad.lcov");
        fs::write(&bad, "SF:wrong/b.rs\nDA:1,1\nend_of_record\n").unwrap();
        cmd.report = vec![bad];
        assert!(cmd.run(Some(&repo)).is_err());
        cmd.allow_path_mismatch = true;
        assert!(cmd.execute(Some(&repo)).is_ok());
    }

    #[test]
    fn windows_runner_paths_fail_instead_of_panicking() {
        let (_dir, repo, base) = repo_with_added_file();
        for path in [r"C:\agent\project\b.rs", "C:/agent/project/b.rs"] {
            let report = repo.join("invalid.lcov");
            fs::write(&report, format!("SF:{path}\nDA:1,1\nend_of_record\n")).unwrap();
            let mut cmd = command(report, &base);
            assert!(cmd.run(Some(&repo)).is_err());
            cmd.allow_path_mismatch = true;
            assert_eq!(cmd.run(Some(&repo)).unwrap().warnings.len(), 1);
            cmd.head_ref = Some("HEAD".into());
            assert_eq!(cmd.run(Some(&repo)).unwrap().warnings.len(), 1);
        }
    }

    #[test]
    fn baseline_warning_samples_at_most_three_paths() {
        let (_dir, repo, base) = repo_with_added_file();
        let baseline = repo.join("baseline.lcov");
        let mut content = String::new();
        for path in ["wrong/a.rs", "wrong/b.rs", "wrong/c.rs", "wrong/d.rs"] {
            content.push_str(&format!("SF:{path}\nDA:1,1\nend_of_record\n"));
        }
        fs::write(&baseline, content).unwrap();
        let mut cmd = command(write_head_lcov(&repo), &base);
        cmd.baseline_report = Some(baseline);
        cmd.allow_path_mismatch = true;
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.warnings.len(), 1);
        let warning = &outcome.warnings[0];
        for text in [
            "baseline.lcov",
            "4 file path(s)",
            "wrong/a.rs",
            "wrong/b.rs",
            "wrong/c.rs",
        ] {
            assert!(warning.contains(text), "{warning}");
        }
        assert!(!warning.contains("wrong/d.rs"), "{warning}");
    }

    #[test]
    fn tracked_files_with_external_paths_and_exclusions_are_quiet() {
        let (_dir, repo, base) = repo_with_added_file();
        let report = repo.join("mixed.lcov");
        fs::write(
            &report,
            "SF:b.rs\nDA:1,1\nend_of_record\nSF:/sdk/runtime.rs\nDA:1,1\nend_of_record\n",
        )
        .unwrap();
        let mut cmd = command(report, &base);
        cmd.ignore_filename_regex = vec!["^b".into()];
        assert!(cmd.run(Some(&repo)).unwrap().warnings.is_empty());
    }

    #[test]
    fn path_matching_uses_index_or_selected_revision() {
        let (_dir, repo, base) = repo_with_added_file();
        let repository = Repository::open(&repo).unwrap();
        fs::write(repo.join("staged.rs"), "one\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("staged.rs")).unwrap();
        index.write().unwrap();
        let report = repo.join("staged.lcov");
        fs::write(&report, "SF:staged.rs\nDA:1,1\nend_of_record\n").unwrap();
        let mut cmd = command(report.clone(), &base);
        assert!(cmd.run(Some(&repo)).unwrap().warnings.is_empty());
        cmd.head_ref = Some("HEAD".into());
        assert!(cmd.run(Some(&repo)).is_err());
        fs::write(&report, "SF:b.rs\nDA:1,1\nend_of_record\n").unwrap();
        assert!(cmd.run(Some(&repo)).is_ok());
        cmd.head_ref = Some(base);
        assert!(cmd.run(Some(&repo)).is_err());
        fs::write(&report, "SF:a.rs\nDA:1,1\nend_of_record\n").unwrap();
        assert!(cmd.run(Some(&repo)).is_ok());
        cmd.head_ref = None;
        let baseline = repo.join("base.lcov");
        fs::write(&baseline, "SF:b.rs\nDA:1,1\nend_of_record\n").unwrap();
        cmd.baseline_report = Some(baseline);
        assert!(cmd.run(Some(&repo)).is_err());
    }

    #[test]
    fn missing_report_errors() {
        let (_dir, repo, base) = repo_with_added_file();
        let cmd = command(repo.join("nope.lcov"), &base);
        assert!(cmd.run(Some(&repo)).is_err());
    }

    #[test]
    fn render_options_use_flags() {
        let (_dir, repo, base) = repo_with_added_file();
        let mut cmd = command(repo.join("head.lcov"), &base);
        cmd.artifact_url = Some("https://artifact".to_string());
        cmd.collapse_ranges = true;
        let opts = cmd.render_options();
        assert_eq!(opts.artifact_url.as_deref(), Some("https://artifact"));
        assert!(opts.collapse_ranges);
    }

    #[test]
    fn execute_succeeds_and_gate_bails() {
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        // Passing gate: execute prints and returns Ok.
        let mut cmd = command(report.clone(), &base);
        cmd.fail_under_patch = Some(10.0);
        assert!(cmd.execute(Some(&repo)).is_ok());

        // Failing gate: execute returns Err.
        let mut cmd = command(report, &base);
        cmd.fail_under_patch = Some(99.0);
        assert!(cmd.execute(Some(&repo)).is_err());
    }

    #[test]
    fn execute_folds_deprecated_format_flag() {
        let (_dir, repo, base) = repo_with_added_file();
        let report = write_head_lcov(&repo);
        // The deprecated `--format` is folded into `output` with a warning
        // before `run` reads it.
        let mut cmd = command(report, &base);
        cmd.format = Some(OutputFormatArg::Yaml);
        assert!(cmd.execute(Some(&repo)).is_ok());
    }

    /// The injected repo root drives BOTH the git repository and relative
    /// report-path resolution. With a RELATIVE `--report` and the injected repo
    /// at `repo` (never the process CWD), the report must be read from
    /// `repo/head.lcov` — proving `-C` is honored consistently and not split
    /// between the injected repo and the ambient CWD.
    #[test]
    fn run_anchors_repo_and_relative_report_to_injected_root() {
        let (_dir, repo, base) = repo_with_added_file();
        write_head_lcov(&repo); // writes <repo>/head.lcov
        let outcome = command(PathBuf::from("head.lcov"), &base)
            .run(Some(&repo))
            .unwrap();
        assert_eq!(outcome.patch_percent, Some(2.0 / 3.0 * 100.0));
        assert!(outcome.rendered.contains("`b.rs:2`"));
    }
    #[test]
    fn partial_added_lines_affect_output_and_both_gates_only_when_opted_in() {
        for text in [
            "SF:b.rs\nDA:1,1\nDA:2,1\nDA:3,1\nBRDA:2,0,0,1\nBRDA:2,0,1,0\nend_of_record",
            r#"<coverage><class filename="b.rs"><lines><line number="1" hits="1"/><line number="2" hits="1" condition-coverage="50% (1/2)"/><line number="3" hits="1"/></lines></class></coverage>"#,
        ] {
            let (_dir, repo, base) = repo_with_added_file();
            let path = repo.join("report");
            fs::write(&path, text).unwrap();
            let mut cmd = command(path, &base);
            cmd.fail_under_patch = Some(90.0);
            cmd.fail_under_lines = Some(90.0);
            let default = cmd.run(Some(&repo)).unwrap();
            assert_eq!(default.patch_percent, Some(100.0));
            assert!(!default.below_gate && !default.below_line_gate);
            cmd.branch_coverage = true;
            let outcome = cmd.run(Some(&repo)).unwrap();
            assert_eq!(outcome.patch_percent, Some(2.0 / 3.0 * 100.0));
            assert_eq!(outcome.line_percent, outcome.patch_percent);
            assert!(outcome.below_gate && outcome.below_line_gate);
            assert!(outcome.rendered.contains("`b.rs:2`"));
            for output in [OutputFormatArg::Json, OutputFormatArg::Yaml] {
                cmd.output = output;
                assert_eq!(
                    cmd.run(Some(&repo)).unwrap().patch_percent,
                    outcome.patch_percent
                );
            }
        }
    }

    #[test]
    fn branch_shards_union_before_scoring_in_either_order() {
        let (_dir, repo, base) = repo_with_added_file();
        let a = repo.join("a.lcov");
        let b = repo.join("b.lcov");
        fs::write(
            &a,
            "SF:b.rs\nDA:1,1\nBRDA:1,0,0,1\nBRDA:1,0,1,0\nend_of_record",
        )
        .unwrap();
        fs::write(
            &b,
            "SF:b.rs\nDA:1,1\nBRDA:1,0,0,0\nBRDA:1,0,1,1\nend_of_record",
        )
        .unwrap();
        let mut cmd = command(a, &base);
        cmd.branch_coverage = true;
        assert_eq!(cmd.run(Some(&repo)).unwrap().patch_percent, Some(0.0));
        cmd.report.push(b);
        let combined = cmd.run(Some(&repo)).unwrap();
        assert_eq!(combined.patch_percent, Some(100.0));
        cmd.report.reverse();
        assert_eq!(cmd.run(Some(&repo)).unwrap().rendered, combined.rendered);
    }

    #[test]
    fn full_to_partial_baseline_is_an_indirect_loss() {
        let (_dir, repo, base) = repo_with_added_file();
        let head = repo.join("head.lcov");
        let baseline = repo.join("base.lcov");
        fs::write(
            &head,
            "SF:a.rs\nDA:1,1\nBRDA:1,0,0,1\nBRDA:1,0,1,0\nend_of_record",
        )
        .unwrap();
        fs::write(
            &baseline,
            "SF:a.rs\nDA:1,1\nBRDA:1,0,0,1\nBRDA:1,0,1,1\nend_of_record",
        )
        .unwrap();
        let mut cmd = command(head, &base);
        cmd.baseline_report = Some(baseline);
        cmd.all_files = true;
        cmd.output = OutputFormatArg::Json;
        let default: serde_json::Value =
            serde_json::from_str(&cmd.run(Some(&repo)).unwrap().rendered).unwrap();
        assert_eq!(default["indirect_changes"]["newly_uncovered"], 0);
        cmd.branch_coverage = true;
        let outcome = cmd.run(Some(&repo)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&outcome.rendered).unwrap();
        assert_eq!(json["indirect_changes"]["newly_uncovered"], 1);
        assert_eq!(outcome.line_percent, Some(0.0));
        // Untouched files remain hidden without --all-files.
        cmd.all_files = false;
        let json: serde_json::Value =
            serde_json::from_str(&cmd.run(Some(&repo)).unwrap().rendered).unwrap();
        assert_eq!(json["indirect_changes"]["newly_uncovered"], 0);
    }

    #[test]
    fn branch_mode_rejects_unsupported_head_and_baseline_formats() {
        let (_dir, repo, base) = repo_with_added_file();
        let path = repo.join("jacoco.xml");
        fs::write(&path, r#"<report><package name=""><sourcefile name="b.rs"><line nr="1" mi="0" ci="1" mb="1" cb="1"/></sourcefile></package></report>"#).unwrap();
        let mut cmd = command(path.clone(), &base);
        cmd.branch_coverage = true;
        assert!(cmd
            .run(Some(&repo))
            .err()
            .unwrap()
            .to_string()
            .contains("supports only lcov and Cobertura"));
        cmd.report = vec![write_head_lcov(&repo)];
        cmd.baseline_report = Some(path);
        assert!(cmd
            .run(Some(&repo))
            .err()
            .unwrap()
            .to_string()
            .contains("supports only lcov and Cobertura"));
    }

    #[test]
    fn branch_flag_parses() {
        use clap::Parser;
        let cmd = DiffCommand::try_parse_from(["diff", "--report", "r.lcov", "--branch-coverage"])
            .unwrap();
        assert!(cmd.branch_coverage);
    }

    #[test]
    fn baseline_partial_to_full_gains_and_partial_to_partial_stays_uncovered() {
        let (_dir, repo, base) = repo_with_added_file();
        let head = repo.join("head.xml");
        let baseline = repo.join("base.xml");
        let report = |covered| {
            format!(
                r#"<coverage><class filename="a.rs"><line number="1" hits="1" condition-coverage="50% ({covered}/2)"/></class></coverage>"#
            )
        };
        fs::write(&baseline, report(1)).unwrap();
        fs::write(&head, report(0)).unwrap();
        let mut cmd = command(head.clone(), &base);
        cmd.baseline_report = Some(baseline);
        cmd.branch_coverage = true;
        cmd.all_files = true;
        cmd.output = OutputFormatArg::Json;
        let json: serde_json::Value =
            serde_json::from_str(&cmd.run(Some(&repo)).unwrap().rendered).unwrap();
        assert_eq!(json["indirect_changes"]["newly_uncovered"], 0);
        assert_eq!(json["indirect_changes"]["newly_covered"], 0);
        fs::write(head, report(2)).unwrap();
        let outcome = cmd.run(Some(&repo)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&outcome.rendered).unwrap();
        assert_eq!(json["indirect_changes"]["newly_covered"], 1);
        assert_eq!(outcome.line_percent, Some(100.0));
    }

    #[test]
    fn ignore_marker_removes_a_partial_line_from_branch_aware_gates() {
        let (_dir, repo, base) = repo_with_added_file();
        fs::write(
            repo.join("b.rs"),
            format!(
                "one\ntwo // {} coverage ignore-line reason=\"generated\"\nthree\n",
                "patchcov:"
            ),
        )
        .unwrap();
        let path = repo.join("head.lcov");
        fs::write(
            &path,
            "SF:b.rs\nDA:1,1\nDA:2,1\nDA:3,1\nBRDA:2,0,0,0\nend_of_record",
        )
        .unwrap();
        let mut cmd = command(path, &base);
        cmd.branch_coverage = true;
        cmd.fail_under_patch = Some(100.0);
        cmd.fail_under_lines = Some(100.0);
        let outcome = cmd.run(Some(&repo)).unwrap();
        assert_eq!(outcome.patch_percent, Some(100.0));
        assert_eq!(outcome.line_percent, Some(100.0));
        assert!(!outcome.below_gate && !outcome.below_line_gate);
        assert!(!outcome.rendered.contains("`b.rs:2`"));
    }
}
