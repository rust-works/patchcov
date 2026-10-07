# Releasing

Releases are driven by [release-plz](https://release-plz.dev) from the
[conventional commits](https://www.conventionalcommits.org) on `main`.
[`.github/workflows/release-plz.yml`](../.github/workflows/release-plz.yml) runs on
every push to `main`, including merge commits created by the
[merge queue](../CONTRIBUTING.md#merging). Release jobs run after the merge, not
on `merge_group`, and are not required queue checks.

## The flow

1. **Queue changes** with **Merge when ready** and conventional commit messages
   (`feat:`, `fix:`, …). Below 1.0, a `feat` raises the minor version and a `fix` the
   patch version; a breaking
   change (`feat!:` or a `BREAKING CHANGE:` footer) also raises the minor version.
2. **release-plz opens (and keeps updating) a release PR** that bumps the version in
   `Cargo.toml` and `Cargo.lock` and writes the new section of `CHANGELOG.md`. It also
   runs `cargo-semver-checks` and fails the PR if the public API broke without a
   matching bump. Edit the PR's changelog text if you want to; select **Merge when
   ready** when you want to release. Release PRs must pass the same CI and queue
   checks as other PRs.
3. **Merging the release PR releases.** On the push to `main`, the `release` job
   publishes the crate to crates.io, pushes the `vX.Y.Z` tag on the merged commit
   (rebased or not, so there is no rebased-SHA problem) and creates the GitHub
   release. The `binaries` job then builds `patchcov` for five targets and uploads the
   `.tar.gz` (Linux/macOS) or `.zip` (Windows) archives and `.sha256` files to that release.

| Target | Runner | Notes |
|---|---|---|
| `x86_64-unknown-linux-gnu` | `ubuntu-22.04` | needs glibc 2.35 or newer |
| `aarch64-unknown-linux-gnu` | `ubuntu-22.04-arm` | needs glibc 2.35 or newer |
| `aarch64-apple-darwin` | `macos-latest` | |
| `x86_64-apple-darwin` | `macos-latest` | cross-compiled |
| `x86_64-pc-windows-msvc` | `windows-latest` | ZIP containing `patchcov.exe` |

Each archive contains the executable, LICENSE and README.md. The Windows job
extracts its ZIP and runs `patchcov.exe --version` before uploading it.

The Linux jobs fail if the binary needs a glibc symbol newer than the floor
(`GLIBC_FLOOR` in the workflow), so a runner-image change is a failed release build
rather than a silent one. Always build on a pinned image, never `ubuntu-latest`.

The binaries are unsigned and not notarized, so macOS Gatekeeper quarantines a
downloaded one; `cargo install patchcov` avoids that.

### Writing a useful changelog

release-plz builds each release's changelog section from the commit subjects on `main`,
grouped by conventional-commit type: `feat` under *Added*, `fix` under *Fixed*, and the rest
(`docs:`, `chore:`, and commits with no type) under *Other*, as the 0.2.0 section shows. The
subject you write on the branch is therefore the only text a user sees, so write it for them.
Before you select **Merge when ready** on a release PR, read its changelog section the way a
user would:

- Rewrite a vague or purely internal subject, or drop an entry that means nothing outside the
  repository.
- Commit anything users will notice as `feat:` or `fix:`, so it does not land in *Other*.
- Describe what changed for the user, not how it was implemented.

Do not hand-edit the generated section of `CHANGELOG.md` on `main` outside a release PR, and
leave released sections alone except to correct a factual error: release-plz owns the file
and a hand-written entry would be duplicated by the commit-generated one.

## One-time setup

These need a repository admin and cannot be done from a workflow.

1. **Publish the first version by hand.** crates.io Trusted Publishing cannot create
   a crate, so `0.1.0` is published manually, from a checkout of the merged commit:

   ```bash
   cargo login            # a crates.io API token with the publish-new scope
   cargo publish --dry-run
   cargo publish
   git tag -a v0.1.0 -m "patchcov 0.1.0" && git push origin v0.1.0
   gh release create v0.1.0 --title v0.1.0 --notes-file <(sed -n '/^## \[0.1.0\]/,/^## \[/p' CHANGELOG.md | sed '$d')
   ```

   The tag matters: release-plz computes the next version from the commits since the
   latest tag, so without `v0.1.0` it has no baseline.
2. **Configure Trusted Publishing** on crates.io, under the crate's settings:
   repository `rust-works/patchcov`, workflow filename `release-plz.yml`, no
   environment. The workflow already grants `id-token: write` to the `release` job,
   and stores no registry token. Until this is done the `release` job fails when it
   has something to publish.
3. **Let release-plz open PRs: set `RELEASE_PLZ_TOKEN` (recommended).** A
   fine-grained personal access token (or GitHub App token), owned by `rust-works`
   and limited to this repository, with *Contents* and *Pull requests* write access,
   stored as the `RELEASE_PLZ_TOKEN` secret. The `pr` job opens the release PR with it,
   so CI runs on that PR (a PR opened with the default `GITHUB_TOKEN` does not trigger
   other workflows). Fine-grained tokens expire, so renew it before then.
4. **Without the token: allow Actions to open PRs.** Repository settings → Actions →
   General → Workflow permissions → enable *Allow GitHub Actions to create and approve
   pull requests*. The `pr` job then falls back to `GITHUB_TOKEN`, and the release PR
   has no CI checks and cannot enter the merge queue. Configure `RELEASE_PLZ_TOKEN`
   and close/reopen the release PR with a user or app token to trigger CI before
   selecting **Merge when ready**. Local tests do not satisfy required checks.
   This setting is not needed when `RELEASE_PLZ_TOKEN` is set, and the `rust-works`
   organization policy currently forbids it.

## If a release goes wrong

- A published crates.io version cannot be deleted, only yanked
  (`cargo yank --version X.Y.Z patchcov`); release a fixed version instead.
- If `release` published but `binaries` failed, re-run the failed jobs: the upload
  uses `--clobber`, so it is safe to repeat.
- If the tag exists but the crate was not published, fix the cause and re-run the
  `release` job; release-plz skips what is already released and publishes what is not.
