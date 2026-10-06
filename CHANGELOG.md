# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
