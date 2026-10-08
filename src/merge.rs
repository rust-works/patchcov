//! Checks on the per-shard reports of a sharded coverage run.
//!
//! The union itself is [`CoverageReport::merge`]. What lives here is the part
//! that keeps a merge honest: a shard that silently produced nothing, or was
//! measured under a different workspace root, would otherwise lower the merged
//! coverage — or simply not line up with the diff — without anyone noticing.

use std::fmt;
use std::path::Path;

use anyhow::{bail, Result};
use serde::Serialize;

use super::model::CoverageReport;

/// Runs every per-shard check on a freshly parsed `report`, before path
/// filtering and before `prefix` is stripped.
///
/// Used by `patchcov merge`: a shard with no executable lines is an error,
/// and one measured under another root is noted in `warnings` when `prefix`
/// is known. `patchcov diff` shares the executable-line check but validates
/// normalized paths against tracked repository files instead.
pub fn check_shard(
    label: &str,
    report: &CoverageReport,
    prefix: Option<&Path>,
    warnings: &mut Vec<PrefixMismatch>,
) -> Result<()> {
    require_executable_lines(label, report)?;
    if let Some(prefix) = prefix {
        warnings.extend(prefix_mismatch(label, report, prefix));
    }
    Ok(())
}

/// Fails when `report` has no executable lines.
///
/// A shard that ran instrumented tests lists every instrumented line, covered
/// or not, so a shard with none means the run failed or its report was lost.
/// Merging it in would not change the result — a union ignores an empty side —
/// so the failure would never surface; this is where it has to.
///
/// `label` names the shard in the message (normally its path). Call this on the
/// freshly parsed report, **before** path filtering: a shard whose files an
/// `--ignore-filename-regex` happens to exclude is not broken.
pub fn require_executable_lines(label: &str, report: &CoverageReport) -> Result<()> {
    if report.total_lines() == 0 {
        bail!(
            "coverage shard {label} has no executable lines; \
             a failed or empty shard would silently lower the merged coverage"
        );
    }
    Ok(())
}

/// A shard measured under a different workspace root, found by
/// [`prefix_mismatch`].
///
/// Its [`Display`](fmt::Display) is the warning `patchcov` prints; the fields are
/// what `--error-format json` adds to that warning, under the names they have
/// here, see `docs/reference.md#warnings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct PrefixMismatch {
    /// The shard, as named in messages (normally its path): a display string,
    /// with U+FFFD for each invalid byte sequence of a non-UTF-8 path, not a
    /// path identifier.
    pub shard: String,
    /// The prefix the shard's absolute paths were expected under, without a
    /// trailing `/`. Lossy in the same way as `shard`.
    pub strip_prefix: String,
    /// How many absolute file paths the shard has, none of them under the prefix.
    pub absolute_paths: usize,
}

impl fmt::Display for PrefixMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "coverage shard {}: none of its {} absolute file path(s) is under \
             `{}`, so it was probably measured under a different workspace root and its \
             files will not line up with the other shards or the diff; pass --strip-prefix \
             with that root, or run every shard under the same one",
            self.shard, self.absolute_paths, self.strip_prefix
        )
    }
}

/// Describes a shard measured under a different workspace root, or `None` when
/// it looks fine.
///
/// Report paths are made repo-relative by stripping one prefix, and a path
/// outside it keeps its directory components as if they were relative. A shard
/// from a runner with another workspace root therefore keys every file under a
/// path that exists nowhere in the repository, and its coverage merges in as
/// extra files rather than against the diff.
///
/// The signal is that the shard has absolute paths and **none** starts with
/// `prefix`. A shard with some in-tree paths is left alone, since reports
/// legitimately name a few out-of-tree files (the standard library, vendored
/// sources). Call this on the freshly parsed report, **before** the prefix is
/// stripped.
pub fn prefix_mismatch(
    label: &str,
    report: &CoverageReport,
    prefix: &Path,
) -> Option<PrefixMismatch> {
    let prefix = prefix.to_string_lossy();
    let prefix = prefix.trim_end_matches('/');
    let prefix_slash = format!("{prefix}/");
    let mut absolute = 0_usize;
    for path in report.files.keys() {
        if path.starts_with(&prefix_slash) {
            return None;
        }
        if path.starts_with('/') {
            absolute += 1;
        }
    }
    (absolute > 0).then(|| PrefixMismatch {
        shard: label.to_owned(),
        strip_prefix: prefix.to_owned(),
        absolute_paths: absolute,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::model::FileCoverage;

    fn report(files: &[(&str, &[(u32, u64)])]) -> CoverageReport {
        let mut report = CoverageReport::new();
        for (path, lines) in files {
            let mut file = FileCoverage::new(*path);
            for &(line, hits) in *lines {
                file.record(line, hits);
            }
            report.insert(file);
        }
        report
    }

    #[test]
    fn an_empty_report_is_rejected_by_name() {
        let message = require_executable_lines("shard-2.lcov", &CoverageReport::new())
            .unwrap_err()
            .to_string();
        assert!(message.contains("shard-2.lcov"), "{message}");
        assert!(message.contains("no executable lines"), "{message}");
    }

    #[test]
    fn a_report_whose_files_have_no_lines_is_rejected() {
        // A parsed `SF:` block with no `DA:` records yields a file with no lines.
        let report = report(&[("src/a.rs", &[])]);
        assert!(require_executable_lines("s", &report).is_err());
    }

    #[test]
    fn a_report_with_executable_lines_passes() {
        let report = report(&[("src/a.rs", &[(1, 0)])]);
        require_executable_lines("s", &report).unwrap();
    }

    #[test]
    fn check_shard_fails_an_empty_shard_and_notes_a_foreign_one() {
        let mut warnings = Vec::new();
        let prefix = Path::new("/home/runner/work/repo");

        let empty = check_shard("s", &CoverageReport::new(), Some(prefix), &mut warnings);
        assert!(empty.is_err());
        assert!(warnings.is_empty());

        let foreign = report(&[("/Users/dev/work/repo/src/a.rs", &[(1, 1)])]);
        check_shard("s", &foreign, Some(prefix), &mut warnings).unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");

        // Without a prefix there is nothing to compare against.
        check_shard("s", &foreign, None, &mut warnings).unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn a_shard_under_another_root_is_flagged() {
        let report = report(&[
            ("/Users/dev/work/repo/src/a.rs", &[(1, 1)]),
            ("/Users/dev/work/repo/src/b.rs", &[(1, 1)]),
        ]);
        let mismatch =
            prefix_mismatch("shard-2.lcov", &report, Path::new("/home/runner/work/repo")).unwrap();
        assert_eq!(
            mismatch,
            PrefixMismatch {
                shard: "shard-2.lcov".to_owned(),
                strip_prefix: "/home/runner/work/repo".to_owned(),
                absolute_paths: 2,
            }
        );
        let message = mismatch.to_string();
        assert!(message.contains("shard-2.lcov"), "{message}");
        assert!(message.contains("2 absolute file path(s)"), "{message}");
        assert!(message.contains("`/home/runner/work/repo`"), "{message}");
        assert!(message.contains("--strip-prefix"), "{message}");
    }

    #[test]
    fn a_shard_with_some_in_tree_paths_is_not_flagged() {
        // The standard library and vendored sources are out of tree in every
        // llvm-cov report; they must not make an ordinary shard look wrong.
        let report = report(&[
            ("/home/runner/work/repo/src/a.rs", &[(1, 1)]),
            ("/rustc/abc123/library/core/src/option.rs", &[(1, 1)]),
        ]);
        assert_eq!(
            prefix_mismatch("s", &report, Path::new("/home/runner/work/repo")),
            None
        );
    }

    #[test]
    fn relative_paths_are_not_flagged() {
        let report = report(&[("src/a.rs", &[(1, 1)])]);
        assert_eq!(
            prefix_mismatch("s", &report, Path::new("/home/runner/work/repo")),
            None
        );
    }

    #[test]
    fn a_prefix_that_is_only_a_string_prefix_does_not_match() {
        // `/work/repo-two/…` shares the string prefix `/work/repo` but is a
        // different directory.
        let report = report(&[("/work/repo-two/src/a.rs", &[(1, 1)])]);
        assert!(prefix_mismatch("s", &report, Path::new("/work/repo")).is_some());
    }

    #[test]
    fn a_trailing_slash_on_the_prefix_is_ignored() {
        let report = report(&[("/work/repo/src/a.rs", &[(1, 1)])]);
        assert_eq!(
            prefix_mismatch("s", &report, Path::new("/work/repo/")),
            None
        );
        // The reported prefix has no trailing slash either.
        let mismatch = prefix_mismatch("s", &report, Path::new("/home/ci/")).unwrap();
        assert_eq!(mismatch.strip_prefix, "/home/ci");
        assert!(mismatch.to_string().contains("under `/home/ci`,"));
    }
}
