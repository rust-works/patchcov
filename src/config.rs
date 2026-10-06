//! Discovery and loading of patchcov's repository config directory.
//!
//! A repository declares persistent settings in `.patchcov/config.yaml`. The
//! directory is resolved from, in order:
//!
//! 1. an explicit override (`--config-dir`);
//! 2. the [`CONFIG_DIR_ENV`] environment variable, when set and non-empty;
//! 3. the nearest `.patchcov/` directory found walking up from the repository
//!    root, stopping at the repository boundary (a `.git` entry);
//! 4. `<repo root>/.patchcov`, which need not exist — a missing config is
//!    simply an empty one.
//!
//! There is no user-level or machine-level tier: a setting that changes what a
//! coverage gate reports belongs in version control, where a reviewer can see it.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Environment variable that overrides config-directory discovery.
pub const CONFIG_DIR_ENV: &str = "PATCHCOV_CONFIG_DIR";

/// Name of the per-repository config directory.
pub const CONFIG_DIR_NAME: &str = ".patchcov";

/// Resolves the config directory for the repository rooted at `repo_root`.
pub fn resolve_config_dir_at(override_dir: Option<&Path>, repo_root: &Path) -> PathBuf {
    let env = std::env::var(CONFIG_DIR_ENV).ok();
    resolve_with_env(override_dir, repo_root, env.as_deref())
}

/// [`resolve_config_dir_at`] with the environment value injected, so every tier
/// is testable without mutating process-global state.
fn resolve_with_env(
    override_dir: Option<&Path>,
    repo_root: &Path,
    env_dir: Option<&str>,
) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir.to_path_buf();
    }
    if let Some(dir) = env_dir.filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    walk_up_find_config_dir(repo_root).unwrap_or_else(|| repo_root.join(CONFIG_DIR_NAME))
}

/// The first `.patchcov/` directory at or above `start`, never escaping the
/// repository (the walk stops after the directory holding `.git`).
fn walk_up_find_config_dir(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        let candidate = current.join(CONFIG_DIR_NAME);
        if candidate.is_dir() {
            return Some(candidate);
        }
        if current.join(".git").exists() || !current.pop() {
            return None;
        }
    }
}

/// Reads `<dir>/<filename>`, or `None` when it does not exist.
pub fn load_config_content(dir: &Path, filename: &str) -> Result<Option<String>> {
    let path = dir.join(filename);
    if !path.exists() {
        return Ok(None);
    }
    fs::read_to_string(&path)
        .with_context(|| format!("could not read config file {}", path.display()))
        .map(Some)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        dir
    }

    #[test]
    fn override_wins_over_everything() {
        let repo = repo();
        fs::create_dir(repo.path().join(CONFIG_DIR_NAME)).unwrap();
        let got = resolve_with_env(Some(Path::new("/x")), repo.path(), Some("/env"));
        assert_eq!(got, Path::new("/x"));
    }

    #[test]
    fn env_wins_over_discovery() {
        let repo = repo();
        fs::create_dir(repo.path().join(CONFIG_DIR_NAME)).unwrap();
        let got = resolve_with_env(None, repo.path(), Some("/env"));
        assert_eq!(got, Path::new("/env"));
    }

    #[test]
    fn empty_env_is_ignored() {
        let repo = repo();
        let got = resolve_with_env(None, repo.path(), Some(""));
        assert_eq!(got, repo.path().join(CONFIG_DIR_NAME));
    }

    #[test]
    fn walk_up_finds_the_nearest_dir_within_the_repository() {
        let repo = repo();
        let nested = repo.path().join("a/b");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(repo.path().join(CONFIG_DIR_NAME)).unwrap();
        assert_eq!(
            resolve_with_env(None, &nested, None),
            repo.path().join(CONFIG_DIR_NAME)
        );
    }

    #[test]
    fn walk_up_does_not_escape_the_repository() {
        let outer = tempfile::tempdir().unwrap();
        fs::create_dir(outer.path().join(CONFIG_DIR_NAME)).unwrap();
        let inner = outer.path().join("repo");
        fs::create_dir_all(inner.join(".git")).unwrap();
        assert_eq!(
            resolve_with_env(None, &inner, None),
            inner.join(CONFIG_DIR_NAME)
        );
    }

    #[test]
    fn load_returns_none_for_a_missing_file_and_content_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load_config_content(dir.path(), "config.yaml").unwrap(),
            None
        );
        fs::write(dir.path().join("config.yaml"), "diff: {}\n").unwrap();
        assert_eq!(
            load_config_content(dir.path(), "config.yaml")
                .unwrap()
                .as_deref(),
            Some("diff: {}\n")
        );
    }
}
