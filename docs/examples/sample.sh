#!/usr/bin/env bash
# Builds a tiny git repository with a base and a head commit, writes an lcov report
# for each, and runs `patchcov diff` on them. The README's sample comment and the
# JSON excerpt in docs/reference.md are this script's output.
#
#   docs/examples/sample.sh            # PR comment (markdown)
#   docs/examples/sample.sh json       # the same result as JSON
#
# Uses `patchcov` from PATH; set PATCHCOV=/path/to/patchcov to use another binary.
# Needs git 2.28 or newer (`git init -b`).
set -euo pipefail

format="${1:-markdown}"
patchcov="${PATCHCOV:-patchcov}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
repo="$work/repo"
mkdir -p "$repo/src"
cd "$repo"
git init -q -b main
git config user.email sample@example.com
git config user.name sample
git config commit.gpgsign false
git config core.hooksPath /dev/null

# `lines <prefix> <from> <to>` writes placeholder source lines.
lines() { for i in $(seq "$2" "$3"); do echo "let $1$i = $i;"; done; }
# `lcov <file> <line:hits>...` writes one lcov record.
lcov() { echo "SF:$1"; shift; for r in "$@"; do echo "DA:${r%%:*},${r##*:}"; done; echo end_of_record; }
span() { for i in $(seq "$1" "$2"); do echo "$i:$3"; done; }

# Base commit: parser.rs has 20 lines, config.rs has 10, all covered.
lines a 1 20 > src/parser.rs
lines c 1 10 > src/config.rs
git add src && git commit -q -m base
{ lcov src/parser.rs $(span 1 20 1); lcov src/config.rs $(span 1 10 1); } > "$work/base.lcov"

# Head commit: parser.rs gains 12 lines (5 never run), and cache.rs is new (2 of 6 never run).
lines a 21 32 >> src/parser.rs
lines d 1 6 > src/cache.rs
git add src && git commit -q -m head
{
  lcov src/parser.rs $(span 1 20 1) $(span 21 27 3) $(span 28 32 0)
  lcov src/config.rs $(span 1 10 1)
  lcov src/cache.rs $(span 1 4 1) $(span 5 6 0)
} > "$work/head.lcov"

"$patchcov" diff --report "$work/head.lcov" --baseline-report "$work/base.lcov" \
  --base-ref HEAD~1 --collapse-ranges -o "$format" --no-explanation
