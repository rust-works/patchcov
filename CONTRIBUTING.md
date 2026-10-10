# Contributing

Open pull requests against `main` and use conventional commit messages (`feat:`, `fix:`,
`docs:`, etc.). Keep those messages on the branch: the merge queue uses merge commits,
preserving them for release-plz.

## Getting oriented

patchcov is a library with a thin command-line layer. The architecture overview is the crate
documentation at the top of [`src/lib.rs`](src/lib.rs): a report is parsed into a per-line
model, a git diff is turned into added-line sets, the two are joined, and the result is
rendered. Read it first, then:

| To change | Look in |
|-----------|---------|
| A report format | `src/lcov.rs`, `src/llvm_json.rs`, `src/cobertura.rs`, `src/jacoco.rs`, `src/go_coverprofile.rs`; detection in `src/format.rs` |
| How coverage is attributed to a diff | `src/diff.rs` (git side) and `src/analysis.rs` |
| The markdown, YAML or JSON output | `src/render.rs` |
| Flags and the order of the pipeline | `src/cli/diff.rs`, `src/cli/merge.rs`, `src/cli/lint_markers.rs` |
| Source markers | `src/markers.rs` |
| Path mappings and `.patchcov/config.yaml` | `src/paths.rs`, `src/config.rs` |

The user-facing docs live in [`README.md`](README.md) and [`docs/`](docs/): `usage.md` for tasks,
`reference.md` for flags, config, schema and exit codes, `explanation.md` for rationale and
`troubleshooting.md`. A change to a flag, a config key, the output schema or an exit code
should update `docs/reference.md` in the same pull request.

## Running the tests

```bash
cargo test --all-targets          # unit and integration tests (not doctests)
cargo test --doc                  # the crate-level example in src/lib.rs is a doctest
cargo test <name>                 # one test, by substring
cargo test --test diff_test       # one integration test file
```

Unit tests sit beside the code in `#[cfg(test)]` modules. The integration tests in
[`tests/`](tests/) build a temporary git repository per test, commit a base and a head
revision, and run the real analysis (`tests/diff_test.rs`) or the real binary
(`tests/lint_markers_test.rs`). They need no network.

To measure patchcov's own coverage, which is a good way to check a change is tested, commit
first (the diff is between committed trees) and then:

```bash
cargo llvm-cov --no-report
cargo llvm-cov report --lcov --output-path head.lcov
patchcov diff --report head.lcov --fail-under-patch 100   # or target/debug/patchcov
```

## Adding a fixture

Fixtures are real or hand-written coverage reports under
[`tests/fixtures/`](tests/fixtures/). The non-Rust producer reports in
`tests/fixtures/non-rust/` are used by `producer_fixture` in `tests/diff_test.rs`, which
commits a source file at the path git would have, runs `patchcov diff` on the fixture without
a mapping (expecting an empty patch), then with the `path-mappings` entry you give it
(expecting the covered and total line counts).

To add one:

1. Generate the report with the real tool and keep it unedited where you can. Put it in
   `tests/fixtures/non-rust/`, and record the tool and version, the exact command, the source
   it measured and the number of executable and covered lines in
   [`tests/fixtures/non-rust/README.md`](tests/fixtures/non-rust/README.md), as the existing
   entries do. Say so if a file was trimmed, copied from upstream (with its licence) or
   written by hand.
2. Add a test that calls `producer_fixture(fixture, git_path, from, to, covered, total)`
   with the mapping the fixture needs.
3. `.gitignore` excludes `*.lcov` except under `tests/fixtures/`, so an lcov fixture there is
   committed without further steps.

## Local checks

Run the same checks as CI from your checkout:

```bash
cargo fmt --check
cargo test --locked --all-targets
cargo test --locked --doc
cargo clippy --locked --all-targets -- -D warnings
cargo run --locked -- lint-markers
cargo +1.88.0 test --locked --all-targets
cargo +1.88.0 test --locked --doc
RUSTDOCFLAGS='-D warnings' cargo doc --locked --no-deps --document-private-items
```

Install the MSRV toolchain with `rustup toolchain install 1.88.0 --profile minimal` if needed.
CI also runs tests on Linux, macOS, and Windows.

The scheduled [MSRV (fresh resolution)](.github/workflows/msrv-fresh.yml) workflow checks what
`cargo install patchcov` without `--locked` would build. It runs the unlocked install on the MSRV
toolchain, so cargo ignores `Cargo.lock` and resolves the newest dependencies. To reproduce it:

```bash
cargo +1.88.0 install --path . --root "$(mktemp -d)"
```

It can go red when a transitive dependency raises its MSRV, so it runs weekly and is not a
required check: do not add it to the ruleset or the list below. Fix a failure by pinning or
capping the dependency in `Cargo.toml`, or by raising the MSRV on purpose.

CI's `Coverage` job runs [action-works/patchcov-action](https://github.com/action-works/patchcov-action)
on pull requests, so patchcov measures itself with the workflow it recommends. It posts a sticky
comment with the patch coverage and the uncovered new lines, fails below 90% patch coverage or 95%
overall line coverage, and on a push to `main` publishes the baseline that later pull requests are
compared with. `Coverage` is required, so a red run blocks a merge. It is skipped on
`merge_group`, where the action would only repeat the instrumented tests; the skipped job
satisfies the queue's required check. The job grants `actions: read` as well as
`pull-requests: write` because the baseline is another run's artifact. Pull requests from forks
get a read-only token, so the job sets `comment: false` for them: the gates still apply, and the
numbers are in the run's Summary tab.

CI also runs an advisory `Coverage (PR build)` job on pull requests. It checks out the
PR head, builds its debug `patchcov` binary with the lockfile, then measures that same
checkout with cargo-llvm-cov. The local binary renders a separate sticky comment and
enforces 90% patch coverage and 95% overall line coverage. Its report is also in the
run summary and the `coverage-pr-build` artifact, including when a coverage gate fails.
Fork PRs skip the comment and still get the summary, artifact and gates. This job is not
in the ruleset or required-check list and is skipped on pushes and merge groups.

The existing `Coverage` job continues to test the released binary and action. The PR-build
job has no baseline, so it reports patch and total coverage without project deltas or
indirect changes; those remain in the release report. It adds a debug build and a separate
instrumented test run, with its own cache key and a 15-minute timeout.

`lint-markers` also runs in CI's `Lint` job. This repository's
[`.patchcov/config.yaml`](.patchcov/config.yaml) limits it to Rust sources, so prose in
documentation that describes the marker syntax is not flagged.

CI also checks that relative links and `#anchors` in every tracked `.md` file still resolve,
except the generated root `CHANGELOG.md`, as part of `cargo test --all-targets`. This includes
fixture documentation and markdown under `.github/`. Stage new documentation files with
`git add` so the local check discovers them. After a documentation change, run it on its own
with:

```bash
cargo test --test doc_links_test
```

It is offline: it reads the markdown, applies GitHub's heading-slug rules and ignores external
URLs. File names are matched case-sensitively, as on GitHub, even on macOS and Windows.
Fragments are checked only against markdown targets, so `src/lib.rs#L10` is not. Links inside
code blocks, code spans and HTML comments are skipped. CommonMark parsing handles headings
(including setext headings), links and images, including reference links and parenthesized
destinations. Unused reference definitions are ignored. Raw HTML support covers double-quoted
`src`/`href` attributes and explicit `<a id="...">` / `<a name="...">` anchors. Symlink entries
under `docs/` are skipped to avoid cycles.

## Merging

`main` requires a pull request and the merge queue. Choose **Merge when ready**
on GitHub to join the queue once the PR requirements pass. The queue runs CI
again against the latest `main`, including changes ahead of yours, before it
merges. A failed or timed-out queue entry is removed; fix the failure and select
Merge when ready again. Do not push changes directly to `main`.

The following GitHub Actions checks are required on both the PR and its queue
entry (Coverage passes as skipped in the queue). Keep their names and the `merge_group` trigger in
[CI](.github/workflows/ci.yml) in sync with the ruleset:

- `Test (ubuntu-latest)`
- `Test (macos-latest)`
- `Test (windows-latest)`
- `Lint`
- `MSRV (1.88)`
- `Docs`
- `Coverage`

The queue starts with one PR building and merging at a time, merge commits,
`ALLGREEN` validation, and a 60-minute check timeout. The minimum group size is
one, so it does not wait to collect a batch. Review approvals are not required.
There are no admin or app bypass actors; emergency changes require an admin to
explicitly edit the ruleset and restore it afterward.

Release PRs use the same queue. Queue merges push to `main`, which starts the
release workflow; release jobs are not queue requirements. See
[the release guide](docs/RELEASE.md) for the token needed to trigger CI on release
PRs.

## Admin: configuring the ruleset

[`.github/rulesets/main.json`](.github/rulesets/main.json) is the reproducible
payload for the active [main merge queue ruleset](https://github.com/rust-works/patchcov/rules/24650034).
Editing this file does not update GitHub automatically. From the repository checkout, an admin can inspect
the current rulesets:

```bash
gh api repos/rust-works/patchcov/rulesets
```

If the ruleset does not exist, create it:

```bash
gh api --method POST repos/rust-works/patchcov/rulesets \
  --input .github/rulesets/main.json
```

To update the existing ruleset, use its numeric ID from the list rather than
creating a duplicate:

```bash
gh api --method PUT repos/rust-works/patchcov/rulesets/RULESET_ID \
  --input .github/rulesets/main.json
gh api repos/rust-works/patchcov/rulesets/RULESET_ID
gh api repos/rust-works/patchcov/rules/branches/main
```

Required checks are pinned to the GitHub Actions app (integration ID `15368`).
Strict up-to-date PR checks are disabled because the queue performs that
validation. The ruleset targets only `refs/heads/main` and does not restrict
release tags or release PR branches. The API format is documented in
[GitHub's ruleset reference](https://docs.github.com/en/rest/repos/rules#create-a-repository-ruleset).

## Admin: smoke test

After enabling or changing the ruleset:

1. Select **Merge when ready** on a reviewed, passing PR, such as a documentation
   change. Confirm it enters the queue.
2. In Actions, find the CI run triggered by `merge_group` and confirm all seven
   required checks pass with the names above (Coverage is skipped; the other six succeed).
3. Confirm the queue merges the PR into `main` with a merge commit.
4. Find the Release workflow triggered by that push. Confirm `Release` and
   `Release PR` succeed and inspect the release PR that release-plz opens or
   updates. A documentation-only change may not require a version bump; a
   successful no-op is distinct from a failed release job. Binary jobs run only
   when a release is created.

Static workflow checks and a ruleset readback cannot prove this end-to-end flow;
record the queue and release run URLs when performing the smoke test.
