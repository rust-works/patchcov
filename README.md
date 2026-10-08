<p align="center">
  <img src="docs/assets/patch.svg" alt="Patch, the patchcov hermit crab, mending a hole in its quilted shell" width="200">
</p>

# patchcov

Patch coverage for git diffs. `patchcov` attributes a per-line coverage report to a
git diff and tells you what share of the lines a change *added* are covered, which
new lines are not, and whether anything moved on code the change did not touch.

- **Reads** lcov, llvm-cov JSON, Cobertura, JaCoCo XML and Go coverprofiles, detected
  from the content. Branch-aware scoring
  ([`--branch-coverage`](docs/usage.md#opt-in-branch-coverage)) reads lcov and Cobertura
  only and fails explicitly on the other formats.
- **Reports** patch coverage, the uncovered new lines, per-file project deltas and
  indirect changes, as a markdown PR comment, YAML or JSON.
- **Gates** a branch with `--fail-under-patch` and `--fail-under-lines`.
- **Merges** the reports of a sharded CI run into one file, or takes them directly.
- **Silences** known-flaky regions with source markers, and files with a repo-level
  ignore list, without hiding real coverage.

## What it prints

`patchcov diff` writes the pull-request comment as markdown to stdout. This one is for a
change that adds 12 lines to `src/parser.rs` and a new `src/cache.rs`, with a baseline
report so the per-file table appears:

> ## Coverage
>
> Total: **85.42%** 🔴 -14.58 pp vs `main`
>
> | File | Before | After | Δ |
> |------|-------:|------:|---|
> | `src/cache.rs` | — | 66.67% | 🆕 new |
> | `src/parser.rs` | 100% | 84.38% | 🔴 -15.63 pp |
>
> ### Patch coverage
>
> Patch: **61.11%** (11/18 new lines covered)
>
> | File | Patch | Uncovered new lines |
> |------|------:|---------------------|
> | `src/cache.rs` | 66.67% (4/6) | 5-6 |
> | `src/parser.rs` | 58.33% (7/12) | 28-32 |
>
> <details><summary>Uncovered new lines (7)</summary>
>
> - `src/cache.rs:5`
> - `src/cache.rs:6`
> - `src/parser.rs:28`
> - `src/parser.rs:29`
> - `src/parser.rs:30`
> - `src/parser.rs:31`
> - `src/parser.rs:32`
>
> </details>
>
> <sub>Full per-file summary is attached as the **coverage-summary** build artifact.</sub>

Without `--baseline-report` there is no delta on the `Total` line, no per-file table and no
indirect changes; a "No baseline available yet" notice stands in for them and the patch
section is unchanged. With `-o json` or `-o yaml` the same result is structured
data, in the shape described in the [output schema](docs/reference.md#output-schema):

```json
{
  "patch_coverage": { "percent": 61.11, "covered": 11, "total": 18, "files": [ ... ] },
  "uncovered_new_lines": ["src/cache.rs:5", "src/cache.rs:6", "src/parser.rs:28", ...],
  "unmeasured_files": [],
  "project_delta": { "total_before": 100.0, "total_after": 85.42, "files": [ ... ] },
  "indirect_changes": { "newly_covered": 0, "newly_uncovered": 0, "lines": [] }
}
```

Both are the output of [`docs/examples/sample.sh`](docs/examples/sample.sh), which builds a
tiny git repository and runs the command, so you can reproduce them.

## Mission

> Help projects that use agentic workflows keep their new code properly tested, with
> minimal friction: a precise, machine-readable answer to "what did this change leave
> uncovered, and where?", in a gate that depends on nothing but your own CI. The result
> should not need a human to interpret it: an agent or a CI step can act on the exit code
> and the `file:line` list, and loop until the gate passes.

The aim is adequately tested new code, found and fixed quickly, not a coverage number for
its own sake: patchcov measures that lines ran, not that tests assert anything (see the
end of [When to choose patchcov](#when-to-choose-patchcov)).

## Why patchcov

patchcov does one job: it tells you, precisely and repeatably, whether a change
is covered by tests. It is built to be a clear signal, not a dashboard.

- **A precise signal.** The headline number is the share of *added* lines that are
  covered, with the uncovered ones listed as `file:line`. Whoever or whatever reads it
  (a reviewer, a script, an AI agent) knows what to do next without interpreting
  charts or trends.
- **Machine-readable and gateable.** JSON and YAML output with a
  [documented schema](docs/reference.md#output-schema), and exit codes from
  `--fail-under-patch` and `--fail-under-lines`, make it easy to automate, including in
  a loop where an agent adds tests until the gate passes.
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
- **Fails loudly.** An empty report, a failed shard, a report whose paths match no tracked
  file or a malformed marker is an error, not a quietly lower number. Everything that
  silences coverage needs a stated reason and is listed in the PR comment, and settings live
  in `.patchcov/config.yaml` in version control, so a reviewer can see what a gate does and
  what was excluded.
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
- per-branch coverage. `diff --branch-coverage` scores lcov and Cobertura branches per
  *line* (`merge` drops branch records), and does not read JaCoCo or llvm-cov JSON
  branches or report per-branch percentages; see
  [opt-in branch coverage](docs/usage.md#opt-in-branch-coverage);
- a broader code-quality platform that covers coverage along with static analysis and
  security scanning.

patchcov measures whether lines ran, not whether the tests assert anything useful about
them. Treat it as a regression signal, and pair it with review, or mutation testing,
for confidence in the tests themselves.

## Install

```bash
cargo install patchcov
```

Prebuilt binaries for Linux (glibc 2.35 or newer; x86_64 and aarch64), macOS (Apple silicon
and Intel) and Windows (x86_64 MSVC) are attached to each
[GitHub release](https://github.com/rust-works/patchcov/releases), along with SHA-256
checksums. Linux and macOS builds use `.tar.gz` archives; Windows builds use
`patchcov-v<version>-x86_64-pc-windows-msvc.zip`. Extract the Windows ZIP and add the
directory containing `patchcov.exe` to your `PATH`. The binaries are not signed or
notarized, so macOS quarantines a downloaded one; `cargo install` avoids that.

To install a prebuilt binary without downloading an archive by hand, use
[cargo-binstall](https://github.com/cargo-bins/cargo-binstall):

```bash
cargo binstall patchcov
```

It fetches the release archive for your platform and falls back to building from source on
any other. Recent cargo-binstall releases (1.25 was tested) already find these archives by
guessing their names; the crate's `[package.metadata.binstall]` states the layout, so the
lookup no longer depends on that guess. Because cargo-binstall reads that metadata from the
published crate, it applies from the first release after 0.2.0. cargo-binstall downloads
with its own client, so the macOS quarantine caveat above does not apply.

### Verify a download

Each archive has a `.sha256` file next to it holding `<hash>  <archive name>`. Download both
into one directory and check the archive against it.

```bash
# Linux and macOS
shasum -a 256 -c patchcov-v0.2.0-x86_64-unknown-linux-gnu.tar.gz.sha256
# patchcov-v0.2.0-x86_64-unknown-linux-gnu.tar.gz: OK
```

```powershell
# Windows (PowerShell)
$expected = (Get-Content patchcov-v0.2.0-x86_64-pc-windows-msvc.zip.sha256).Split(' ')[0]
$actual = (Get-FileHash patchcov-v0.2.0-x86_64-pc-windows-msvc.zip -Algorithm SHA256).Hash
if ($actual -eq $expected) { 'OK' } else { 'MISMATCH' }
```

Checksums are published beside the archives, so they catch a corrupted download but not a
compromised release; they are not a signature.

## Use

```bash
# Produce a per-line report (any tool that writes one of the supported formats).
cargo llvm-cov --no-report
cargo llvm-cov report --lcov --output-path head.lcov

# Patch coverage against the merge base with the default branch (origin/HEAD, else main or master).
patchcov diff --report head.lcov

# Fail the job if patch coverage is under 80% or overall coverage under 70%.
patchcov diff --report head.lcov --fail-under-patch 80 --fail-under-lines 70

# Merge the shards of a sharded run, then gate as usual.
patchcov merge shard-1.lcov shard-2.lcov -o merged.lcov

# Check source markers without a coverage report.
patchcov lint-markers
```

**Measure the report at the revision you diff.** The report must come from a test run on
the code at `HEAD` (or at `--head-ref`). A report measured on older code gives line numbers
that no longer match the diff, and patchcov cannot tell: the result is silently wrong. The
merge base must also resolve, which needs full git history in CI (`fetch-depth: 0`).

The command prints the report to stdout and exits `0`, or `1` when a gate fails, so a CI job
can post the comment and then fail. Other failures have their own codes (`2` usage, `3` report,
`4` marker, `5` config, `6` git, `7` path mismatch, `8` other), so a script can tell a failed
gate from an unreadable report. See the [exit codes](docs/reference.md#exit-codes), and
`--error-format json` for the cause as a JSON object on stderr
([error output](docs/reference.md#error-output)).

Where to go next:

- [Usage](docs/usage.md): input formats and path mapping for non-Rust languages, gating,
  sharded runs, the ignore list and source markers, CI.
- [Reference](docs/reference.md): every flag, the `.patchcov/config.yaml` keys, the JSON/YAML
  schema, exit codes and environment variables.
- [Explanation](docs/explanation.md): why the total differs from llvm-cov's summary, how
  `tolerate` masking and diff scoping work.
- [Troubleshooting](docs/troubleshooting.md): an empty result, no files matching, an
  unresolvable merge base, a report from the wrong revision.

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

Settings that belong to a repository live in `.patchcov/config.yaml`, found by walking up from
the repository root. `--config-dir` and the `PATCHCOV_CONFIG_DIR` environment variable
override the location. There are no user-level or machine-level settings, so what a gate
reports is visible in version control.

| Key | Purpose |
|-----|---------|
| `diff.ignore-filename-regex` | Regexes for files to exclude from both reports; unioned with the flag |
| `diff.path-mappings` | `from`/`to` directory replacements for reports whose paths differ from git's |
| `diff.require-measured` | Globs of touched files that must appear in a report; unioned with the flag |
| `diff.allow-path-mismatch` | Warn instead of failing when a report matches no tracked file; enabled by either this or the flag |
| `lint-markers.include` | Globs narrowing which files `lint-markers` scans; replaced by the flag |

```yaml
# .patchcov/config.yaml
diff:
  ignore-filename-regex:
    - 'src/bits/popcount\.rs'   # CPU-gated, flaps between runners
lint-markers:
  include:
    - '**/*.rs'
```

Types, defaults and how each key combines with its command-line flag are in the
[config reference](docs/reference.md#patchcovconfigyaml). This repository's own
[`.patchcov/config.yaml`](.patchcov/config.yaml) is a working example.

## Library

The analysis is a library as well as a command, published on crates.io with API documentation
at <https://docs.rs/patchcov>. `patchcov::parse` reads a report, `patchcov::DiffModel` builds
the added-line sets from `git2`, `patchcov::analyze` attributes coverage to the diff and
`patchcov::render` formats the result:

```rust
use git2::Repository;
use patchcov::{
    analyze, default_base_ref, parse, render, DiffModel, DiffScope, OutputFormat, RenderOptions,
};

fn main() -> anyhow::Result<()> {
    let repo = Repository::open(".")?;

    // Read a report (the format is detected from its content), then make its paths
    // repo-relative so they line up with git's.
    let mut head = parse(&std::fs::read_to_string("head.lcov")?, None)?;
    if let Some(workdir) = repo.workdir() {
        head.strip_prefix(workdir);
    }

    // The lines added since the merge base of the default branch and HEAD, which is what
    // `patchcov diff` uses by default. An explicit revision is compared directly.
    let base = default_base_ref(&repo)?;
    let diff = DiffModel::between(&repo, &base, None)?;

    // Attribute coverage to the diff. Pass a baseline report instead of `None` for deltas.
    let result = analyze(&head, &diff, None, DiffScope::DiffOnly);
    println!("patch coverage: {:?}", result.patch.percent());
    println!("{}", render(&result, &RenderOptions::default(), OutputFormat::Markdown)?);
    Ok(())
}
```

This example is compiled as a doctest in [`src/lib.rs`](src/lib.rs), which also holds the
architecture overview. **The API is not stable at 0.x.** Breaking changes are allowed in minor
releases (`0.2` to `0.3`), and the release process runs `cargo-semver-checks` so each one is
accompanied by a minor version bump. Depend on `patchcov = "~0.2"` if you need to avoid surprises,
and read the [changelog](CHANGELOG.md) when upgrading. The command line is
the better-supported interface.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, run the tests and add a fixture, and
for the merge queue flow.

## Releasing

See [docs/RELEASE.md](docs/RELEASE.md).

## License

BSD-3-Clause. See [LICENSE](LICENSE).
