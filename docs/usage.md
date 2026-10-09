# Usage

How to get patch coverage out of `patchcov`, task by task. Looking something up?

- [Reference](reference.md): every flag, the `.patchcov/config.yaml` keys, the JSON/YAML
  schema, exit codes and environment variables.
- [Explanation](explanation.md): why the numbers come out the way they do.
- [Troubleshooting](troubleshooting.md): an empty result, no files matched, a merge
  base that will not resolve.

`patchcov diff` attributes a per-line coverage report to a git diff and reports
**patch coverage**, the share of the lines a change *added* that are covered by tests,
plus the list of uncovered new lines, per-file project deltas and indirect coverage
changes on unchanged code. It renders the PR coverage comment in CI, and it is a plain
command you can run locally to check a branch before you push.

## Contents

- [What it computes](#what-it-computes)
- [Quick start](#quick-start)
- [Inputs](#inputs)
- [Report paths and repository paths](#report-paths-and-repository-paths)
- [Using with other languages](#using-with-other-languages)
  ([JaCoCo XML](#jacoco-xml), [Go coverprofiles](#go-coverprofiles))
- [Output formats](#output-formats)
- [Gating](#gating)
- [Opt-in branch coverage](#opt-in-branch-coverage)
- [Sharded runs](#sharded-runs) and [merging shards into one file](#merging-shards-into-one-file)
- [Requiring touched files to be measured](#requiring-touched-files-to-be-measured)
- [Excluding files](#excluding-files)
- [Excluding or tolerating regions in source](#excluding-or-tolerating-regions-in-source)
- [CI usage](#ci-usage)

## What it computes

Given a coverage report and a diff, it produces:

- **Patch coverage**: the fraction of *added* lines that are covered. This is the
  headline number, and what `--fail-under-patch` gates on.
- **Overall line coverage**: the project-wide total, which `--fail-under-lines` gates on.
- **Uncovered new lines**: a `file:line` list of added lines with no coverage. With
  `--collapse-ranges`, the markdown per-file table shows ranges such as `9-11`.
- **Per-file project deltas**: each touched file's overall covered-line change
  (requires a `--baseline-report`).
- **Indirect coverage changes**: coverage that flipped on lines the diff never touched
  (also requires a baseline).

Line coverage is the default. Opt into lcov/Cobertura branch-aware line scoring with
[`--branch-coverage`](#opt-in-branch-coverage). For what a rendered comment looks like,
see the [README](../README.md#what-it-prints).

## Quick start

```bash
# 1. Commit your changes, then produce a per-line report for that commit
#    (example: cargo-llvm-cov). The diff is between committed trees.
cargo llvm-cov --no-report      # instrument + run tests
cargo llvm-cov report --lcov --output-path head.lcov

# 2. Attribute it to the diff against the default merge base.
patchcov diff --report head.lcov

# 3. Gate a branch locally: fail if patch coverage is under 80%, or overall
#    line coverage is under 70%.
patchcov diff --report head.lcov --fail-under-patch 80 --fail-under-lines 70

# 4. Full report with project deltas, as JSON for tooling.
patchcov diff \
  --report head.lcov --baseline-report base.lcov \
  -o json
```

**Measure the report at the revision you diff.** The report must come from a test run
on the code at `--head-ref` (default `HEAD`), with no uncommitted changes to files the diff
touches.
A report measured before you added or moved lines gives line numbers that no longer
match the diff, and the result is silently wrong, not an error. If you commit or
rebase after measuring, measure again. See
[the report was measured at a different revision](troubleshooting.md#the-report-was-measured-at-a-different-revision).

## Inputs

- `--report <PATH>` (**required**, repeatable): the head coverage report. Five formats
  are accepted and **auto-detected** from content: lcov trace files, llvm-cov JSON
  (`cargo llvm-cov report --json`), Cobertura XML, JaCoCo XML, and Go coverprofiles
  (`go test -coverprofile`, see [Go coverprofiles](#go-coverprofiles)). Override
  detection with
  `--report-format <auto|lcov|llvm-cov-json|cobertura|jacoco|go-coverprofile>`. Pass it
  once per shard to merge a [sharded run](#sharded-runs), or merge the shards into one
  file first with [`patchcov merge`](#merging-shards-into-one-file).
- `--base-ref <REV>` / `--head-ref <REV>`: the revisions to diff, as committed trees (see
  [choosing the base](#choosing-the-base)). The default base is the merge base of
  the default branch and `HEAD`; the default head is `HEAD`, the revision the report
  was measured at.
- `--baseline-report <PATH>` (+ `--baseline-report-format`): an optional *base-side*
  report. Supplying it enables the project-delta and indirect-change sections; without
  it you still get patch coverage and the uncovered-line list.

### Choosing the base

The default base is a **merge base**: the commit where your branch left the default branch, so
the diff is exactly what your branch added, however far that branch has moved since.

The default branch is the first of these refs that resolves to a commit:

1. `refs/remotes/origin/HEAD`, the remote's default branch, which `git clone` sets
2. `origin/main`, then `main`
3. `origin/master`, then `master`

So a repository whose default branch is `develop` or a release branch needs no flag once
`origin/HEAD` points at it (`git remote set-head origin --auto` sets it). A repository with no
`origin/HEAD` and a `main` or `origin/main` resolves as it always did. Where `origin/HEAD` and
`origin/main` both exist but differ, `origin/HEAD` wins. `origin/HEAD` is a local record that
`git fetch` does not update, so after the remote renames its default branch run
`git remote set-head origin --auto`. A
branch that is not the default (a stacked PR, a long-lived release branch) still needs
`--base-ref`.

An explicit `--base-ref` is *not* turned into a merge base. patchcov compares that revision's
tree directly with the head's, as `git diff <base> <head>` does. `--base-ref origin/main` on a
branch that has fallen behind `main` therefore also counts everything that landed on `main`
since, as lines your branch removed or reverted. Pass a revision that is already the right
base:

```bash
# A branch checkout: the merge base of the PR's base branch and HEAD.
patchcov diff --report head.lcov --base-ref "$(git merge-base origin/main HEAD)"
```

On a CI checkout of the PR's *merge commit* (the `pull_request` default in GitHub Actions),
`HEAD` already contains the base branch's tip, so `--base-ref "origin/${GITHUB_BASE_REF}"` gives
the PR's changes. Both diffs are between committed trees; uncommitted edits are not in
them (only source markers are read from the working tree).

## Report paths and repository paths

Coverage attribution joins report filenames to git's repo-relative paths. A mismatch
can leave patch coverage empty even when overall coverage is present. Check the `SF:`
records (lcov), `filename` attributes (Cobertura), or package and sourcefile names
(JaCoCo) against `git ls-files` before trusting a patch gate.

The path pipeline for `patchcov diff` is:

1. Parse the report. For a native Go coverprofile, strip the module import path from
   the repository root's `go.mod`, if available.
2. Apply explicit `diff.path-mappings`, if configured.
3. Strip `--strip-prefix` (default: the repository working directory), then trim
   leading `./` and `/`.
4. Apply filename exclusions, read source markers, and join to the git diff.

These steps apply to each head shard and the baseline independently. Cobertura
`<sources>` are metadata: the parser uses each class's `filename` and does not choose or
prepend a source root. JaCoCo uses `package/SourceFile.java`, without its module or
`src/main/java` directory. For why the pipeline is ordered this way, see
[the path pipeline](explanation.md#the-path-pipeline).

### Absolute paths from another runner

For reports generated under a different checkout, remove that runner's root:

```bash
patchcov diff --report head.lcov \
  --strip-prefix /home/runner/work/project/project
```

After path normalization, `diff` **fails** when a nonempty head report, shard or baseline has
**no** path that matches a tracked repository file. Such a report cannot be joined to the diff,
so the patch would look empty and every gate would pass without measuring anything. The error
names the report, samples up to three unmatched normalized paths, and suggests `--strip-prefix`
or `diff.path-mappings`. Any single tracked-file match avoids the error, because reports
legitimately name some SDK or vendor files. The check runs before exclusions, so
`--ignore-filename-regex` cannot hide it. An implicit head is checked against the git index, an
explicit `--head-ref` against that revision's tree, and the baseline against the base tree.

If a report legitimately matches nothing, pass `--allow-path-mismatch` (or set
`diff.allow-path-mismatch: true` in `.patchcov/config.yaml`) to get the same message as a
warning on stderr and exit as usual. Prefer fixing the paths: with the opt-out, a gate cannot
see the patch. See [`diff.allow-path-mismatch`](reference.md#diffallow-path-mismatch).

One strip prefix applies to both head and baseline. If they came from different roots,
map each root explicitly instead:

```yaml
# .patchcov/config.yaml
diff:
  path-mappings:
    - from: /ci/head/project
      to: ''
    - from: /ci/base/project
      to: ''
```

### Package-relative and source-root-relative paths

Use explicit directory replacements when a tool omits a directory git retains:

```yaml
# .patchcov/config.yaml: choose the mappings for your report, not all examples
diff:
  path-mappings:
    # nyc invoked inside packages/web: src/app.ts -> packages/web/src/app.ts
    - from: src
      to: packages/web/src
    # JaCoCo: com/example/App.java -> services/api/src/main/java/com/example/App.java
    - from: com/example
      to: services/api/src/main/java/com/example
    # Go module below repo root (no root go.mod to infer this import prefix)
    - from: example.com/project/service
      to: services/api
```

An empty `from: ''` prepends `to` to every relative report filename, useful for a
single source root such as `src/main/java` or a coverage.py report that contains only
basenames. It does not match absolute filenames. An empty `to: ''` removes the matched
prefix. Matches respect directory boundaries (`src` cannot match `src2`), and the
longest prefix wins. Each path is mapped once; replacements are not chained. With
mappings enabled, backslashes become `/` and leading `./` and trailing `/` on prefixes
are removed, allowing Windows runner prefixes such as `from: 'C:\agent\project'`.
Destinations must be repo-relative without `..`; duplicate normalised source prefixes
and misspelled mapping fields fail loudly. Absent or empty mappings leave paths as the
tool wrote them.

Unmatched filenames keep their paths and continue through prefix stripping. Aliases
mapping to one git path combine by maximum line hits. Avoid mapping unrelated files to
the same path: their coverage would be unioned. If two modules both emit `src/App.java`,
a single combined report cannot distinguish them; make filenames unique in each module
report before combining. There is no filesystem search or guessed source-root selection.

Mappings use the same config discovery and override rules as the ignore list. They
belong to `patchcov diff`; `patchcov merge` does not read this block. Merge
repo-relative reports, use its `--strip-prefix` for a common runner root, or normalise
module reports before merging them.

## Using with other languages

The parser depends on the report format, not the source language. Run coverage against
the head revision, produce a report below, then use `patchcov diff --report <report>`
(and optionally a baseline produced at the base revision). Reports must include per-line
records; summary-only JSON, HTML, Clover XML and OpenCover XML are not supported. By
default, branch and function data is ignored. Install and configure each tool in your
project before using these examples.

- **JavaScript / TypeScript: Istanbul/nyc lcov.**
  `npx nyc --reporter=lcov npm test` writes `coverage/lcov.info`. Run from the
  repository root or map package-relative `SF:` paths. For TypeScript, configure
  instrumentation and source maps so the report names original `.ts` files, not emitted
  JavaScript. See [nyc's configuration](https://github.com/istanbuljs/nyc).
- **Python: coverage.py Cobertura.**
  `coverage run -m pytest` followed by `coverage xml -o coverage.xml`.
  Prefer `[run] include = ...` over `source = ...` when you need complete paths;
  `source` can shorten filenames relative to source roots. Use mappings when needed;
  `<sources>` does not add the root automatically. See
  [coverage.py XML reporting](https://coverage.readthedocs.io/en/latest/commands/cmd_xml.html).
- **C / C++: gcovr Cobertura.** Build with GCC `--coverage`, run the tests, then
  `gcovr --root . --cobertura coverage.xml` from the repo root. Set `--root` to the
  repository rather than a build subdirectory; filenames are relative to that root. See
  [gcovr's Cobertura output](https://gcovr.com/en/stable/output/cobertura.html).
- **Swift: llvm-cov lcov or JSON.** Run `swift test --enable-code-coverage`, obtain the
  test binary and profile under the build directory, then run
  `xcrun llvm-cov export <test-binary> -instr-profile=<profile.profdata> -format=lcov > coverage.lcov`.
  Omit `-format=lcov` for llvm-cov JSON; do not use `-summary-only`. Supply the
  instrumented binary for your platform and strip the original checkout prefix. See
  [llvm-cov export](https://www.llvm.org/docs/CommandGuide/llvm-cov.html#llvm-cov-export).
- **Ruby: SimpleCov with simplecov-lcov.** In the test helper, before loading
  application code, require `simplecov` and `simplecov-lcov`, set
  `SimpleCov::Formatter::LcovFormatter.config.report_with_single_file = true`, set
  `SimpleCov.formatter = SimpleCov::Formatter::LcovFormatter`, then call
  `SimpleCov.start`. Run `bundle exec rspec` (or your test runner) and pass the
  generated `.lcov` file under `coverage/lcov/`. The filename can be configured; see
  [simplecov-lcov](https://github.com/fortissimo1997/simplecov-lcov).
- **PHP: PHPUnit Cobertura.**
  `XDEBUG_MODE=coverage vendor/bin/phpunit --coverage-filter src --coverage-cobertura coverage.xml`.
  Enable Xdebug coverage mode or install PCOV, and set the source filter (or its XML
  configuration equivalent). See
  [PHPUnit coverage](https://docs.phpunit.de/en/12.5/code-coverage.html).
- **.NET: coverlet Cobertura.** For VSTest projects with `coverlet.collector`,
  `dotnet test --collect:"XPlat Code Coverage"` writes
  `TestResults/<id>/coverage.cobertura.xml`. Map source-root-relative filenames or runner
  prefixes as needed. Microsoft Testing Platform projects use `coverlet.MTP` and its
  `--coverlet --coverlet-output-format cobertura` options instead. See
  [coverlet integrations](https://github.com/coverlet-coverage/coverlet).
- **Dart: package:coverage lcov.**
  `dart pub global activate coverage` then
  `dart pub global run coverage:test_with_coverage` produces `coverage/lcov.info` from a
  package root. Explicit formatting uses `--lcov` and
  `--packages=.dart_tool/package_config.json` to resolve package URIs to source files;
  map or strip the resulting paths. See [package:coverage](https://pub.dev/packages/coverage).
- **Scala: scoverage Cobertura.** `sbt clean coverage test coverageReport` produces
  `target/scala-<version>/coverage-report/cobertura.xml` in each module. Map
  module/source prefixes as necessary. Do not pass `scoverage.xml`, whose schema is
  different. See [sbt-scoverage](https://github.com/scoverage/sbt-scoverage).
- **Go: native coverprofile.** `go test ./... -coverprofile=coverage.out`; see
  [Go coverprofiles](#go-coverprofiles) for block-to-line semantics and root module
  inference. A nested module needs an explicit import-prefix mapping.
- **Java / Kotlin: native JaCoCo XML.** With the JaCoCo Maven plugin configured,
  `mvn verify` (with its report goal bound) produces `target/site/jacoco/jacoco.xml`;
  with Gradle's JaCoCo plugin, `./gradlew test jacocoTestReport` produces XML only if
  `reports.xml.required = true` is configured. Map the package path to the correct
  module's source root; see [JaCoCo XML](#jacoco-xml).

The [non-Rust fixtures](../tests/fixtures/non-rust/README.md) record producer provenance
and exercise git patch attribution without installing all these toolchains to run the
Rust tests.

### JaCoCo XML

Native JaCoCo reports from Java, Kotlin and Scala are auto-detected by their `<report>`
root, including reports with an XML declaration and JaCoCo DTD. Use
`--report-format jacoco` to select it explicitly. Both `patchcov diff` and
`patchcov merge` accept module reports and aggregate reports with groups. A line with
`ci > 0` is covered, including a partially covered line with both missed and covered
instructions. Instruction counts become boolean hits; branch counters and class/method
summaries do not contribute to line coverage.

Paths are `package/SourceFile.java`, relative to the source root. For example,
`com/example/App.java` may correspond to `src/main/java/com/example/App.java` in git. Use
[path mappings](#package-relative-and-source-root-relative-paths) to prepend that source
root before diff attribution. Overall line coverage and shard merging also work with
source-root-relative paths. Identical package/file paths across modules merge by
covered-line union, so unrelated sources with the same path must be distinguished before
merging.

### Go coverprofiles

`go test -coverprofile=cover.out ./...` writes its own format, read natively with no
`gocover-cobertura` or `gcov2lcov` step. It is detected by its `mode: set`,
`mode: count` or `mode: atomic` header line, or named with
`--report-format go-coverprofile`. Sharded profiles work like any other: repeat
`--report`, or [merge them](#merging-shards-into-one-file) (a `cat` of profiles is also
read, since a repeated `mode:` line is accepted).

```bash
go test -coverprofile=cover.out ./...
patchcov diff --report cover.out
```

A profile records **blocks**, source ranges written
`file:startLine.startCol,endLine.endCol numStmts count`, not per-line hits, so they are
expanded to lines:

- **Every line a block spans takes the block's count.** Where blocks overlap, the larger
  count wins, the rule for any line several regions cover. A line shared by two blocks
  (`} else {`) therefore reads covered if either ran, because the blocks are sub-line
  spans and the model is per line.
- **A line is executable if a block with statements spans it.** The total is the number
  of such lines, so it differs from the statement-weighted percentage of
  `go tool cover -func` (a long multi-statement block counts its lines, not its
  statements). A block with no statements, such as an empty function body, holds nothing
  to execute and is not a line. The reasons the total differs from a tool's own summary
  are the ones under
  [Why the total differs from llvm-cov's summary](explanation.md#why-the-total-differs-from-llvm-covs-summary).
- **Lines between a block's first and last statement all count.** Go starts a block at
  its first statement and ends it at its last, so a comment before the first statement
  or after the last is outside it. The profile does not say which lines in between hold
  code, so a blank or comment line there is an executable line, covered or not with the
  block. `ignore` markers can mask such lines.
- **`set`, `count` and `atomic` read the same way.** Only whether a count is zero matters
  to the output.

Go names files by import path (`github.com/org/repo/pkg/a.go`), not by path on disk. The
module path declared by the `go.mod` at the repository root is stripped from them, which
makes them repo-relative. When the module is **not** at the root (a
`services/api/go.mod`) or there is no `go.mod`, nothing is stripped and no file matches
the diff; pass the import path of the repository root instead, so the subdirectory stays
in the path: `--strip-prefix github.com/org/repo`. `--ignore-filename-regex` sees the
mapped paths. The `go.mod` is the one in the working tree and is applied to
`--baseline-report` too, so a baseline measured before a module rename needs
`--strip-prefix`.

## Output formats

`-o`/`--output` selects the renderer:

- `markdown` (default): the PR-comment layout. The footer can carry CI context via
  `--artifact-url`, `--run-url`, `--base-sha`, `--head-sha` and `--commit-url` (a
  SHA-link prefix); these only affect the rendered links.
- `yaml` / `json`: structured output for scripting and downstream tooling. Every field is
  listed in the [output schema](reference.md#output-schema).

JSON and YAML include an `explanation` block by default, describing the output fields.
Use `--no-explanation` to omit only that block on repeated runs:

```bash
patchcov diff --report head.lcov -o json --no-explanation
patchcov diff --report head.lcov -o yaml --no-explanation
```

All other fields, formatting and coverage gates stay the same. The flag has no effect on
Markdown output. The report is written to stdout; warnings and errors go to stderr.

## Gating

`--fail-under-patch <PCT>` makes the command exit non-zero when patch coverage is below
`<PCT>` percent, so it can fail a CI step or a local pre-push check. Without it the
command only reports and exits zero. `<PCT>` must be a finite number of at least `0`; `nan`,
`inf` and negative values are usage errors (exit `2`), because a threshold that can never be
undercut would disable the gate silently. Write `-inf` as `--fail-under-patch=-inf` to see why
it is refused; with a space, the parser reports an unexpected argument instead. A value above
`100`, such as `150`, is allowed and always fails.

`--fail-under-lines <PCT>` is the overall counterpart: it exits non-zero when line
coverage across the whole head report is below `<PCT>` percent. It gates on the same
`Total` the markdown headline prints, so it moves with the `--ignore-filename-regex` list
and `ignore` markers, exactly as the patch gate does. Both gates can be set together, and
every gate that fails is named in the error. The report is still printed to stdout when a
gate fails, so a CI job can post the comment and then fail. A failed gate exits `1`;
other failures have their own codes, so a script can tell them apart. All exit codes are
in the [reference](reference.md#exit-codes). `--error-format json` prints the cause as a JSON
object on stderr, and each warning, `lint-markers` finding and `merge` summary as a JSON line,
for a wrapper that wants them without parsing text; see [error output](reference.md#error-output).

- **The figure is per-line, not llvm-cov's summary.** It is covered lines over the
  distinct executable lines in the report's per-line records (an lcov's `DA:` records, or
  the JSON export's `segments`), so it differs from
  `cargo llvm-cov report --summary-only`, which counts per *function record* and so
  counts some lines more than once. Pick a threshold against this command's number, not
  against that one. On this repository's own test suite the per-line figure is 99.32%
  where the summary says 99.01% (measured in
  [the explanation](explanation.md#why-the-total-differs-from-llvm-covs-summary)).
  `ignore` markers and `--ignore-filename-regex` move the gate figure further, since
  llvm-cov's summary knows nothing of them.
- **An unmeasurable total fails.** A report with no executable lines, whether empty or
  fully excluded by `--ignore-filename-regex`, fails `--fail-under-lines`. This is
  deliberately unlike `--fail-under-patch`, where "no added lines" is a property of the
  change and passes. A gate that passed on an empty report would let a silently failed
  coverage run through.

Because the gate needs only the report, it works wherever `patchcov diff` does, including
when the report was produced elsewhere (for example, from several CI jobs, see
[Sharded runs](#sharded-runs)) and the profile data is not on the machine running the
gate.

## Opt-in branch coverage

```bash
patchcov diff --report head.lcov --branch-coverage --fail-under-patch 80
```

`--branch-coverage` supports **lcov** `BRDA` and **Cobertura**
`condition-coverage="50% (1/2)"` counts. Other report formats, including a baseline in an
unsupported format, fail explicitly in this mode. The default uses line hits only.

An executable line is covered only if it has hits and every recorded branch ran. A line
with hits but missed branches appears in the existing uncovered list and counts as
uncovered. lcov `-` branch counts count as missed. Lines without branch records keep their
line-hit semantics; branch records do not introduce additional executable lines. This is a
percentage of **lines**, not a percentage of branches: a line with one or many missed
branches counts once in the denominator. Partial lines have no separate label or
missed-count annotation.

The same scoring applies to patch coverage, project totals, file deltas and both
`--fail-under-patch` and `--fail-under-lines`. With `--baseline-report`, an unchanged line
going from fully covered to partially covered is an indirect loss; partial-to-full is an
indirect gain. Partial-to-partial changes stay uncovered on both sides and produce no
indirect change. The usual file scope applies: untouched files require `--all-files`.
Ignore and tolerate markers behave as they do without the flag.

For multiple lcov `--report` shards, a branch is identified by line, block and branch IDs
and counts as covered if any shard ran it. Scoring happens after this union. Cobertura
exposes only aggregate counts, so duplicate records and shards conservatively keep the
largest missed count for each line; complementary coverage cannot be reconstructed from
counts alone. Exact covered/total counts are used rather than rounded percentages;
malformed branch data fails only when branch mode is enabled.

**Use original reports.** `patchcov merge` is line-only and drops branch records, so its
output cannot supply branch evidence for either head or baseline. JaCoCo and llvm-cov
JSON branches are not read. The feature is tested with report fixtures, not by running
`cargo llvm-cov --branch`; no Rust branch-instrumentation toolchain has been validated for
it.

## Sharded runs

When the instrumented test run is split across several concurrent CI jobs, each job
produces its own report. Pass them all to one aggregation step, one `--report` each, and
everything (the patch gate, `--fail-under-lines`, the deltas, the PR comment) is computed
over the merged result:

```bash
patchcov diff \
  --report shard-1.lcov --report shard-2.lcov --report shard-3.lcov \
  --baseline-report base.lcov \
  --fail-under-patch 80 --fail-under-lines 70
```

**How shards combine.** The merge is a *union*: the files are unioned, and for a line
present in several shards the larger hit count wins. In practice:

- A line **any** shard covered is covered, which is the point of sharding.
- A line only **one** shard instrumented is judged by that shard alone: it is neither
  covered nor uncovered "elsewhere". With a normal run every shard instruments the same
  code and lists every line, so this only matters when shards are built from different
  configurations.
- Hit counts are **maxed, not summed**. Nothing in the line-coverage output reads the
  count itself, so a total across shards is not available.
- Shard order never changes the output, and each shard may be in a different format (they
  are detected independently; an explicit `--report-format` applies to all of them).

**A bad shard cannot lower coverage unnoticed.** Merging ignores a shard that contributes
nothing, so a shard whose job failed would otherwise just make the result look slightly
worse. With more than one `--report`:

- a shard that is missing, unparseable, or has **no executable lines** fails the run,
  naming the shard (checked before `--ignore-filename-regex`, so a shard that only covers
  excluded files is not mistaken for a failed one);
- a shard whose normalized paths **all** fail to match tracked repository files fails the
  run: it was probably measured under a different workspace root, and its files would key
  under paths that exist nowhere in the repository. A shard with some in-tree paths is left
  alone, because reports legitimately name a few out-of-tree files (the standard library,
  vendored sources).

The tracked-path check also applies to a single `--report` and to the baseline, and
`--allow-path-mismatch` turns its error into a warning (see
[report paths](#report-paths-and-repository-paths)). The empty-shard check is specific to runs
with more than one `--report`.

**Limits.**

- `--strip-prefix` is one value, so every shard has to share a workspace root. Shards from
  the same CI runner image do; a mix of, say, Linux and macOS runners does not, and the
  path-mismatch error above is the signal.
- `--baseline-report` takes **one** report, so a baseline from a sharded run must be a
  single file. Make it with [`patchcov merge`](#merging-shards-into-one-file).
- Recomputing the baseline at the merge base, and the `codecov.json` /
  `coverage-summary.txt` files, belong to the
  [GitHub Action](../README.md#github-action) and `cargo llvm-cov`, not to this command.

## Merging shards into one file

`patchcov diff --report` merges shards on the fly, but a single file is the contract in
some places: `--baseline-report` takes one report, a baseline publish uploads one file,
and so does a codecov upload. `patchcov merge` writes that file:

```bash
patchcov merge shard-1.lcov shard-2.lcov shard-3.lcov -o merged.lcov

# The merged file is an ordinary report, and the baseline a sharded run could not give.
patchcov diff --report head.lcov --baseline-report merged.lcov
```

- **Same merge, same checks.** The files are unioned and, for a line present in several
  inputs, the larger hit count wins, exactly as for repeated `--report` (see
  [how shards combine](#sharded-runs)). Each input may be lcov, llvm-cov JSON, Cobertura,
  JaCoCo or a Go coverprofile, detected per file; `--report-format` applies to all of them.
  An input that is missing, unparseable or has no executable lines fails the run and names
  it, **including a lone input**, unlike `patchcov diff`, because the output is trusted by
  whatever reads it next. An input whose absolute paths all fall outside the strip prefix
  draws a warning on stderr; the file is still written, so check stderr when the merged
  file is published unattended. Both commands judge a shard with the same function, so the
  checks cannot drift apart.
- **Same total.** Reading the merged file as a single `--report` gives the same total,
  patch coverage and rendered output as passing the shards.
- **Deterministic.** The output is lcov with files sorted by path, each file's `DA`
  records sorted by line, and a trailing newline, so it is byte-identical for any order of
  the inputs.
- **Line coverage only.** The file carries `TN`, `SF`, `DA`, `LF`, `LH` and
  `end_of_record`. Function (`FN*`) and branch (`BRDA`) records are dropped, which is fine
  for `patchcov diff` and for a codecov line view but not for a consumer that wants
  branches. `LF`/`LH` count the `DA` records, so unlike `llvm-cov`'s own lcov the file
  agrees with itself (see [why the total differs](explanation.md#why-the-total-differs-from-llvm-covs-summary)).
  A file with no executable lines is kept as an empty record, so the text reads back as
  exactly the merged report. A path with a line break or leading or trailing whitespace,
  which lcov cannot carry, fails the merge.
- **Repo-relative paths.** Paths are written with the repository working directory
  stripped (found from `-C/--repo` or the current directory), or with
  `--strip-prefix <PATH>` when the shards were measured under another root. The file
  therefore reads back the same on any runner. A shard measured under another root would
  otherwise key the same file under a second path and never union with the others, which
  is what the shard warning under [Sharded runs](#sharded-runs) is for. Outside a
  repository, and with no `--strip-prefix`, paths are written as the reports have them. A
  path outside the prefix is normalised as `patchcov diff` normalises it (a leading `./`
  or `/` is dropped), so a standard-library path such as `/rustc/<hash>/library/...` comes
  out without its leading `/`. If the repository exists but cannot be opened (say, a
  checkout owned by another user), the merge fails and asks for `--strip-prefix` rather
  than writing absolute paths.
- **Atomic.** Every input is read and checked before anything is written, and the file is
  replaced through a temporary file and a rename, through a symlink if the path is one and
  keeping an existing file's mode: a merge that fails creates nothing and leaves an
  existing file as it was, and `-o` may name one of the inputs. `-o` is a **path** here,
  whereas `patchcov diff -o` selects an output format, so a bare `markdown`, `yaml`, `json`
  or `lcov` is refused as a probable slip (write `./json` for a file of that name).
- **Not applied: filters.** `--ignore-filename-regex`, `.patchcov/config.yaml` and `ignore`
  markers are applied by `patchcov diff`, to the head and baseline alike, so the merged
  file stays a complete record.

**Do not join lcov files with `cat`.** `cargo llvm-cov` writes no newline after its final
`end_of_record`, so `cat shard-*.lcov` glues one shard's last record onto the next shard's
`SF:` line. `patchcov` happens to read that file, but other lcov consumers can drop or
misattribute the next file's lines, and a join has no empty-shard check either: a failed
shard just makes the total look slightly worse. `patchcov merge` has neither problem.

## Requiring touched files to be measured

A report can omit a changed file entirely, for example when a crate or test target was
skipped. Patch coverage alone cannot distinguish this from a documentation change. Markdown
lists these paths in a collapsed "Touched files absent from every coverage report" note.
JSON and YAML expose a sorted `unmeasured_files` array (empty when every eligible file was
reported).

Opt in to failure for code paths with repeatable globs:

```sh
patchcov diff --report coverage.lcov --fail-under-patch 80 \
  --fail-on-unmeasured 'src/**/*.rs' --fail-on-unmeasured 'lib/**/*.py'
```

Or persist the policy in `.patchcov/config.yaml`:

```yaml
diff:
  require-measured:
    - 'src/**/*.rs'
    - 'lib/**/*.py'
```

CLI and config globs are unioned. They match complete repo-relative paths: `*` stays
within a directory, while `**` traverses directories (`src/**/*.rs` also matches
`src/a.rs`). Invalid or empty globs are errors. With no globs, missing files leave exit
codes and coverage gates unchanged.

Presence in any head shard or baseline report counts as measured, under either name of a
renamed file, even when its entry has no executable lines. Deleted files and paths
explicitly excluded by `ignore-filename-regex` under either rename name are omitted. Source
ignore markers do not turn a reported file into an unmeasured file. This policy checks
report presence; it does not prove every executable line was instrumented, or require that
baseline-only files appear in the head report.

## Excluding files

Some source files have coverage that is **inherently non-deterministic across runs**: a
region gated on a *runtime* CPU-feature check (`is_x86_feature_detected!`, runtime SIMD-level
dispatch) is compiled and counted in the denominator but only *executed* on a host whose CPU
has the instruction. When the baseline and head runs draw different runner CPUs, the file's
coverage swings with no source change, surfacing as phantom deltas or in the "unchanged files
also moved" note.

`--ignore-filename-regex <REGEX>` excludes files whose repo-relative path matches any of the
given regexes from **both** the head and baseline reports before the diff. Filtering both
sides symmetrically keeps the total, per-file deltas, patch coverage, indirect-change list
and the `--fail-under-patch` gate computed over the same denominator, so an excluded file
can never produce a spurious "moved" entry. Matching is **unanchored** (partial), the same
semantics as `cargo llvm-cov --ignore-filename-regex`, and is applied **after**
`--strip-prefix` normalisation, so patterns match the repo-relative path. The flag is
repeatable and comma-separated; an empty pattern is treated as a no-op (a bare empty regex
would match every path).

### The comment says what the filter excluded

A filter leaves nothing in the numbers: a smaller total and an empty patch look exactly like
a diff that added no code. So whenever the filter removed at least one file from a report,
the comment says so:

```text
_Excluded by ignore-filename-regex: 3 files (1 of them touched by this diff, adding 40 executable lines)._
```

The clause after the touched count appears when the diff added executable lines to excluded
files: those are lines the filter took out of the patch denominator, so a mixed diff is
visible even though it still shows a patch percentage. The files the diff touched are listed
beneath it (the first 20; the rest are counted), and `-o json`/`-o yaml` carry an
[`excluded_files`](reference.md#excluded_files) object. Only files that were *in a report*
count, as a head or a baseline entry, once each; a path matching the regex that no report
mentioned was never measured.

The empty-patch sentence tells its two causes apart. `_No new executable lines added by this
diff._` means the diff added no instrumented code. When the filter removed files in which the
diff added executable lines, it reads `_No new executable lines in the files measured: 3 new
executable lines are in files excluded by ignore-filename-regex._` instead. Nothing is shown
when the filter removed nothing. The gates are unchanged: a patch with no measured lines
still passes `--fail-under-patch`.

### Declaring the ignore list in repo config

Because these files are CPU-conditional forever, a property of the *repository* and not of a
single command line, the ignore list can be declared once in version control under a
`.patchcov/` directory at the repository root. Create `.patchcov/config.yaml`:

```yaml
# .patchcov/config.yaml
diff:
  # repo-relative path regexes; same unanchored semantics as
  # --ignore-filename-regex, applied after --strip-prefix normalisation
  ignore-filename-regex:
    - 'src/bits/popcount\.rs'   # AVX-512 VPOPCNTDQ path is CPU-gated
    - 'src/dsv/simd/.*'         # runtime SIMD dispatch fallback arms
    - 'src/yaml/simd/.*'
```

- **Discovery** is the same for every command: `--config-dir <PATH>` wins, else
  `PATCHCOV_CONFIG_DIR`, else the nearest `.patchcov/` found walking up from the repo root
  (never past the repository boundary). There is no user-level or machine-level file: a
  setting that changes what a coverage gate reports belongs in version control, where a
  reviewer can see it.
- **Union, not replacement.** The config list is set-unioned with any
  `--ignore-filename-regex` passed on the command line, so the flag still works and only
  ever *adds* to the config list.
- **Empty or missing is a no-op.** Behavior is unchanged when the file is absent or its list
  is empty. A present-but-malformed `config.yaml`, or an invalid regex from either source,
  is a hard error (it fails loudly rather than silently letting the excluded noise back in).
  Unknown keys are ignored, so the schema can grow without breaking older binaries.

This is the recommended path when patchcov runs **through a wrapper** (a CI action, a task
runner) that does not thread the flag through: patchcov reads `.patchcov/config.yaml`
directly from the checkout, so no wrapper change is needed. Every key is in the
[config reference](reference.md#patchcovconfigyaml).

## Excluding or tolerating regions in source

Excluding a whole file is usually too blunt. The noise is typically **one function**: a
CPU-gated dispatch helper is ten lines inside a two-hundred-line file, and ignoring the file
hides far more real coverage than noise.

Config cannot name the region either: line numbers are invalidated by every edit above
them, and function *extents* are absent from the lcov `FN:` records (they carry a start line
and a mangled symbol, nothing more). So the region is delimited **in the source**, with a
comment that travels with the code:

```rust
// patchcov: coverage tolerate reason="CPU-gated: the avx512f arm only executes on Zen 4+ runners"
fn detect_fast_bmi2() -> bool {
    if is_x86_feature_detected!("avx512f") {
        return true;
    }
    cpuid_amd_zen3_or_later()
}
// patchcov: coverage end
```

patchcov scans **each revision's own source**, head from the working tree and base from the
base revision's blob, so no line number is ever recorded, and a region that moves, grows, or
disappears between base and head is handled by construction.

### The two kinds

| Kind       | Effect on the percentages (total, per-file, patch)  | Effect on the delta signals (headline Δ, per-file Δ, notable, indirect) |
|------------|-----------------------------------------------------|-------------------------------------------------------------------------|
| `ignore`   | lines removed from **both** reports before analysis | none: the lines no longer exist                                         |
| `tolerate` | lines kept; the reported coverage stays honest      | flips **masked**: each tolerated head line is scored with its baseline hit status |

`ignore` is the scoped twin of `ignore-filename-regex`. `tolerate` keeps the number
truthful while the noise stops moving the needle.

**Prefer `tolerate`.** Reach for `ignore` only when the code is genuinely unreachable on CI
and counting it in the denominator is simply wrong. A tolerated region still shows its real
coverage in the total and in the per-file table; it just cannot manufacture a delta. How the
masking works is in [the explanation](explanation.md#how-tolerate-masking-works).

### Syntax

```text
patchcov: coverage ignore reason="…"     … patchcov: coverage end
patchcov: coverage tolerate reason="…"   … patchcov: coverage end
patchcov: coverage ignore-line reason="…"
patchcov: coverage tolerate-line reason="…"
```

- **Matching is a plain substring** anywhere on a line, so any comment syntax works (`//`,
  `#`, `--`, `<!-- -->`). Nothing parses the host language, which also means a marker inside
  a string literal *is* matched.
- **Both marker lines are inside the region.** They are comments, so they are never
  executable and never appear in a report.
- **`reason="…"` is mandatory** on every region start and single-line marker. Silencing has
  to be explained at the site.
- **Malformed markers are hard errors** naming `path:line`: a missing or empty reason, an
  unterminated quote, a nested region, an `end` with no start, or a region left open at
  end-of-file. An unterminated region is never widened silently to EOF, because that would
  silence an unbounded amount of code nobody looked at.
- **Markers are never invisible.** Whenever a marker applies, the markdown comment gains a
  collapsed note listing each region's path, kind, line span as observed at head, and
  reason; the YAML and JSON views carry the same information in a
  [`markers`](reference.md#markers) array.
- **File-level exclusion wins.** A file excluded by `--ignore-filename-regex` or
  `config.yaml` is never read, so markers inside it are never scanned, and never report
  errors.
- **Without a baseline**, `ignore` still applies (it shapes the total and the patch) while
  `tolerate` is a no-op, since there are no deltas to mask.
- **Added lines inside a `tolerate` region stay in the patch-coverage denominator.** New code
  should still be tested even if it flaps later. Added lines inside an `ignore` region leave
  it, because they are gone from the head report entirely.

### Check marker syntax locally

```bash
patchcov lint-markers
patchcov lint-markers src/bits/popcount.rs
patchcov lint-markers -C /path/to/repo src/bits/popcount.rs
patchcov lint-markers --include 'src/**/*.py' --include 'ci/*.sh'
```

With no paths, the command checks every tracked text file in the Git index against its
current working-tree contents, including staged additions and unstaged edits, in any
language, since a marker is a plain substring in any comment syntax. Deleted worktree files
are skipped, as are symlinks and submodules, and so is any file that is not valid UTF-8:
binaries are never flagged. Explicit paths select only those files, including untracked
files, and are relative to the repository root unless absolute; a non-UTF-8 file named
explicitly is skipped too, so a hook can pass every staged path. Malformed markers print the
same `path:line` diagnostics as `patchcov diff` and make the command exit nonzero. No
coverage report or `llvm-cov` run is needed, so it suits a pre-commit hook or a CI step.

To narrow the default scan, pass `--include <GLOB>` (repeatable) or declare the same list in
`.patchcov/config.yaml`:

```yaml
# .patchcov/config.yaml
lint-markers:
  include:
    - 'src/**/*.py'
    - '**/*.rs'
```

- **Globs** match the repo-relative, `/`-separated path. `*` stays within one path component
  and `**` crosses directories, so `*.py` is a top-level file and `**/*.py` is any. A file
  is scanned when it matches *any* glob; an invalid glob is a hard error.
- **The flag replaces the config list** rather than adding to it (unlike
  `diff.ignore-filename-regex`, which only ever grows), so a one-off `--include` can narrow
  a repo-wide list. With neither, every tracked text file is scanned.
- **Explicit paths ignore both**: naming a file is a selection, so `--include` and the config
  list apply only to the default scan.
- Discovery of `config.yaml` is the one `patchcov diff` uses (`--config-dir`, else
  `PATCHCOV_CONFIG_DIR`, else a walk-up for `.patchcov/`). A malformed file is a hard error,
  but a misspelled key is ignored, like every unknown key. A glob that matches no tracked
  file scans nothing and prints a warning.

A repository that documents the marker syntax in prose (in Markdown, say) will see errors
when the bare command scans those files. This repository does: its
[`.patchcov/config.yaml`](../.patchcov/config.yaml) sets `lint-markers.include` to
`['**/*.rs']`, so `patchcov lint-markers` passes here. Do the same, or use
`--include '**/*.rs'`.

## CI usage

A pull-request job needs three steps:

1. Measure the head commit and write a report (`head.lcov` above).
2. Measure the merge base the same way, for project deltas and indirect changes
   (`base.lcov`). This is optional: patch coverage needs only the head report.
3. Run `patchcov diff` and post its markdown as the PR comment, failing the job on the gates:

```bash
# --base-ref as below is right on a pull_request merge-commit checkout; on a branch
# checkout use --base-ref "$(git merge-base "origin/${GITHUB_BASE_REF}" HEAD)".
patchcov diff \
  --report head.lcov --baseline-report base.lcov \
  --base-ref "origin/${GITHUB_BASE_REF}" \
  --run-url "${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}" \
  --fail-under-patch 80 > coverage-comment.md
```

The markdown is written to stdout, and exit status `1` means a gate failed, so the comment can
still be posted then: capture the status, post the file if it is `0` or `1`, then exit with
it. Any other non-zero status (`2` to `8`, see the
[exit codes](reference.md#exit-codes)) means no report was produced, so there is nothing to
post. A
sharded run passes one `--report` per shard, or merges them first with
[`patchcov merge`](#merging-shards-into-one-file). The merge base has to resolve, so check
out with full history (`fetch-depth: 0`); see
[the merge base cannot be resolved](troubleshooting.md#the-merge-base-cannot-be-resolved).
[action-works/patchcov-action](https://github.com/action-works/patchcov-action) does all of
this for you.
