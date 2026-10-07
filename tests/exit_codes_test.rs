//! The process exit code for each class of failure, from the real binary.
//!
//! The codes are listed in `docs/reference.md#exit-codes`; a script tells a failed
//! gate from an unreadable report from these alone.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use git2::{Repository, Signature};
use tempfile::TempDir;

/// A repository with a base commit and a head commit that adds `a.rs` lines 2-5
/// and a new `b.rs`.
struct Fixture {
    dir: TempDir,
    base: String,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let base = commit(&repo, "base", &[("a.rs", "fn a() {}\n")], &[]);
        commit(
            &repo,
            "head",
            &[
                (
                    "a.rs",
                    "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n",
                ),
                ("b.rs", "fn f() {}\n"),
            ],
            &[base],
        );
        Self {
            dir,
            base: base.to_string(),
        }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Writes `contents` to `name` in the working tree and returns its path.
    fn file(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.root().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// A report that measures `a.rs` lines 2-5, with 2 and 3 covered (50% patch
    /// coverage, 50% of the lines of the report).
    fn report(&self) -> PathBuf {
        self.file(
            "head.lcov",
            "SF:a.rs\nDA:2,1\nDA:3,1\nDA:4,0\nDA:5,0\nend_of_record\n",
        )
    }

    /// `patchcov diff` against the base commit, in this repository.
    fn diff(&self, args: &[&str]) -> Output {
        run(self.root(), &["diff", "--base-ref", &self.base], args)
    }
}

fn commit(
    repo: &Repository,
    message: &str,
    files: &[(&str, &str)],
    parents: &[git2::Oid],
) -> git2::Oid {
    let root = repo.workdir().unwrap();
    let mut index = repo.index().unwrap();
    index.clear().unwrap();
    for (name, contents) in files {
        std::fs::write(root.join(name), contents).unwrap();
        index.add_path(Path::new(name)).unwrap();
    }
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let signature = Signature::now("Test User", "test@example.com").unwrap();
    let parents: Vec<_> = parents
        .iter()
        .map(|id| repo.find_commit(*id).unwrap())
        .collect();
    let parents: Vec<_> = parents.iter().collect();
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parents,
    )
    .unwrap()
}

/// Runs `patchcov -C <root> <head...> <args...>`.
fn run(root: &Path, head: &[&str], args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_patchcov"))
        // Keep an ambient override out of the run.
        .env_remove("PATCHCOV_CONFIG_DIR")
        // A temp directory inside a checkout must not make `root` look like part of it.
        .env("GIT_CEILING_DIRECTORIES", root.parent().unwrap())
        .arg("-C")
        .arg(root)
        .args(head)
        .args(args)
        .output()
        .unwrap()
}

fn code(output: &Output) -> i32 {
    output
        .status
        .code()
        .expect("the process was killed by a signal")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn success_is_0() {
    let fx = Fixture::new();
    let report = fx.report();
    let output = fx.diff(&["--report", path(&report), "--fail-under-patch", "50"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let output = fx.diff(&["--report", path(&report), "--branch-coverage"]);
    assert_eq!(
        code(&output),
        0,
        "lcov supports --branch-coverage: {}",
        stderr(&output)
    );
}

#[test]
fn a_failed_gate_is_1_and_still_prints_the_report() {
    let fx = Fixture::new();
    let report = fx.report();
    for (flag, value) in [("--fail-under-patch", "80"), ("--fail-under-lines", "100")] {
        let output = fx.diff(&["--report", path(&report), flag, value]);
        assert_eq!(code(&output), 1, "{flag}: {}", stderr(&output));
        assert!(
            !output.stdout.is_empty(),
            "{flag}: the report is printed first"
        );
        assert!(stderr(&output).contains(flag), "{}", stderr(&output));
    }
}

#[test]
fn an_unmeasured_touched_file_is_1() {
    let fx = Fixture::new();
    let report = fx.report();
    let output = fx.diff(&["--report", path(&report), "--fail-on-unmeasured", "b.rs"]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert!(stderr(&output).contains("b.rs"), "{}", stderr(&output));
}

#[test]
fn every_failed_gate_is_still_named_in_one_error() {
    let fx = Fixture::new();
    let report = fx.report();
    let output = fx.diff(&[
        "--report",
        path(&report),
        "--fail-under-patch",
        "80",
        "--fail-under-lines",
        "100",
    ]);
    assert_eq!(code(&output), 1);
    let text = stderr(&output);
    assert!(text.contains("--fail-under-patch") && text.contains("--fail-under-lines"));
}

#[test]
fn usage_errors_are_2() {
    let fx = Fixture::new();
    // From the argument parser.
    assert_eq!(code(&fx.diff(&[])), 2);
    // From a flag combination the parser cannot check.
    let report = fx.report();
    let go = fx.file("go.cover", "mode: set\nexample.com/m/a.go:1.1,2.2 1 1\n");
    let output = fx.diff(&["--report", path(&go), "--branch-coverage"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    // `merge` takes its output as a file, not a format.
    let output = run(fx.root(), &["merge", "-o", "json"], &[path(&report)]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(!fx.root().join("json").exists());
}

#[test]
fn an_unusable_report_is_3() {
    let fx = Fixture::new();
    let missing = fx.root().join("missing.lcov");
    let empty = fx.file("empty.lcov", "");
    let garbage = fx.file("garbage.lcov", "this is not a coverage report\n");
    for report in [&missing, &empty, &garbage] {
        let output = fx.diff(&["--report", path(report)]);
        assert_eq!(
            code(&output),
            3,
            "{}: {}",
            report.display(),
            stderr(&output)
        );
    }
    // An empty shard of a sharded run, and a missing input to `merge`.
    let good = fx.report();
    let no_lines = fx.file("no-lines.lcov", "SF:a.rs\nend_of_record\n");
    let output = fx.diff(&["--report", path(&good), "--report", path(&no_lines)]);
    assert_eq!(code(&output), 3, "{}", stderr(&output));
    let output = run(fx.root(), &["merge", "-o", "out.lcov"], &[path(&missing)]);
    assert_eq!(code(&output), 3, "{}", stderr(&output));
}

#[test]
fn a_malformed_marker_is_4() {
    let fx = Fixture::new();
    let report = fx.report();
    let marker = format!(
        "// {} ignore reason=\"unclosed\"\n",
        patchcov::markers::INTRODUCER
    );
    fx.file("a.rs", &format!("{marker}fn a() {{}}\n"));
    let output = fx.diff(&["--report", path(&report)]);
    assert_eq!(code(&output), 4, "{}", stderr(&output));

    let output = run(fx.root(), &["lint-markers"], &["a.rs"]);
    assert_eq!(code(&output), 4, "{}", stderr(&output));
    assert!(stderr(&output).contains("coverage marker lint failed"));
}

#[test]
fn bad_config_is_5() {
    let fx = Fixture::new();
    let report = fx.report();
    // Malformed YAML, for `diff` and for `lint-markers`.
    fx.file(".patchcov/config.yaml", "diff: [unclosed\n");
    let output = fx.diff(&["--report", path(&report)]);
    assert_eq!(code(&output), 5, "{}", stderr(&output));
    let output = run(fx.root(), &["lint-markers"], &[]);
    assert_eq!(code(&output), 5, "{}", stderr(&output));
    std::fs::remove_dir_all(fx.root().join(".patchcov")).unwrap();

    // A path mapping the report cannot be transformed with.
    fx.file(
        ".patchcov/config.yaml",
        "diff:\n  path-mappings:\n    - from: x\n      to: ../escape\n",
    );
    let output = fx.diff(&["--report", path(&report)]);
    assert_eq!(code(&output), 5, "{}", stderr(&output));
    std::fs::remove_dir_all(fx.root().join(".patchcov")).unwrap();

    // An invalid regex and an invalid glob, from flags.
    let output = fx.diff(&["--report", path(&report), "--ignore-filename-regex", "("]);
    assert_eq!(code(&output), 5, "{}", stderr(&output));
    let output = fx.diff(&["--report", path(&report), "--fail-on-unmeasured", "["]);
    assert_eq!(code(&output), 5, "{}", stderr(&output));
    let output = run(fx.root(), &["lint-markers", "--include", "["], &[]);
    assert_eq!(code(&output), 5, "{}", stderr(&output));
}

#[test]
fn git_failures_are_6() {
    let fx = Fixture::new();
    let report = fx.report();
    let output = run(
        fx.root(),
        &["diff", "--base-ref", "no-such-ref"],
        &["--report", path(&report)],
    );
    assert_eq!(code(&output), 6, "{}", stderr(&output));

    // Not a repository at all.
    let elsewhere = tempfile::tempdir().unwrap();
    let output = run(elsewhere.path(), &["diff"], &["--report", path(&report)]);
    assert_eq!(code(&output), 6, "{}", stderr(&output));
    let output = run(elsewhere.path(), &["lint-markers"], &[]);
    assert_eq!(code(&output), 6, "{}", stderr(&output));
}

#[test]
fn a_path_mismatch_is_7_unless_allowed() {
    let fx = Fixture::new();
    let report = fx.file(
        "elsewhere.lcov",
        "SF:nowhere/else.rs\nDA:1,1\nend_of_record\n",
    );
    let output = fx.diff(&["--report", path(&report)]);
    assert_eq!(code(&output), 7, "{}", stderr(&output));

    let output = fx.diff(&["--report", path(&report), "--allow-path-mismatch"]);
    assert_eq!(
        code(&output),
        0,
        "a warning, not a failure: {}",
        stderr(&output)
    );
    assert!(stderr(&output).contains("warning:"), "{}", stderr(&output));
}

#[test]
fn another_runtime_failure_is_8() {
    let fx = Fixture::new();
    let report = fx.report();
    // The output directory does not exist, so the write fails after the merge.
    let output = run(
        fx.root(),
        &["merge", "-o", "no-such-dir/merged.lcov"],
        &[path(&report)],
    );
    assert_eq!(code(&output), 8, "{}", stderr(&output));
}

#[test]
fn the_message_is_unchanged_by_the_code() {
    let fx = Fixture::new();
    let missing = fx.root().join("missing.lcov");
    let output = fx.diff(&["--report", path(&missing)]);
    let text = stderr(&output);
    let last = text.lines().last().unwrap();
    assert!(
        last.starts_with("Error: could not read coverage report "),
        "{text}"
    );
    // The cause is part of the one-line chain, as it always was.
    assert!(last.contains("missing.lcov: "), "{text}");
}
