# Explanation

Why `patchcov` computes what it computes. For how to do something, see
[usage](usage.md); for exact flags and fields, see the [reference](reference.md).

## Contents

- [Why the total differs from llvm-cov's summary](#why-the-total-differs-from-llvm-covs-summary)
- [Diff scoping and the headline delta](#diff-scoping-and-the-headline-delta)
- [How `tolerate` masking works](#how-tolerate-masking-works)
- [The path pipeline](#the-path-pipeline)

## Why the total differs from llvm-cov's summary

One `cargo llvm-cov` run yields two different line counts, and the lcov it writes contains
both. This table is from this repository's own test suite, at commit `f20177b`, measured
with `cargo-llvm-cov` 0.9.1 and rustc 1.99.0:

| Count                                                       | Lines | Covered | Uncovered |       % |
|-------------------------------------------------------------|------:|--------:|----------:|--------:|
| `llvm-cov report --summary-only` (the `TOTAL` row)          | 6,836 |   6,768 |        68 | 99.0053 |
| lcov `LF:` / `LH:` records, summed over files               | 6,836 |   6,768 |        68 | 99.0053 |
| lcov `DA:` records, counted: **what `patchcov diff` uses**  | 6,652 |   6,607 |        45 | 99.3235 |

The figures move as the code and tests change; the shape of the gap is what matters, and
[below](#measuring-the-gap-on-your-own-report) is how to reproduce it for any report.
llvm-cov writes `LF`/`LH` from the summary, so even a single lcov disagrees with its own
`DA` records. `patchcov diff` counts the `DA` records (and, for the JSON export, rebuilds
lines from `segments`) and never reads `LF`/`LH` or the export's `summary`/`totals`, which
hold the summary's figure too. The lcov and the JSON export of one run give the same gate
figure; on this suite both print `Total: 99.32%`.

**The summary counts per function record; the per-line view counts per line.** The two agree
until a line is mapped by more than one function record. Three effects separate them:

1. **Shared lines.** rustc emits a separate function record for every closure and every
   `async fn` body, in addition to the enclosing function's, and the enclosing record also
   maps some of the inner one's lines: the line a closure starts on, an `async fn`'s
   signature line, parts of a `#[tokio::test]` body. The summary adds a line once per record
   that maps it; the per-line view adds it once. The lcov is not missing any code: its `DA`
   list is the same lines without the repeats. This is the whole line-count gap.
2. **Generic instantiations.** The summary merges the instantiations of one generic function
   by taking the larger *covered* count and the larger *line* count separately; the per-line
   view calls a line covered if any instantiation ran it. When `f::<u8>` takes one branch and
   `f::<u16>` the other, the summary reports a missed line that the per-line view does not.
3. **Nested records.** Where an inner record (a closure) never ran but the enclosing function
   did, the file view takes the inner region's count for the lines they share and the summary
   takes the enclosing record's.

**The direction depends on the code.** Effect 2 can only lower the summary relative to the
per-line figure, and effect 3 can only raise it (and needs a closure that never ran inside a
function that did). Effect 1 lowers it when the shared lines are covered less often than the
rest of the code and raises it when they are covered more often. On this suite the per-line
figure reads higher than the summary, so a threshold carried over from
`llvm-cov report --fail-under-lines` would be lenient here; a codebase whose closures and
async bodies are better covered than the rest could read lower. Measure it rather than
assume.

**Why the gate does not use the summary.** The summary is a sum over function records, and
neither an lcov's `DA` records nor the JSON `segments` say which records map a line, so it
cannot be rebuilt from the per-line report that patch coverage, `ignore` markers,
`--ignore-filename-regex`, shard merging and baselines all work on. The JSON export's
`summary` does carry it, per file only, but using it would give up those features and count
every shared line twice. The per-line figure has one more property worth having: each source
line counts once, which is the unit patch coverage already uses.

### Measuring the gap on your own report

Both counts are in one llvm-cov lcov:

```bash
cargo llvm-cov report --lcov --output-path head.lcov
awk -F'[:,]' '
  /^DA:/ { da++; if ($3 > 0) dah++ }
  /^LF:/ { lf += $2 }
  /^LH:/ { lh += $2 }
  END {
    if (!da) { print "no DA records in the report"; exit 1 }
    printf "summary  (LF/LH): %d of %d lines (%.4f%%)\n", lh, lf, lf ? 100 * lh / lf : 0
    printf "per-line (DA):    %d of %d lines (%.4f%%)\n", dah, da, 100 * dah / da
  }' head.lcov
```

On the commit above this prints:

```text
summary  (LF/LH): 6768 of 6836 lines (99.0053%)
per-line (DA):    6607 of 6652 lines (99.3235%)
```

An lcov whose `LF`/`LH` count its own `DA` records, as the lcov format defines them, prints
the same figure twice, and there is nothing to measure. `patchcov merge` writes such a file.

## Diff scoping and the headline delta

By default the project-delta and indirect-change sections are scoped to **files the diff
touches**. Coverage is measured by two independent instrumented runs (baseline and head), so
lines in untouched files can flip covered↔uncovered purely from run-to-run variance and
surface as phantom deltas. Genuine cross-file effects still surface through a
magnitude-gated "notable unchanged" note: an untouched file shows up there only when it
moved by at least 10 covered lines. Pass `--all-files` to restore the unscoped (noisier)
report. Patch coverage is unaffected by this scoping.

The **headline total** applies the same tolerance the per-file sections do: a move smaller
than 0.05 pp renders neutral (⚪), and the direction emoji is taken from the *rounded* value
printed beside it, so a delta displayed as `0 pp` is never painted red or green. When the
total does move but no per-file row and no "unchanged files also moved" entry accounts for
it, the headline is annotated `_(not attributable to this diff)_`: the move is then, by
construction, cross-run variance spread across files rather than an effect of the PR. The
percentage itself is always the real measured value.

## How `tolerate` masking works

For each head file, the tolerated head lines are scored with the hit status of their **base
counterpart** (found through the same diff alignment used for indirect changes; identity for
a file the diff never touched). A tolerated line with no counterpart, that is an added line,
keeps its real status.

That effective coverage is what feeds the headline Δ, the per-file Δ, the "unchanged files
also moved" gate, and the indirect-change list. The **displayed percentages are always the
real, measured values**. So a CPU-gated function that flips between two runner CPUs renders
as `Total: 92.79% ⚪ 0 pp` rather than `🔴 -0.01 pp`, with `92.79%` still the true number. In
structured output the real value is `after` and the masked one is `after_effective`, present
only when masking changed it; see [the schema](reference.md#project_delta).

Two consequences worth stating outright:

- **Added lines inside a `tolerate` region stay in the patch-coverage denominator.** New code
  should still be tested even if it flaps later.
- **Without a baseline**, `tolerate` is a no-op, since there are no deltas to mask.

`ignore` needs no masking: its lines are removed from both reports before any analysis, so
they are in neither the percentages nor the deltas.

## The path pipeline

Coverage tools write file names however suits them (absolute paths on the runner that
measured, paths relative to a package, Go import paths, `package/File.java`), while git
reports paths relative to the repository root. Everything patchcov does is a join between
the two, so every step of the pipeline exists to turn the first kind into the second, in an
order that keeps each step predictable:

1. **Parse.** The only format-specific step. A Go coverprofile also loses the module path
   declared by the root `go.mod` here, the one transformation inferred from the checkout
   rather than configured.
2. **Explicit `path-mappings`.** Done before prefix stripping, so a mapping can name the
   runner's absolute root (`/ci/head/project`) and have the rest handled for it, and a
   mapped runner root is not mistaken for a stray out-of-tree shard.
3. **`--strip-prefix`.** The default (the working directory) covers the common case of a
   report measured in this checkout.
4. **Filters and markers.** `--ignore-filename-regex` and source markers see repo-relative
   paths, so a pattern written for the repository works the same on any runner. A file
   excluded here is never read for markers.

Each step is deliberately mechanical. There is no filesystem search and no guessing of source
roots: a heuristic that picked the wrong root would attribute coverage to the wrong file
without any sign, whereas an unmapped path shows up as an empty patch that
[troubleshooting](troubleshooting.md#no-files-match-the-diff) tells you how to read.
