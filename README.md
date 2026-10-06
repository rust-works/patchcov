# patchcov

Patch coverage for git diffs. `patchcov` attributes a per-line coverage report to a
git diff and tells you what share of the lines a change *added* are covered, which
new lines are not, and whether anything moved on code the change did not touch.

- **Reads** lcov, llvm-cov JSON, Cobertura, JaCoCo XML and Go coverprofiles, detected
  from the content.
- **Reports** patch coverage, the uncovered new lines, per-file project deltas and
  indirect changes, as a markdown PR comment, YAML or JSON.
- **Gates** a branch with `--fail-under-patch` and `--fail-under-lines`.
- **Merges** the reports of a sharded CI run into one file, or takes them directly.
- **Silences** known-flaky regions with source markers, and files with a repo-level
  ignore list, without hiding real coverage.

## Install

```bash
cargo install --git https://github.com/rust-works/patchcov
```

It is not yet published to crates.io.

## Use

```bash
# Produce a per-line report (any tool that writes one of the supported formats).
cargo llvm-cov --no-report
cargo llvm-cov report --lcov --output-path head.lcov

# Patch coverage against the merge base with origin/main.
patchcov diff --report head.lcov

# Fail the job if patch coverage is under 80% or overall coverage under 70%.
patchcov diff --report head.lcov --fail-under-patch 80 --fail-under-lines 70

# Merge the shards of a sharded run, then gate as usual.
patchcov merge shard-1.lcov shard-2.lcov -o merged.lcov

# Check source markers without a coverage report.
patchcov lint-markers
```

The full guide is in [docs/usage.md](docs/usage.md): input formats and path mapping
for non-Rust languages, gating semantics, sharded runs, the ignore list, source
markers and the flag reference.

## Configuration

Settings that belong to a repository live in `.patchcov/config.yaml`, found by
walking up from the repository root. `--config-dir` and the `PATCHCOV_CONFIG_DIR`
environment variable override the location. There are no user-level or
machine-level settings, so what a gate reports is visible in version control.

## Library

The analysis is a library as well as a command: `patchcov::parse` reads a report,
`patchcov::DiffModel` builds the added-line sets from `git2`, `patchcov::analyze`
attributes coverage to the diff and `patchcov::render` formats the result.

## License

BSD-3-Clause. See [LICENSE](LICENSE).
