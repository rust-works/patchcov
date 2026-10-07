# Contributing

Open pull requests against `main` and use conventional commit messages (`feat:`,
`fix:`, `docs:`, etc.). Keep those messages on the branch: the merge queue uses
merge commits, preserving them for release-plz.

## Local checks

Run the same checks as CI from your checkout:

```bash
cargo fmt --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo +1.88.0 check --all-targets
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --document-private-items
```

Install the MSRV toolchain with `rustup toolchain install 1.88.0 --profile minimal`
if needed. CI also runs tests on Linux, macOS, and Windows.

## Merging

`main` requires a pull request and the merge queue. Choose **Merge when ready**
on GitHub to join the queue once the PR requirements pass. The queue runs CI
again against the latest `main`, including changes ahead of yours, before it
merges. A failed or timed-out queue entry is removed; fix the failure and select
Merge when ready again. Do not push changes directly to `main`.

The following GitHub Actions checks are required on both the PR and its queue
entry. Keep their names and the `merge_group` trigger in
[CI](.github/workflows/ci.yml) in sync with the ruleset:

- `Test (ubuntu-latest)`
- `Test (macos-latest)`
- `Test (windows-latest)`
- `Lint`
- `MSRV (1.88)`
- `Docs`

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
2. In Actions, find the CI run triggered by `merge_group` and confirm all six
   required checks pass with the names above.
3. Confirm the queue merges the PR into `main` with a merge commit.
4. Find the Release workflow triggered by that push. Confirm `Release` and
   `Release PR` succeed and inspect the release PR that release-plz opens or
   updates. A documentation-only change may not require a version bump; a
   successful no-op is distinct from a failed release job. Binary jobs run only
   when a release is created.

Static workflow checks and a ruleset readback cannot prove this end-to-end flow;
record the queue and release run URLs when performing the smoke test.
