# Reference

Exact flags, config keys, output fields, exit codes and environment variables. For how to
do a task see [usage](usage.md); for the reasoning behind the numbers see
[explanation](explanation.md). `patchcov <command> --help` prints the same flags from the
binary you have.

## Contents

- [Commands](#commands)
- [`patchcov diff` flags](#patchcov-diff-flags)
- [`patchcov merge` flags](#patchcov-merge-flags)
- [`patchcov lint-markers` flags](#patchcov-lint-markers-flags)
- [Exit codes](#exit-codes)
- [Output schema](#output-schema) (JSON and YAML)
- [`.patchcov/config.yaml`](#patchcovconfigyaml)
- [Environment variables](#environment-variables)

## Commands

| Command                  | Purpose                                                                 |
|--------------------------|-------------------------------------------------------------------------|
| `patchcov diff`          | Attribute a coverage report to a git diff; report and gate on it        |
| `patchcov merge`         | Merge the per-shard reports of a sharded run into one lcov file         |
| `patchcov lint-markers`  | Check `patchcov: coverage` source markers without a coverage report     |

`-C, --repo <PATH>` is accepted by every command, before or after the subcommand. It runs
patchcov as if started in `<PATH>`, like `git -C`. `-h/--help` and `-V/--version` work
everywhere.

## `patchcov diff` flags

| Flag | Purpose |
|------|---------|
| `--report <PATH>` | Head coverage report (**required**); repeat once per shard to merge a [sharded run](usage.md#sharded-runs) |
| `--report-format <FMT>` | Format of every `--report`: `auto` (default) \| `lcov` \| `llvm-cov-json` \| `cobertura` \| `jacoco` \| `go-coverprofile` |
| `--base-ref <REV>` | Base revision (default: merge base of the [default branch](usage.md#choosing-the-base) and `HEAD`). An explicit revision is compared directly with the head, **not** via a merge base; see [choosing the base](usage.md#choosing-the-base) |
| `--head-ref <REV>` | Head revision the report was measured at (default: `HEAD`) |
| `--baseline-report <PATH>` | Base-side report; enables project deltas and indirect changes. Takes one report |
| `--baseline-report-format <FMT>` | Format of `--baseline-report` (auto-detected by default); same values as `--report-format` |
| `-o, --output <FMT>` | `markdown` (default) \| `yaml` \| `json` |
| `--no-explanation` | Omit the `explanation` block from JSON/YAML output; no effect on markdown |
| `--fail-under-patch <PCT>` | Exit non-zero when patch coverage is below `<PCT>`. No added lines passes |
| `--fail-under-lines <PCT>` | Exit non-zero when overall line coverage is below `<PCT>`, or the report has no executable lines |
| `--fail-on-unmeasured <GLOB>` | Exit non-zero for matching touched files absent from every report (repeatable; unioned with `diff.require-measured`) |
| `--branch-coverage` | Score lines with missed lcov/Cobertura branches as uncovered, in head and baseline |
| `--collapse-ranges` | Collapse consecutive uncovered new lines into ranges (`9-11`) in the markdown per-file table; JSON and YAML always list single lines |
| `--all-files` | Report deltas and indirect changes for all files, not just touched ones |
| `--allow-path-mismatch` | Warn, instead of failing, when a nonempty head, shard or baseline report has no path matching a tracked file after normalization. Off by default: such a report fails. Unioned with `diff.allow-path-mismatch` |
| `--strip-prefix <PATH>` | Prefix stripped from report paths to make them repo-relative (default: the repository working directory) |
| `--ignore-filename-regex <REGEX>` | Exclude matching files from both reports (repeatable or comma-separated); unioned with `diff.ignore-filename-regex` |
| `--config-dir <PATH>` | Directory searched for `config.yaml` (default: the discovered `.patchcov/`, honouring `PATCHCOV_CONFIG_DIR`) |
| `--artifact-url <URL>` | Link to the full coverage-summary artifact, in the markdown footer |
| `--run-url <URL>` | Link to the CI run, in the markdown footer |
| `--commit-url <URL>` | Commit-URL prefix for linking SHAs |
| `--base-sha <SHA>` / `--head-sha <SHA>` | SHAs shown in the markdown `Comparing` line (both are needed for the line to appear) |

The five footer and link flags fall back to environment variables; see
[Environment variables](#environment-variables). `--format` is a hidden, deprecated alias
of `-o/--output` that prints a warning. `--fail-on-path-mismatch` is a hidden, deprecated no-op
(failing is now the default) that prints a warning; `--allow-path-mismatch` still wins over it.

## `patchcov merge` flags

| Flag | Purpose |
|------|---------|
| `<REPORT>...` | Reports to merge, one per shard (**required**); lcov, llvm-cov JSON, Cobertura, JaCoCo or Go coverprofile |
| `--report-format <FMT>` | Format of every report: `auto` (default) \| `lcov` \| `llvm-cov-json` \| `cobertura` \| `jacoco` \| `go-coverprofile` |
| `-o, --output <PATH>` | File to write the merged lcov report to (**required**); a path, not a format |
| `--strip-prefix <PATH>` | Prefix stripped from report paths (default: the repository working directory, if there is one) |

## `patchcov lint-markers` flags

| Flag | Purpose |
|------|---------|
| `[PATH]...` | Files to check (default: every tracked text file); an explicit path ignores `--include` and the config list |
| `--include <GLOB>` | Check only tracked files matching this glob (repeatable); replaces `lint-markers.include` |

## Exit codes

Each class of failure has its own exit code, so a script can tell a failed gate from an
unreadable report without parsing stderr. The message on stderr is unchanged (its last line
starts with `Error:`). The report itself, on stdout, is still printed when a *gate* fails, so
a CI job can post it first.

| Code | Meaning | Examples (stderr) |
|-----:|---------|-------------------|
| `0` | Success. The report was printed and no gate failed. Warnings on stderr (a path mismatch you allowed, a lint glob that matched nothing) do not change it | |
| `1` | A **gate failed** (`diff`) | `patch coverage 61.11% is below the --fail-under-patch threshold of 80.00%`<br>`line coverage 75.00% is below the --fail-under-lines threshold of 100.00%`<br>`the report has no executable lines, so the --fail-under-lines threshold of 50.00% cannot be met`<br>`touched files absent from every coverage report (--fail-on-unmeasured / diff.require-measured): src/a.rs`. Every failed gate is named, joined by `;` |
| `2` | **Usage error**: reported by the argument parser before anything runs, or a combination of flags it cannot check | `the following required arguments were not provided: --report <PATH>`<br>`invalid value 'abc' for '--fail-under-patch <PCT>'`<br>`unrecognized subcommand`<br>`--branch-coverage supports only lcov and Cobertura reports: ./go.cover`<br>`-o/--output is the file to write, but `json` looks like an output format ...` (`merge`) |
| `3` | A **report is unreadable, empty or unparseable** (`diff`, `merge`) | `could not read coverage report ./missing.lcov: No such file or directory`<br>`could not parse coverage report ./empty.lcov: coverage report format auto-detection failed: coverage report is empty; cannot detect format`<br>`coverage shard <path> has no executable lines` (several `--report`, or any `merge` input) |
| `4` | A **source marker is malformed** (`lint-markers`, and `diff` for files in the report) | ``src/m.rs:1: `patchcov: coverage ignore` needs a reason (write ...)``<br>`coverage marker lint failed` |
| `5` | **Config is bad** | `could not parse coverage config ./.patchcov/config.yaml: ...` (malformed YAML, wrong type, a misspelled `path-mappings` field)<br>`coverage path-mappings destination must be repo-relative without '..': ...`<br>`invalid ignore-filename-regex pattern`<br>`invalid glob ... in --fail-on-unmeasured / diff.require-measured` |
| `6` | **Git cannot answer** | `could not open git repository at ...`<br>`could not resolve base ref ...`<br>``could not resolve a default base ref (tried `refs/remotes/origin/HEAD`, `origin/main`, `main`, `origin/master`, `master`); pass --base-ref``<br>``could not compute merge-base of `<ref>` and HEAD`` |
| `7` | A **path mismatch** (`diff`): a nonempty report has no path matching a tracked file. With `--allow-path-mismatch` or `diff.allow-path-mismatch` it is only a warning | `coverage report <path>: none of its N file path(s) matches a tracked file in the repository; unmatched normalized paths: ...; use --strip-prefix or diff.path-mappings to make paths repo-relative (or pass --allow-path-mismatch / set diff.allow-path-mismatch to warn instead)` |
| `8` | **Any other runtime error** | `could not write merged report to ./out/merged.lcov: No such file or directory`<br>`could not read <path> to scan for coverage markers` |

When one failure fits two rows, the most specific one wins: a config file that cannot be read
is `5`, not `8`. A failure that no row names also exits `8`. A process killed by a signal
exits with the shell's `128 + signal`, as usual.

Up to and including 0.3.0, every runtime error, gate or not, exited `1`. A script that treated `1` as "a
gate failed" keeps working. One that tested for `1` to mean "anything went wrong" should test
for a non-zero status instead.

```bash
patchcov diff --report head.lcov --fail-under-patch 80 > coverage-comment.md
case $? in
  0) ;;                                       # passed
  1) echo "coverage gate failed" ;;           # post the comment, then fail the job
  *) echo "patchcov could not run" >&2 ;;     # no usable report; do not post
esac
```

## Output schema

`patchcov diff -o json` and `-o yaml` emit the same structure. Percentages are on a 0 to 100
scale and rounded to two decimal places. `null` (`~` in YAML) means "not measurable", for
example a percentage over zero lines. Field order follows the table below.

The example is the output of [`docs/examples/sample.sh json`](examples/sample.sh), a small
repository where a patch adds 18 executable lines and 7 are uncovered, trimmed to the
interesting parts:

```json
{
  "patch_coverage": {
    "percent": 61.11,
    "covered": 11,
    "total": 18,
    "files": [
      {
        "path": "src/cache.rs",
        "percent": 66.67,
        "covered": 4,
        "total": 6,
        "uncovered_lines": [5, 6]
      }
    ]
  },
  "uncovered_new_lines": ["src/cache.rs:5", "src/cache.rs:6"],
  "unmeasured_files": [],
  "project_delta": {
    "total_before": 100.0,
    "total_after": 85.42,
    "files": [
      { "path": "src/cache.rs", "before": null, "after": 66.67, "delta": null }
    ]
  },
  "indirect_changes": { "newly_covered": 0, "newly_uncovered": 0, "lines": [] }
}
```

(The real output lists both files and all seven uncovered lines, and spreads arrays over
several lines.)

### Top level

| Field | Type | Present | Meaning |
|-------|------|---------|---------|
| `explanation` | object | by default; omitted by `--no-explanation` | A description of each field, and whether it is populated in this document. See [`explanation`](#explanation) |
| `patch_coverage` | object | always | Coverage of the lines this diff added. See [`patch_coverage`](#patch_coverage) |
| `uncovered_new_lines` | string[] | when non-empty | The added lines that are not covered, as `path:line`, in path then line order |
| `unmeasured_files` | string[] | always (may be empty) | Sorted repo-relative paths of non-deleted files the diff touched that appear under neither name in any head or baseline report. Files excluded by `--ignore-filename-regex` are omitted. May be documentation or uninstrumented code; only `--fail-on-unmeasured` / `diff.require-measured` globs turn a match into a failure |
| `project_delta` | object | only with `--baseline-report` | Project coverage before and after. See [`project_delta`](#project_delta) |
| `indirect_changes` | object | only with `--baseline-report` | Lines whose coverage flipped without their content changing. See [`indirect_changes`](#indirect_changes) |
| `markers` | object[] | when a source marker applied | The silenced regions. See [`markers`](#markers) |
| `excluded_files` | object | when `--ignore-filename-regex` removed a file | What the filter removed. See [`excluded_files`](#excluded_files) |

### `patch_coverage`

| Field | Type | Meaning |
|-------|------|---------|
| `percent` | number or null | Covered over total added executable lines. `null` when `total` is `0` (the diff added no instrumented code) |
| `covered` | integer | Added executable lines that are covered |
| `total` | integer | Added executable lines: the patch-coverage denominator |
| `files` | object[] | One entry per file that added executable lines, in path order |
| `files[].path` | string | Repo-relative path |
| `files[].percent` / `covered` / `total` | as above | The same three figures for this file |
| `files[].uncovered_lines` | integer[] | Uncovered added line numbers; omitted when the file has none |

### `project_delta`

| Field | Type | Meaning |
|-------|------|---------|
| `total_before` | number or null | Overall line coverage of the baseline |
| `total_after` | number or null | Overall line coverage of the head: the real, measured value. This is what `--fail-under-lines` gates on |
| `total_after_effective` | number | Head coverage with [`tolerate`](explanation.md#how-tolerate-masking-works) masking applied. Present only when masking changed the total |
| `files` | object[] | Touched files (all files with `--all-files`) whose coverage is listed |
| `files[].path` | string | Repo-relative path |
| `files[].before` | number or null | The file's baseline coverage; `null` for a new file |
| `files[].after` | number or null | The file's real head coverage |
| `files[].after_effective` | number | Masked head coverage; present only when masking changed it, in which case `delta` is `after_effective - before` |
| `files[].delta` | number or null | Change in percentage points; `null` for a new file |
| `notable_unchanged` | object[] | Files the diff did *not* touch that still moved by at least 10 covered lines, with the same fields as `files[]`. Not attributable to this diff. Omitted when empty |

### `indirect_changes`

| Field | Type | Meaning |
|-------|------|---------|
| `newly_covered` | integer | Unchanged lines that went from uncovered to covered |
| `newly_uncovered` | integer | Unchanged lines that went from covered to uncovered |
| `lines` | object[] | One entry per flipped line |
| `lines[].path` | string | Repo-relative path (at head) |
| `lines[].head_line` | integer | The line number at head |
| `lines[].base_line` | integer | The same line's number at base |
| `lines[].transition` | string | `uncovered_to_covered` or `covered_to_uncovered` |

### `markers`

One object per region a `patchcov: coverage` marker silenced, so a reviewer can see what was
silenced and why. See [source markers](usage.md#excluding-or-tolerating-regions-in-source).

| Field | Type | Meaning |
|-------|------|---------|
| `path` | string | Repo-relative path of the marked file |
| `kind` | string | `ignore` (lines removed from both reports) or `tolerate` (lines kept, flips masked) |
| `side` | string | Where the region was observed: `both`, `head` or `base` |
| `start`, `end` | integer | First and last line of the region, 1-based and inclusive, as observed on that side |
| `reason` | string | The marker's mandatory `reason="…"` text |

### `excluded_files`

| Field | Type | Meaning |
|-------|------|---------|
| `count` | integer | Files removed by `--ignore-filename-regex` / `diff.ignore-filename-regex`; only files that were in a head or baseline report count, once each |
| `touched_count` | integer | How many of them this diff touched |
| `new_executable_lines` | integer | Executable lines the diff added to excluded files: patch lines the filter took out of the denominator |
| `paths` | string[] | Every excluded path |
| `touched` | string[] | The subset of `paths` this diff touched |

### `explanation`

| Field | Type | Meaning |
|-------|------|---------|
| `text` | string | A paragraph describing the document |
| `fields` | object[] | One entry per documented field |
| `fields[].name` | string | Path of the field, e.g. `patch_coverage.files[].path` |
| `fields[].text` | string | What it contains |
| `fields[].present` | boolean | Whether this document populates it |

## `.patchcov/config.yaml`

Repository settings live in `.patchcov/config.yaml`, in version control. There are no
user-level or machine-level settings, so what a gate reports is visible to a reviewer.

**Discovery**, the same for every command, first match wins:

1. `--config-dir <PATH>` (`diff` only).
2. The `PATCHCOV_CONFIG_DIR` environment variable, when set and non-empty.
3. The nearest `.patchcov/` directory walking up from the repository root, never past the
   repository boundary (a `.git` entry).
4. `<repo root>/.patchcov`, which need not exist: a missing directory or file is an empty
   config, and behaviour is unchanged.

A file that is present but malformed (bad YAML, a value of the wrong type) is a hard error
rather than a silent default. Unknown keys are ignored, so a newer config stays readable by an
older binary, with one exception: a misspelled field inside a `path-mappings` entry fails.

| Key | Type | Default | Used by | Combines with the CLI flag |
|-----|------|---------|---------|-----------------------------|
| `diff.ignore-filename-regex` | list of regex strings | `[]` | `diff` | **Union** with `--ignore-filename-regex`: the flag only adds |
| `diff.path-mappings` | list of `{from, to}` | `[]` | `diff` | No flag; config only. `merge` does not read it |
| `diff.require-measured` | list of glob strings | `[]` | `diff` | **Union** with `--fail-on-unmeasured` |
| `diff.allow-path-mismatch` | boolean | `false` | `diff` | **Either** one enables it: set here or pass `--allow-path-mismatch` |
| `lint-markers.include` | list of glob strings | `[]` (every tracked text file) | `lint-markers` | **Replaced** by `--include` when the flag is given |

```yaml
# .patchcov/config.yaml
diff:
  ignore-filename-regex:
    - 'src/bits/popcount\.rs'
  path-mappings:
    - from: com/example
      to: services/api/src/main/java/com/example
  require-measured:
    - 'src/**/*.rs'
  allow-path-mismatch: false
lint-markers:
  include:
    - '**/*.rs'
```

### `diff.ignore-filename-regex`

Regexes ([Rust regex syntax](https://docs.rs/regex/latest/regex/#syntax)) matched against the
repo-relative path, **unanchored** (a partial match is a match), after `--strip-prefix`. A
matching file is removed from both the head and baseline reports. Empty patterns are dropped,
because an empty regex would match every path. An invalid regex is an error. See
[excluding files](usage.md#excluding-files).

### `diff.path-mappings`

Each entry has exactly two string fields, `from` and `to`; any other field is an error.

| Field | Meaning |
|-------|---------|
| `from` | A report path prefix. Matches whole directory components (`src` does not match `src2`). `''` matches every *relative* path |
| `to` | The repo-relative replacement. `''` removes the matched prefix. Must not start with `/`, contain `:`, or contain `..` |

The longest matching `from` wins, each path is mapped once (no chaining), and two entries with
the same normalised `from` are an error. Backslashes become `/` and a leading `./` and
trailing `/` are dropped before comparison. Aliases that map to one path combine by maximum
line hits. Mappings run before `--strip-prefix`. See
[package-relative paths](usage.md#package-relative-and-source-root-relative-paths) and
[the path pipeline](explanation.md#the-path-pipeline).

### `diff.require-measured`

Globs matched against whole repo-relative paths: `*` stays within one directory, `**` crosses
directories (`src/**/*.rs` also matches `src/a.rs`). A touched file that matches and is absent
from every report fails the run. An empty or invalid glob is an error. See
[requiring touched files to be measured](usage.md#requiring-touched-files-to-be-measured).

### `diff.allow-path-mismatch`

A boolean, `false` by default. By default `diff` fails when a nonempty head, shard or baseline
report has no path matching a tracked file, because such a report cannot be joined to the diff
and a gate over it would pass without measuring anything. `true` turns that error into the same
message as a warning on stderr, exactly as `--allow-path-mismatch` does; either one is enough.
Set it only for a repository where a report legitimately matches nothing, since a gate cannot
see a patch it cannot join. A report with at least one tracked path never triggers the check.
See [absolute paths from another runner](usage.md#absolute-paths-from-another-runner).

### `lint-markers.include`

Globs with the same syntax, selecting which tracked files `patchcov lint-markers` scans when
given no paths. A file is scanned when it matches any glob. A glob list that matches no tracked
file scans nothing and prints a warning. See
[checking marker syntax](usage.md#check-marker-syntax-locally).

## Environment variables

| Variable | Used by | Meaning |
|----------|---------|---------|
| `PATCHCOV_CONFIG_DIR` | every command that reads config | Directory holding `config.yaml`; overrides discovery, overridden by `--config-dir`. Empty is ignored |
| `COVERAGE_ARTIFACT_URL` | `diff` | Fallback for `--artifact-url` |
| `COVERAGE_RUN_URL` | `diff` | Fallback for `--run-url` |
| `COVERAGE_COMMIT_URL` | `diff` | Fallback for `--commit-url` |
| `COVERAGE_BASE_SHA` | `diff` | Fallback for `--base-sha` |
| `COVERAGE_HEAD_SHA` | `diff` | Fallback for `--head-sha` |

A flag wins over its variable, and an empty variable counts as unset. These only change the
links and SHAs in the markdown footer.
