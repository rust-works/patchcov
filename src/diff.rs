//! Unified-diff model built from `git2`.
//!
//! [`DiffModel`] captures exactly what coverage attribution needs from a diff:
//! per file, the set of *added* (new-side) line numbers, and a deterministic
//! base↔head alignment for *unchanged* lines so coverage can be compared across
//! versions without heuristics. Added lines come from the `+` lines of the diff;
//! the alignment is derived from hunk offsets (`@@ -a,b +c,d @@`) plus the exact
//! context-line pairs `git2` reports inside each hunk.
//!
//! Rename detection is enabled so a moved file is diffed against its original
//! content — added lines and "lost coverage" are attributed correctly instead of
//! the whole file looking new.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use git2::{Delta, DiffFindOptions, DiffOptions, Repository};

/// One hunk's line ranges, used to compute the unchanged-line offset.
#[derive(Debug, Clone, Copy)]
struct Hunk {
    old_start: u32,
    old_lines: u32,
    new_lines: u32,
}

/// Per-file diff information.
#[derive(Debug, Clone)]
pub struct FileDiff {
    /// Old-side (base) path. Differs from `new_path` on a rename; `None` for an
    /// added file.
    pub old_path: Option<String>,
    /// New-side (head) path.
    pub new_path: String,
    /// Whether the file is newly added in head.
    pub is_new: bool,
    /// Whether the file was deleted from head.
    pub is_deleted: bool,
    /// Whether the file was renamed (and possibly modified).
    pub is_rename: bool,
    /// New-side line numbers added or modified by the diff.
    pub added: BTreeSet<u32>,
    /// Old-side line numbers removed by the diff.
    pub removed: BTreeSet<u32>,
    /// Exact `(old_line, new_line)` pairs for context lines inside hunks.
    context: BTreeMap<u32, u32>,
    /// Hunk ranges, ordered by old-side start.
    hunks: Vec<Hunk>,
}

impl FileDiff {
    /// Builds a [`FileDiff`] with only added/removed line sets and no hunk
    /// alignment. Used to construct diffs programmatically and in tests; the
    /// `git2`-driven path uses [`DiffModel::from_diff`] which also records hunks.
    pub fn new(
        new_path: impl Into<String>,
        old_path: Option<String>,
        is_new: bool,
        is_rename: bool,
        added: BTreeSet<u32>,
        removed: BTreeSet<u32>,
    ) -> Self {
        Self {
            old_path,
            new_path: new_path.into(),
            is_new,
            is_deleted: false,
            is_rename,
            added,
            removed,
            context: BTreeMap::new(),
            hunks: Vec::new(),
        }
    }

    /// Maps an unchanged base-side line to its head-side line number.
    ///
    /// Returns `None` when the base line was removed or modified (i.e. it has no
    /// unchanged counterpart in head). Context lines inside hunks use the exact
    /// pairs `git2` reported; lines outside every hunk use the cumulative hunk
    /// offset.
    pub fn map_base_to_head(&self, old_line: u32) -> Option<u32> {
        if let Some(&new_line) = self.context.get(&old_line) {
            return Some(new_line);
        }
        // Inside a hunk's old-range but not a recorded context line ⇒ removed/modified.
        if self
            .hunks
            .iter()
            .any(|h| old_line >= h.old_start && old_line < h.old_start + h.old_lines)
        {
            return None;
        }
        // Outside all hunks: shift by the net line delta of all preceding hunks.
        let delta: i64 = self
            .hunks
            .iter()
            .filter(|h| h.old_start + h.old_lines <= old_line)
            .map(|h| i64::from(h.new_lines) - i64::from(h.old_lines))
            .sum();
        let mapped = i64::from(old_line) + delta;
        u32::try_from(mapped).ok().filter(|&n| n >= 1)
    }
}

/// A whole diff: the set of changed files between two revisions.
#[derive(Debug, Clone, Default)]
pub struct DiffModel {
    /// Changed files, keyed by new-side path.
    pub files: BTreeMap<String, FileDiff>,
}

impl DiffModel {
    /// Builds a diff model between `base_ref` and `head_ref` in `repo`.
    ///
    /// `head_ref` defaults to `HEAD` when `None`. Both revisions are resolved
    /// with `revparse_single` (so branch names, tags, and SHAs all work).
    pub fn between(repo: &Repository, base_ref: &str, head_ref: Option<&str>) -> Result<Self> {
        let base_tree = repo
            .revparse_single(base_ref)
            .with_context(|| format!("could not resolve base ref `{base_ref}`"))?
            .peel_to_tree()
            .with_context(|| format!("base ref `{base_ref}` is not a tree-ish"))?;
        let head_ref = head_ref.unwrap_or("HEAD");
        let head_tree = repo
            .revparse_single(head_ref)
            .with_context(|| format!("could not resolve head ref `{head_ref}`"))?
            .peel_to_tree()
            .with_context(|| format!("head ref `{head_ref}` is not a tree-ish"))?;

        let mut opts = DiffOptions::new();
        opts.context_lines(3).patience(true).indent_heuristic(true);
        let mut diff = repo
            .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), Some(&mut opts))
            .context("failed to diff base against head")?;
        let mut find = DiffFindOptions::new();
        find.renames(true).copies(true);
        diff.find_similar(Some(&mut find))
            .context("failed to run rename detection")?;

        Self::from_diff(&diff)
    }

    /// Builds a diff model from an already-computed `git2` diff.
    pub fn from_diff(diff: &git2::Diff) -> Result<Self> {
        // Interior mutability so the three foreach callbacks can share the
        // accumulator (git2 hands each a separate `&mut FnMut`).
        let builders: RefCell<BTreeMap<String, FileDiff>> = RefCell::new(BTreeMap::new());

        let new_path_of = |delta: &git2::DiffDelta| -> Option<String> {
            delta
                .new_file()
                .path()
                .and_then(|p| p.to_str())
                .map(str::to_string)
        };

        diff.foreach(
            &mut |delta, _progress| {
                if let Some(new_path) = new_path_of(&delta) {
                    let old_path = delta
                        .old_file()
                        .path()
                        .and_then(|p| p.to_str())
                        .map(str::to_string);
                    let status = delta.status();
                    builders
                        .borrow_mut()
                        .entry(new_path.clone())
                        .or_insert(FileDiff {
                            old_path: old_path.filter(|_| status != Delta::Added),
                            new_path,
                            is_new: status == Delta::Added,
                            is_deleted: status == Delta::Deleted,
                            is_rename: status == Delta::Renamed || status == Delta::Copied,
                            added: BTreeSet::new(),
                            removed: BTreeSet::new(),
                            context: BTreeMap::new(),
                            hunks: Vec::new(),
                        });
                }
                true
            },
            None,
            Some(&mut |delta, hunk| {
                if let Some(new_path) = new_path_of(&delta) {
                    if let Some(file) = builders.borrow_mut().get_mut(&new_path) {
                        file.hunks.push(Hunk {
                            old_start: hunk.old_start(),
                            old_lines: hunk.old_lines(),
                            new_lines: hunk.new_lines(),
                        });
                    }
                }
                true
            }),
            Some(&mut |delta, _hunk, line| {
                if let Some(new_path) = new_path_of(&delta) {
                    if let Some(file) = builders.borrow_mut().get_mut(&new_path) {
                        match line.origin() {
                            '+' => {
                                if let Some(n) = line.new_lineno() {
                                    file.added.insert(n);
                                }
                            }
                            '-' => {
                                if let Some(n) = line.old_lineno() {
                                    file.removed.insert(n);
                                }
                            }
                            ' ' => {
                                if let (Some(o), Some(n)) = (line.old_lineno(), line.new_lineno()) {
                                    file.context.insert(o, n);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                true
            }),
        )
        .context("failed to walk diff")?;

        let mut files = builders.into_inner();
        for file in files.values_mut() {
            file.hunks.sort_by_key(|h| h.old_start);
        }
        Ok(Self { files })
    }
}

/// Refs tried, in order, to find the default base branch. The first that resolves
/// to a commit wins: the remote's declared default branch, then the conventional
/// names.
const DEFAULT_BASE_CANDIDATES: [&str; 5] = [
    "refs/remotes/origin/HEAD",
    "origin/main",
    "main",
    "origin/master",
    "master",
];

/// Resolves the default base ref: the merge-base of the default branch and `HEAD`,
/// returned as a hex SHA.
///
/// The default branch is the first of these that resolves:
/// `refs/remotes/origin/HEAD` (what `git clone` records), `origin/main`, `main`,
/// `origin/master`, `master`.
pub fn default_base_ref(repo: &Repository) -> Result<String> {
    let head = repo
        .head()
        .context("could not resolve HEAD")?
        .peel_to_commit()
        .context("HEAD is not a commit")?
        .id();
    let branch = DEFAULT_BASE_CANDIDATES
        .iter()
        .find_map(|name| {
            repo.revparse_single(name)
                .ok()
                .and_then(|object| object.peel_to_commit().ok())
        })
        .with_context(|| {
            format!(
                "could not resolve a default base ref (tried {}); pass --base-ref",
                DEFAULT_BASE_CANDIDATES
                    .map(|name| format!("`{name}`"))
                    .join(", ")
            )
        })?
        .id();
    let base = repo
        .merge_base(branch, head)
        .context("could not compute merge-base of base branch and HEAD")?;
    Ok(base.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Builds a `FileDiff` with the given hunks and context pairs for testing the
    /// alignment function in isolation.
    fn file_diff(hunks: Vec<Hunk>, context: &[(u32, u32)]) -> FileDiff {
        FileDiff {
            old_path: Some("f".into()),
            new_path: "f".into(),
            is_new: false,
            is_deleted: false,
            is_rename: false,
            added: BTreeSet::new(),
            removed: BTreeSet::new(),
            context: context.iter().copied().collect(),
            hunks,
        }
    }

    #[test]
    fn lines_before_first_hunk_map_identically() {
        let fd = file_diff(
            vec![Hunk {
                old_start: 10,
                old_lines: 2,
                new_lines: 5,
            }],
            &[],
        );
        assert_eq!(fd.map_base_to_head(1), Some(1));
        assert_eq!(fd.map_base_to_head(9), Some(9));
    }

    #[test]
    fn lines_after_hunk_shift_by_net_delta() {
        // Hunk replaces 2 old lines with 5 new lines: +3 shift afterwards.
        let fd = file_diff(
            vec![Hunk {
                old_start: 10,
                old_lines: 2,
                new_lines: 5,
            }],
            &[],
        );
        // old line 12 is the first line after the hunk's old-range [10,12).
        assert_eq!(fd.map_base_to_head(12), Some(15));
        assert_eq!(fd.map_base_to_head(20), Some(23));
    }

    #[test]
    fn modified_lines_inside_hunk_have_no_mapping() {
        let fd = file_diff(
            vec![Hunk {
                old_start: 10,
                old_lines: 2,
                new_lines: 5,
            }],
            &[],
        );
        assert_eq!(fd.map_base_to_head(10), None);
        assert_eq!(fd.map_base_to_head(11), None);
    }

    #[test]
    fn context_pairs_take_precedence() {
        let fd = file_diff(
            vec![Hunk {
                old_start: 10,
                old_lines: 4,
                new_lines: 4,
            }],
            &[(10, 10), (13, 13)],
        );
        // Recorded context lines map exactly even though inside the hunk range.
        assert_eq!(fd.map_base_to_head(10), Some(10));
        assert_eq!(fd.map_base_to_head(13), Some(13));
        // 11/12 are inside the hunk and not context ⇒ no mapping.
        assert_eq!(fd.map_base_to_head(11), None);
    }

    /// Commits `files` to `repo`, returning the new commit's id.
    fn commit(
        repo: &Repository,
        path: &std::path::Path,
        files: &[(&str, &str)],
        parent: Option<git2::Oid>,
    ) -> git2::Oid {
        let mut index = repo.index().unwrap();
        index.clear().unwrap();
        for (name, content) in files {
            std::fs::write(path.join(name), content).unwrap();
            index.add_path(std::path::Path::new(name)).unwrap();
        }
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("T", "t@e.com").unwrap();
        let parent = parent.map(|id| repo.find_commit(id).unwrap());
        let parents: Vec<&git2::Commit> = parent.as_ref().into_iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, &parents)
            .unwrap()
    }

    #[test]
    fn default_base_ref_is_merge_base_with_main() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        let repo = Repository::init(path).unwrap();
        let first = commit(&repo, path, &[("a.rs", "1\n")], None);
        // Mark `main` at the first commit, then advance HEAD.
        repo.branch("main", &repo.find_commit(first).unwrap(), false)
            .unwrap();
        commit(&repo, path, &[("a.rs", "1\n2\n")], Some(first));

        let base = default_base_ref(&repo).unwrap();
        assert_eq!(base, first.to_string());
    }

    /// Builds a repo with a first commit, then advances `HEAD` one commit past it,
    /// returning the repo and the first commit's id. `HEAD` is on `trunk`, so no
    /// default-base candidate exists until a test creates one.
    fn repo_with_branch_point(dir: &std::path::Path) -> (Repository, git2::Oid) {
        let mut opts = git2::RepositoryInitOptions::new();
        opts.initial_head("trunk");
        let repo = Repository::init_opts(dir, &opts).unwrap();
        let first = commit(&repo, dir, &[("a.rs", "1\n")], None);
        commit(&repo, dir, &[("a.rs", "1\n2\n")], Some(first));
        (repo, first)
    }

    /// Points `reference` (e.g. `refs/remotes/origin/develop`) at `target`.
    fn set_ref(repo: &Repository, reference: &str, target: git2::Oid) {
        repo.reference(reference, target, true, "test").unwrap();
    }

    #[test]
    fn default_base_ref_falls_back_to_master() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, first) = repo_with_branch_point(dir.path());
        repo.branch("master", &repo.find_commit(first).unwrap(), false)
            .unwrap();
        assert_eq!(default_base_ref(&repo).unwrap(), first.to_string());
    }

    #[test]
    fn default_base_ref_follows_origin_head_to_a_non_main_branch() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, first) = repo_with_branch_point(dir.path());
        set_ref(&repo, "refs/remotes/origin/develop", first);
        repo.reference_symbolic(
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/develop",
            true,
            "test",
        )
        .unwrap();
        assert_eq!(default_base_ref(&repo).unwrap(), first.to_string());
    }

    #[test]
    fn default_base_ref_prefers_origin_head_over_main() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, first) = repo_with_branch_point(dir.path());
        let head = repo.head().unwrap().peel_to_commit().unwrap().id();
        // `main` is at HEAD (merge base would be HEAD); origin/HEAD is at `first`.
        set_ref(&repo, "refs/heads/main", head);
        set_ref(&repo, "refs/remotes/origin/develop", first);
        repo.reference_symbolic(
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/develop",
            true,
            "test",
        )
        .unwrap();
        assert_eq!(default_base_ref(&repo).unwrap(), first.to_string());
    }

    #[test]
    fn default_base_ref_skips_a_dangling_origin_head() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, first) = repo_with_branch_point(dir.path());
        repo.branch("main", &repo.find_commit(first).unwrap(), false)
            .unwrap();
        // origin/HEAD names a branch that was never fetched.
        repo.reference_symbolic(
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/gone",
            true,
            "test",
        )
        .unwrap();
        assert_eq!(default_base_ref(&repo).unwrap(), first.to_string());
    }

    #[test]
    fn default_base_ref_prefers_main_names_over_master_names() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, first) = repo_with_branch_point(dir.path());
        let head = repo.head().unwrap().peel_to_commit().unwrap().id();
        // Every lower-priority ref sits at HEAD, so picking one would give HEAD.
        set_ref(&repo, "refs/remotes/origin/master", head);
        set_ref(&repo, "refs/heads/master", head);
        set_ref(&repo, "refs/heads/main", head);
        set_ref(&repo, "refs/remotes/origin/main", first);
        assert_eq!(default_base_ref(&repo).unwrap(), first.to_string());

        // Without origin/main, local `main` still beats origin/master.
        repo.find_reference("refs/remotes/origin/main")
            .unwrap()
            .delete()
            .unwrap();
        set_ref(&repo, "refs/heads/main", first);
        assert_eq!(default_base_ref(&repo).unwrap(), first.to_string());
    }

    #[test]
    fn default_base_ref_error_names_every_ref_tried() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, _) = repo_with_branch_point(dir.path());
        let message = format!("{:#}", default_base_ref(&repo).unwrap_err());
        for name in DEFAULT_BASE_CANDIDATES {
            assert!(message.contains(&format!("`{name}`")), "{message}");
        }
        assert!(message.contains("--base-ref"), "{message}");
    }

    #[test]
    fn between_records_removed_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        let repo = Repository::init(path).unwrap();
        let first = commit(&repo, path, &[("a.rs", "a\nb\nc\n")], None);
        commit(&repo, path, &[("a.rs", "a\nc\n")], Some(first)); // remove line "b"
        let diff = DiffModel::between(&repo, &first.to_string(), Some("HEAD")).unwrap();
        let fd = diff.files.get("a.rs").unwrap();
        assert!(fd.removed.contains(&2), "removed old line 2 (b)");
    }

    #[test]
    fn between_errors_on_bad_base_ref() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        let repo = Repository::init(path).unwrap();
        commit(&repo, path, &[("a.rs", "1\n")], None);
        assert!(DiffModel::between(&repo, "does-not-exist", Some("HEAD")).is_err());
    }
}
