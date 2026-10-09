# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0](https://github.com/rust-works/patchcov/compare/v0.3.0...v0.4.0) - 2026-10-09

### Added

- *(cli)* mark a truncated path-mismatch sample and name its size once
- *(cli)* JSON lines for lint-markers findings and the merge summary
- *(cli)* structured fields on JSON warnings
- *(cli)* structured gate fields in the JSON error output
- *(cli)* print warnings as JSON lines under --error-format json
- *(cli)* opt-in JSON error output on stderr
- [**breaking**] exit with a distinct code for each class of failure
- *(diff)* [**breaking**] fail by default when a report's paths match no tracked file
- *(diff)* detect the default base branch from origin/HEAD, main or master

### Documentation

- align path-mismatch example with checkout normalization
- *(reference)* explain the path-mismatch change when upgrading from 0.3.0
- *(reference)* show level in the gate error example
- *(reference)* show the anchored report path in the JSON warning examples
- *(troubleshooting)* say a path mismatch exits 7, not 1
- *(reference)* word the path-mismatch truncation rule without Rust notation
- *(reference)* say how to tell that a path-mismatch unmatched sample is truncated
- *(cli)* tighten the note on display-string paths in JSON warnings
- *(cli)* say the report and shard fields of JSON warnings are display strings
- *(reference)* describe the JSON finding and merge summary lines
- *(reference)* list the fields of each JSON warning kind
- recommend cargo install --locked and document the fresh-resolution MSRV job
- *(reference)* document the JSON warning line and the error level
- *(readme)* say the other formats fail under --branch-coverage
- *(readme)* say which report formats support --branch-coverage in the intro
- *(reference)* document the exit code for each failure class
- *(release)* anchor the docs changelog parser and list every group
- *(release)* group docs commits under Documentation in the changelog
- describe the default base branch resolution order
- *(contributing)* clarify what CI covers for lint-markers and docs
- *(readme)* address review of the mission and branch-coverage wording
- *(readme)* add a mission statement and fix the branch-coverage line

### Fixed

- *(cli)* discover diff repositories from subdirectories
- *(cli)* preserve input arguments in report and shard labels
- *(cli)* explain spaced negative float thresholds
- *(cli)* preserve chain and gates in JSON error fallback
- *(cli)* simplify the mismatch-sample suffix and tighten docs wording
- *(cli)* address review of the JSON error fallback
- *(cli)* keep code in the JSON error line when the full report cannot be serialized
- *(tests)* match GitHub more closely in the doc link check
- *(cli)* report a missing --fail-under-* value instead of blaming the next argument
- *(cli)* carry the record's level and kind into the JSON fallback line
- *(cli)* print a minimal JSON line when a record fails to serialize
- *(diff)* normalise -0 thresholds and cover negatives end to end
- *(diff)* reject negative --fail-under-patch and --fail-under-lines values
- *(markers)* make MarkerError non-exhaustive and tidy the JSON line docs
- *(cli)* reject non-finite --fail-under-patch / --fail-under-lines values
- *(cli)* report measured gate values unrounded
- *(ci)* run the real unlocked cargo install in the fresh-resolution job
- *(cli)* address review of the JSON warnings
- *(cli)* address review of the JSON error output
- *(cli)* address review of the exit-code change
- *(diff)* keep the deprecated --fail-on-path-mismatch a pure no-op
- *(diff)* name the chosen base in the merge-base error; isolate the master test

### Other

- Merge pull request #126 from rust-works/issue-124-perf-json-fallback-lazy
- *(cli)* check troubleshooting example against exit-7 error
- *(cli)* build JSON fallbacks only on serialization failure
- check links in all tracked markdown except the changelog
- *(cli)* preserve as-typed labels after rebase
- *(cli)* cover baseline Go reports from subdirectories
- *(cli)* define JSON fallback fields in one place
- Merge pull request #117 from rust-works/issue-108-docs-path-mismatch-upgrade-note
- Merge pull request #120 from rust-works/issue-106-decide-relative-path-naming-in-warnings
- Merge pull request #121 from rust-works/issue-105-test-path-mismatch-doc-examples
- Merge pull request #118 from rust-works/issue-104-diff-spaced-nonfinite-threshold-message
- Merge pull request #116 from rust-works/issue-102-json-fallback-keep-gates
- *(cli)* accept CRLF in the troubleshooting example
- *(cli)* pin input labels in empty shard errors
- *(cli)* check path-mismatch doc examples against Display
- *(cli)* check warning kind field types against docs
- parse documentation links with CommonMark
- bound release and fresh MSRV job durations
- *(cli)* check JSON gate order against reference docs
- Merge pull request #90 from rust-works/issue-83-json-error-keep-code
- Merge pull request #94 from rust-works/issue-88-docs-json-warning-path
- Merge pull request #98 from rust-works/issue-85-emit-derive-level-kind
- *(cli)* cover the fallback labels through one fixed-label wrapper
- *(cli)* address review of the docs-table checks
- *(cli)* derive emit's level and kind from the record
- *(cli)* check documented types, constants and the error object against the JSON output
- *(cli)* share the isolated command setup and cover a relative -C in the warning path tests
- check relative links and anchors in README, CONTRIBUTING and docs
- *(cli)* pin the unexplained -inf spelling and more missing-value cases
- cite the measured maximum in the timeout comment
- explain the job timeout values
- set timeout-minutes on the CI jobs
- Merge pull request #77 from rust-works/issue-73-docs-path-mismatch-truncation
- Merge pull request #76 from rust-works/issue-72-docs-json-warning-non-utf8-paths
- Merge pull request #80 from rust-works/issue-70-test-json-fields-vs-docs
- *(cli)* make the docs field checks less brittle
- *(cli)* pin the JSON fields of findings, the merge summary and warnings against docs/reference.md
- *(cli)* drop a warn test that restated render
- *(cli)* share the stderr format lookup and render path between warn and emit
- *(msrv)* run the test suite on the MSRV toolchain
- *(markers)* return a structured MarkerError from scan
- Merge pull request #66 from rust-works/issue-61-json-warning-fields
- *(cli)* derive a deprecated warning's fields from its flag
- *(cli)* assert warnings by value without untested panic branches
- *(cli)* exercise -inf through the value parser and assert accepted thresholds
- *(msrv)* run doctests on the MSRV toolchain
- *(diff)* assert the patch gate's data without an untested panic branch
- Merge pull request #57 from rust-works/issue-52-ci-msrv-fresh-resolution
- *(msrv)* check the MSRV against a fresh dependency resolution weekly
- run doctests in the test job
- Merge pull request #50 from rust-works/issue-47-json-error-output
- *(exit)* do not depend on the OS wording of a missing file
- *(deps)* drop the unused clap env feature
- pass --locked to every cargo step
- *(exit)* a path mismatch exits 7 unless it is allowed
- Merge pull request #38 from rust-works/issue-35-default-base-branch
- Merge pull request #40 from rust-works/issue-30-cargo-binstall
- *(binstall)* fail when the binstall metadata drifts from the release workflow
- *(binstall)* describe the release archives to cargo-binstall
- run patchcov lint-markers in the Lint job

### Changed

- Report and shard diagnostic labels now preserve the input argument in `diff` and
  `merge`, even with `-C`. This changes warning messages and JSON `report`/`shard`
  values that previously included the working directory, and the corresponding
  path-mismatch and empty-shard error messages. File resolution is unchanged.

## [0.3.0](https://github.com/rust-works/patchcov/compare/v0.2.0...v0.3.0) - 2026-10-07

### Fixed

- *(diff)* avoid index lookup panic on Windows report paths
- *(diff)* warn when report paths match no tracked files

### Other

- explain that an explicit --base-ref is not a merge base
- *(contributing)* add contributor guide and changelog guidance
- *(readme)* show sample output and cover install, library and config
- split usage into usage, reference, explanation and troubleshooting
- *(diff)* cover baseline path mismatch diagnostics

## [0.2.0](https://github.com/rust-works/patchcov/compare/v0.1.1...v0.2.0) - 2026-10-07

### Added

- *(diff)* allow omitting structured output explanation

### Other

- *(readme)* lead with availability as a reason to choose patchcov
- *(readme)* explain why and when to choose patchcov
- require the merge queue for main
- *(readme)* introduce the patchcov-action GitHub Action
- *(readme)* add Patch the hermit crab mascot

## [0.1.1](https://github.com/rust-works/patchcov/compare/v0.1.0...v0.1.1) - 2026-10-06

### Other

- *(release)* make RELEASE_PLZ_TOKEN the recommended PR setup

## [0.1.0] - 2026-10-06

### Added
- Initial release: patch coverage for git diffs, as the `patchcov` command and
  library.
  - `patchcov diff` attributes an lcov, llvm-cov JSON, Cobertura, JaCoCo or Go
    coverprofile report to a git diff and reports patch coverage, the uncovered new
    lines, per-file project deltas and indirect changes, as markdown, YAML or JSON,
    with `--fail-under-patch` and `--fail-under-lines` gates.
  - `patchcov merge` merges the per-shard reports of a sharded run into one lcov
    file.
  - `patchcov lint-markers` checks the syntax of `patchcov: coverage ignore` and
    `tolerate` source markers.
  - Repository settings live in `.patchcov/config.yaml`; `PATCHCOV_CONFIG_DIR` and
    `--config-dir` override discovery.

  The code was extracted, with its history, from `omni-dev coverage`. It is a clean
  break: the config directory, environment variable and marker introducer are
  patchcov's own, and nothing reads `.omni-dev/` or `OMNI_DEV_*`.
