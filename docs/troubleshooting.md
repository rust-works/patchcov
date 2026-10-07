# Troubleshooting

Find the symptom, then the cause. Most problems are one of two things: the report's file
names do not line up with git's, or the report and the diff describe different code. Both
leave patchcov with nothing to attribute, which it reports as an empty result rather than as
an error.

A quick way to see what patchcov saw is structured output, which lists every touched file the
reports did not mention:

```bash
patchcov diff --report head.lcov -o json --no-explanation
```

Look at `patch_coverage.total` and `unmeasured_files`. For the flags and fields used below see
the [reference](reference.md).

## Contents

- [Patch coverage is empty, or "No new executable lines"](#patch-coverage-is-empty-or-no-new-executable-lines)
- [No files match the diff](#no-files-match-the-diff)
- [The merge base cannot be resolved](#the-merge-base-cannot-be-resolved)
- [The report was measured at a different revision](#the-report-was-measured-at-a-different-revision)
- [Touched files are reported as absent](#touched-files-are-reported-as-absent)
- [A gate fails and the number looks wrong](#a-gate-fails-and-the-number-looks-wrong)
- [A marker or config error stops the run](#a-marker-or-config-error-stops-the-run)
- [A warning about a shard's workspace root](#a-warning-about-a-shards-workspace-root) (`merge`)

## Patch coverage is empty, or "No new executable lines"

The patch section reads `_No new executable lines added by this diff._` and `patch_coverage`
has `"percent": null, "total": 0`. That is a fact about the join of diff and report, with
four possible causes. Check them in this order:

1. **The diff really added no instrumented code.** A documentation-only change, a deleted
   file, or edits to comments. Confirm with `git diff --stat <base>...HEAD` and, for a code
   change, look at `unmeasured_files`.
2. **The diff is empty because the base is the head.** On the default branch, or any branch
   with no commits beyond the default branch, the default base (the merge base of that branch and
   `HEAD`) is `HEAD` itself. Pass `--base-ref` explicitly, for example `--base-ref HEAD~1` or
   a commit SHA. **Uncommitted changes are not in the diff either**: commit them first.
3. **The paths do not match.** Overall coverage is present but no changed file is found in the
   report. See [No files match the diff](#no-files-match-the-diff).
4. **A filter removed the files.** When `--ignore-filename-regex` removed files the diff added
   code to, the sentence changes to `_No new executable lines in the files measured: N new
   executable lines are in files excluded by ignore-filename-regex._` and `excluded_files`
   says how many. Narrow the regex.

The empty patch is not a failure: `--fail-under-patch` passes when there are no added lines.
If you want a *missing* file to fail, use
[`--fail-on-unmeasured`](usage.md#requiring-touched-files-to-be-measured).

## No files match the diff

patchcov joins report file names to git's repo-relative paths. Nothing matches when the report
names files some other way, and the symptom is patch coverage that is empty or low while the
overall total is fine, plus the changed files listed under "Touched files absent from every
coverage report". When *no* path in a report matches a tracked file, patchcov fails the run
(exit 1) rather than report an empty patch that every gate would pass, naming the report and a
few of the paths it could not match:

```text
Error: coverage report head.lcov: none of its 120 file path(s) matches a tracked file in the
repository; unmatched normalized paths: `/ci/work/app/src/a.rs`, ...; use --strip-prefix or
diff.path-mappings to make paths repo-relative (or pass --allow-path-mismatch / set
diff.allow-path-mismatch to warn instead)
```

Fix the paths with the table below. If a report legitimately matches nothing, pass
`--allow-path-mismatch` or set `diff.allow-path-mismatch: true` in `.patchcov/config.yaml` to
print the same message as a warning and carry on; a gate then cannot see the patch, so use it
sparingly. A single tracked-file match avoids the error, so a report with *some* wrong paths
still needs the comparison below.

`--fail-on-path-mismatch`, which used to opt in to this error, is now the default; it is
still accepted but does nothing and prints a deprecation warning.

Compare the two spellings directly:

```bash
git diff --name-only <base>...HEAD          # what git calls the files
grep '^SF:' head.lcov | head                # lcov
grep -o 'filename="[^"]*"' coverage.xml | head   # Cobertura
```

Then fix the one that applies:

| The report has | Fix |
|----------------|-----|
| Absolute paths from another checkout (`/home/runner/work/app/app/src/a.rs`) | `--strip-prefix /home/runner/work/app/app`, or a `path-mappings` entry with `to: ''` when head and baseline came from different roots |
| Paths relative to a package (`src/app.ts`, measured inside `packages/web`) | `path-mappings`: `from: src`, `to: packages/web/src` |
| A bare source root, or only basenames | `path-mappings` with `from: ''` and `to: <source root>` |
| JaCoCo `com/example/App.java` | `path-mappings`: `from: com/example`, `to: <module>/src/main/java/com/example` |
| Go import paths (`github.com/org/repo/pkg/a.go`) with `go.mod` not at the repository root | `--strip-prefix github.com/org/repo` (the import path of the repository root) |
| Windows paths (`C:\agent\project\src\a.rs`) | a `path-mappings` entry such as `from: 'C:\agent\project'`, `to: ''` |

The full recipes, with examples, are in
[report paths and repository paths](usage.md#report-paths-and-repository-paths). Run from the
repository root, or pass `-C <repo>`: a relative `--report` path is resolved against it.

## The merge base cannot be resolved

Without `--base-ref`, patchcov diffs against the merge base of the default branch and `HEAD`.
The default branch is the first of `refs/remotes/origin/HEAD`, `origin/main`, `main`,
`origin/master`, `master` that exists (see [choosing the base](usage.md#choosing-the-base)).
It fails with one of:

```text
Error: could not resolve a default base ref (tried `refs/remotes/origin/HEAD`, `origin/main`, `main`, `origin/master`, `master`); pass --base-ref
Error: could not compute merge-base of `origin/main` and HEAD
```

Both mean the history needed to find the common ancestor is not in the checkout. The usual
cause is a shallow clone: CI checkouts fetch a single commit by default. The first error can
also mean the default branch has another name and `origin/HEAD` is not set; run
`git remote set-head origin --auto`, or fetch the branch and pass `--base-ref`.

- **GitHub Actions:** check out with full history.

  ```yaml
  - uses: actions/checkout@v4
    with:
      fetch-depth: 0
  ```

- **Other CI, or an existing shallow clone:** `git fetch --unshallow`, and make sure the base
  branch exists locally: `git fetch origin main`.
- **A base that is not `main`** (a release branch, a stacked PR): pass a merge base,
  `--base-ref "$(git merge-base origin/release-1.2 HEAD)"`. A bare `--base-ref origin/release-1.2`
  is compared directly with `HEAD`, so it is only right when `HEAD` already contains that
  branch's tip; see [choosing the base](usage.md#choosing-the-base).
- **You already know the commit:** `--base-ref <sha>` needs no history beyond that commit.

A bad explicit ref fails differently, naming it: `could not resolve base ref ...`.

## The report was measured at a different revision

This is the most common silent mistake. patchcov never reads your source to cross-check the
report; it trusts that the line numbers in the report are the line numbers at `--head-ref`
(default `HEAD`). If the report was measured on other code, the join still runs and produces a
confident, wrong answer: covered lines attributed to the wrong text, new lines reported
uncovered or missing from the report entirely.

Signs:

- Patch coverage is far lower, or higher, than the tests plausibly give.
- Uncovered lines listed are ones you know are exercised, or are blank or comment lines.
- The result changes after you commit, rebase or amend without re-measuring.

Fixes:

- Re-run the instrumented tests on the exact commit (and working tree) you are diffing, then
  run patchcov.
- When diffing a revision other than the checked-out one, measure at that revision and pass
  `--head-ref <rev>` so source markers are also read from it. Without `--head-ref`, markers
  are read from the working tree, which is the code the report describes.
- In CI, take the report and the diff from the same checkout: measure and run `patchcov diff`
  in the same job, or make sure the job that measures checked out the commit `patchcov diff`
  will see as `HEAD`.
- The same applies to `--baseline-report`: it must be measured at the base revision.

## Touched files are reported as absent

The comment has a collapsed note "Touched files absent from every coverage report", and
`unmeasured_files` is non-empty. A changed file appears in no head or baseline report, under
either name if it was renamed. Causes: the file is not code (a README, a config file), the test
target that covers it was skipped, the file is excluded by the coverage tool's own filters, or
the name does not match (see [No files match the diff](#no-files-match-the-diff)).

The note never fails the run by itself. Add `--fail-on-unmeasured 'src/**/*.rs'` (or
`diff.require-measured` in config) to make an absent code file a failure.

## A gate fails and the number looks wrong

- **`--fail-under-lines` fails with "the report has no executable lines".** The report is
  empty, or `--ignore-filename-regex` removed everything. Unlike the patch gate, an unmeasurable
  total fails on purpose.
- **The total differs from `cargo llvm-cov report`.** patchcov counts lines, llvm-cov's summary
  counts per function record. The difference is expected; see
  [why the total differs](explanation.md#why-the-total-differs-from-llvm-covs-summary) and
  choose the threshold against patchcov's figure.
- **Everything was `ignore`d or excluded.** Look for the "ignored region(s)" and "Excluded by
  ignore-filename-regex" notes in the comment, or `markers` and `excluded_files` in JSON.
- **An empty report fails the run before any number appears.** `could not parse coverage report
  ...: coverage report is empty` means the coverage step wrote nothing; fix that step.

## A marker or config error stops the run

patchcov fails loudly rather than letting silenced coverage back in or out unnoticed.

- **`path:line: ... needs a reason`, `unterminated ... region`, `nested coverage region`.** A
  `patchcov: coverage` marker is malformed. Fix the line it names; the syntax is in
  [source markers](usage.md#syntax). `patchcov lint-markers` checks every tracked file without
  a coverage report.
- **Marker errors in files that are not source code.** The bare `patchcov lint-markers` scans
  every tracked text file, so a document that describes the marker syntax in prose is flagged.
  Narrow the scan with `lint-markers.include` in `.patchcov/config.yaml` or `--include`.
- **`could not parse coverage config ./.patchcov/config.yaml: ...`.** The file is not valid for
  its schema (the message names the key, line and column). A misspelled field in a
  `path-mappings` entry is an error; a misspelled top-level key is silently ignored, so if a
  setting seems to do nothing, check the spelling against the
  [config reference](reference.md#patchcovconfigyaml).
- **`invalid ignore-filename-regex pattern`.** The regex from `--ignore-filename-regex` or
  `diff.ignore-filename-regex` does not compile. Remember `(`, `[` and `\` need escaping, and
  that YAML single quotes keep a backslash literal.

## A warning about a shard's workspace root

`patchcov merge` warns when all of a shard's absolute paths are outside `--strip-prefix`:

```text
warning: coverage shard <path>: none of its N absolute file path(s) is under `<root>`, so it was
probably measured under a different workspace root ...
```

Such a shard was probably measured under a different workspace root, so its files would be keyed
under paths that exist nowhere in the repository and would never join the diff. Map the root with
[`--strip-prefix` or `path-mappings`](usage.md#absolute-paths-from-another-runner), or measure
every shard on the same runner image. `patchcov diff` reports the same situation as an
[error](#no-files-match-the-diff). See [sharded runs](usage.md#sharded-runs).
