<p align="center">
  <img src="docs/assets/patch.svg" alt="Patch, the patchcov hermit crab, mending a hole in its quilted shell" width="200">
</p>

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

## Why patchcov

patchcov does one job: it tells you, precisely and repeatably, whether a change
is covered by tests. It is built to be a clear signal, not a dashboard.

- **A precise signal.** The headline number is the share of *added* lines that are
  covered, with the uncovered ones listed as `file:line`. Whoever or whatever reads it
  (a reviewer, a script, an AI agent) knows what to do next without interpreting
  charts or trends.
- **Machine-readable and gateable.** JSON and YAML output, `--collapse-ranges` for a
  compact list, and exit codes from `--fail-under-patch` and `--fail-under-lines`
  make it easy to automate, including in a loop where an agent adds tests until the
  gate passes.
- **No service in the loop, so no outage to block you.** It reads report files from
  disk and sends nothing anywhere: no account, upload token or server to be down, slow
  or rate-limited. It runs on the same runners as your other CI jobs, so the coverage
  gate is available whenever your CI is, and a hosted quality server going down for
  days cannot block merges. It runs the same on a laptop or in a sandbox, so a coverage
  check is a command you can run before you push, not something you learn about
  afterwards.
- **Quiet by construction.** Coverage that flaps between runs (a region gated on a
  runtime CPU feature, say) is the usual source of phantom regressions. Per-file
  scoping, a tolerance on the headline delta and `tolerate` source markers keep that
  noise from reading as a regression while the reported numbers stay honest.
- **Fails loudly.** An empty report, a failed shard or a malformed marker is an error,
  not a quietly lower number. Everything that silences coverage needs a stated
  reason and is listed in the PR comment, and settings live in `.patchcov/config.yaml`
  in version control, so a reviewer can see what a gate does and what was excluded.
- **One tool across languages.** Five report formats are detected from content, so the
  same command works for Rust, Go, Java and Kotlin, JavaScript and TypeScript,
  Python, C and C++ and more.

## When to choose patchcov

Choose patchcov when:

- the question you need answered is "did this change add untested code, and where?";
- coverage is checked by automation, including AI agents, as much as by people;
- you want the coverage gate to depend on nothing but your own CI runners, so an
  outage of an external quality server or coverage service cannot block your work;
- you want the check to run locally and in CI without sending code coverage to an
  external service;
- your coverage is noisy and you need regressions you can trust;
- you run a sharded CI job and want a single combined result and gate.

Choose something else, or use patchcov alongside it, when you need:

- a hosted dashboard with coverage history, trend graphs or cross-repository views;
- branch coverage (patchcov reads line coverage only);
- a broader code-quality platform that covers coverage along with static analysis and
  security scanning.

patchcov measures whether lines ran, not whether the tests assert anything useful about
them. Treat it as a regression signal, and pair it with review, or mutation testing,
for confidence in the tests themselves.

## Install

```bash
cargo install patchcov
```

Prebuilt binaries for Linux (glibc 2.35 or newer), macOS and Windows (x86_64 MSVC)
are attached to each [GitHub release](https://github.com/rust-works/patchcov/releases),
along with SHA-256 checksums. Linux and macOS builds use `.tar.gz` archives; Windows
builds use `patchcov-v<version>-x86_64-pc-windows-msvc.zip`. Extract the Windows ZIP
and add the directory containing `patchcov.exe` to your `PATH`.

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

## GitHub Action

[action-works/patchcov-action](https://github.com/action-works/patchcov-action) runs
`patchcov` in a pull-request workflow. It installs a cached `patchcov` binary, runs
`cargo-llvm-cov` (or takes a report you produced in any language), posts a sticky PR
comment with patch coverage and the uncovered new lines, publishes the baseline on
`main` and applies the gates after the comment posts. It also combines the reports
of sharded runs.

```yaml
jobs:
  coverage:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      pull-requests: write        # to post the coverage comment
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0          # full history so the merge base resolves
      - uses: action-works/patchcov-action@v1
        with:
          fail-under-patch: 80
```

See the action's README for thin mode, sharded runs and its inputs.

## Configuration

Settings that belong to a repository live in `.patchcov/config.yaml`, found by
walking up from the repository root. `--config-dir` and the `PATCHCOV_CONFIG_DIR`
environment variable override the location. There are no user-level or
machine-level settings, so what a gate reports is visible in version control.

## Library

The analysis is a library as well as a command: `patchcov::parse` reads a report,
`patchcov::DiffModel` builds the added-line sets from `git2`, `patchcov::analyze`
attributes coverage to the diff and `patchcov::render` formats the result.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for local checks and the merge queue flow.

## Releasing

See [docs/RELEASE.md](docs/RELEASE.md).

## License

BSD-3-Clause. See [LICENSE](LICENSE).
