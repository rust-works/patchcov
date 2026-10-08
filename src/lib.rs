//! Diff/patch coverage analysis.
//!
//! Ingests a per-line coverage report (lcov / llvm-cov JSON / cobertura / JaCoCo / Go
//! coverprofile) plus a
//! git diff and produces PR-attributable coverage: **patch coverage** (the
//! fraction of lines the diff added that are covered), the explicit list of
//! **uncovered new lines**, project before/after **deltas**, and **indirect**
//! coverage flips on unchanged lines.
//!
//! Pipeline:
//! 1. [`format::parse`] turns report text into a per-line [`model::CoverageReport`].
//! 2. [`diff::DiffModel::between`] builds the added-line sets and base↔head
//!    alignment from `git2`.
//! 3. [`analysis::analyze`] attributes coverage to the diff.
//! 4. [`render::render`] emits markdown / YAML / JSON.
//!
//! The command line in [`cli`] is a thin layer over these four steps, plus source
//! markers ([`markers`]), the ignore list and `config.yaml` ([`config`]), path
//! mapping ([`paths`]) and shard merging ([`merge`]). `src/cli/diff.rs` shows the
//! whole pipeline in order.
//!
//! # Example
//!
//! Attribute a coverage report to the changes on the current branch:
//!
//! ```no_run
//! use git2::Repository;
//! use patchcov::{
//!     analyze, default_base_ref, parse, render, DiffModel, DiffScope, OutputFormat, RenderOptions,
//! };
//!
//! fn main() -> anyhow::Result<()> {
//!     let repo = Repository::open(".")?;
//!
//!     // Read a report (the format is detected from its content), then make its paths
//!     // repo-relative so they line up with git's.
//!     let mut head = parse(&std::fs::read_to_string("head.lcov")?, None)?;
//!     if let Some(workdir) = repo.workdir() {
//!         head.strip_prefix(workdir);
//!     }
//!
//!     // The lines added since the merge base of the default branch and HEAD, which is what
//!     // `patchcov diff` uses by default. An explicit revision is compared directly.
//!     let base = default_base_ref(&repo)?;
//!     let diff = DiffModel::between(&repo, &base, None)?;
//!
//!     // Attribute coverage to the diff. Pass a baseline report instead of `None` for deltas.
//!     let result = analyze(&head, &diff, None, DiffScope::DiffOnly);
//!     println!("patch coverage: {:?}", result.patch.percent());
//!     println!("{}", render(&result, &RenderOptions::default(), OutputFormat::Markdown)?);
//!     Ok(())
//! }
//! ```
//!
//! The API is not stable at 0.x: breaking changes arrive in minor releases.

pub mod analysis;
pub mod cli;
pub mod cobertura;
pub mod config;
pub mod diff;
pub mod format;
pub mod go_coverprofile;
pub mod jacoco;
pub mod lcov;
pub mod llvm_json;
pub mod markers;
pub mod merge;
pub mod model;
pub mod paths;
pub mod render;
mod yaml;

pub use analysis::{
    analyze, analyze_with_markers, AppliedMarker, CoverageDiff, DiffScope, ExcludedFiles,
    MarkerSide, Markers,
};
pub use diff::{default_base_ref, DiffModel};
pub use format::{parse, Format};
pub use markers::{FileMarkers, MarkerError, MarkerKind, Region};
pub use model::{CoverageReport, FileCoverage};
pub use paths::PathMapping;
pub use render::{render, OutputFormat, RenderOptions};
