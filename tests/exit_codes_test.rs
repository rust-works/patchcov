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
        .env_remove("PATCHCOV_ERROR_FORMAT")
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
    // A non-finite threshold would disable the gate (`nan`) or have no JSON form (`inf`).
    let report = fx.report();
    for (flag, value) in [("--fail-under-patch", "nan"), ("--fail-under-lines", "inf")] {
        let output = fx.diff(&["--report", path(&report), flag, value]);
        assert_eq!(code(&output), 2, "{flag} {value}: {}", stderr(&output));
    }
    // From a flag combination the parser cannot check.
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

/// The last line of stderr, parsed as the JSON error report.
fn report(output: &Output) -> serde_json::Value {
    let text = stderr(output);
    let last = text.lines().last().unwrap_or_default();
    serde_json::from_str(last).unwrap_or_else(|err| panic!("not JSON ({err}): {text}"))
}

/// Runs `args` twice, in the default format and with `--error-format json`, and
/// checks that only the shape of the failure on stderr differs.
fn assert_json_error(fx: &Fixture, args: &[&str], code: i32, kind: &str) {
    let plain = run(fx.root(), args, &[]);
    let mut with_flag: Vec<&str> = args.to_vec();
    with_flag.extend(["--error-format", "json"]);
    let json = run(fx.root(), &with_flag, &[]);

    assert_eq!(self::code(&plain), code, "{args:?}: {}", stderr(&plain));
    assert_eq!(self::code(&json), code, "{args:?}: {}", stderr(&json));
    assert_eq!(plain.stdout, json.stdout, "{args:?}: stdout is unchanged");

    let value = report(&json);
    assert_eq!(value["code"], code, "{args:?}: {value}");
    assert_eq!(value["kind"], kind, "{args:?}: {value}");
    let object = value.as_object().unwrap();
    // `gates` is for a gate failure only.
    let expected: &[&str] = if kind == "gate" {
        &["chain", "code", "gates", "kind", "level", "message"]
    } else {
        &["chain", "code", "kind", "level", "message"]
    };
    assert_eq!(object.keys().collect::<Vec<_>>(), expected, "{args:?}");
    let chain: Vec<&str> = value["chain"]
        .as_array()
        .unwrap()
        .iter()
        .map(|cause| cause.as_str().unwrap())
        .collect();
    let message = value["message"].as_str().unwrap();
    assert!(!message.is_empty(), "{args:?}");

    // The same text as the default format, which is still what it was.
    let mut joined = vec![message];
    joined.extend(chain);
    let plain_text = stderr(&plain);
    if kind == "usage" && !plain_text.contains("Error: ") {
        // clap's own error: the message is its first paragraph.
        assert!(plain_text.contains(message), "{args:?}: {plain_text}");
    } else {
        let expected = format!("Error: {}", joined.join(": "));
        // A message may itself span lines, so compare the whole tail.
        assert!(
            plain_text.trim_end().ends_with(&expected),
            "{args:?}: {plain_text}"
        );
    }
    // One object, as the last line, and no `Error:` line in its place.
    assert!(
        !stderr(&json).contains("Error: "),
        "{args:?}: {}",
        stderr(&json)
    );
}

#[test]
fn json_errors_name_each_class() {
    let fx = Fixture::new();
    let report = fx.report();
    let report = path(&report);
    let base = fx.base.clone();
    let diff = |extra: &[&'static str]| -> Vec<String> {
        let mut args = vec!["diff".to_string(), "--base-ref".into(), base.clone()];
        args.extend(extra.iter().map(ToString::to_string));
        args
    };
    let check = |args: Vec<String>, code: i32, kind: &str| {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        assert_json_error(&fx, &args, code, kind);
    };

    // gate
    let mut args = diff(&["--fail-under-patch", "80"]);
    args.extend(["--report".into(), report.to_string()]);
    check(args, 1, "gate");
    // usage: from the argument parser, and from a flag combination it cannot check
    check(diff(&[]), 2, "usage");
    let go = fx.file("go.cover", "mode: set\nexample.com/m/a.go:1.1,2.2 1 1\n");
    let mut args = diff(&["--branch-coverage"]);
    args.extend(["--report".into(), path(&go).to_string()]);
    check(args, 2, "usage");
    // report
    let mut args = diff(&[]);
    args.extend(["--report".into(), "missing.lcov".to_string()]);
    check(args, 3, "report");
    // marker
    let marker = format!(
        "// {} ignore reason=\"unclosed\"\n",
        patchcov::markers::INTRODUCER
    );
    fx.file("m.rs", &format!("{marker}fn m() {{}}\n"));
    check(vec!["lint-markers".into(), "m.rs".into()], 4, "marker");
    // config
    let mut args = diff(&["--ignore-filename-regex", "("]);
    args.extend(["--report".into(), report.to_string()]);
    check(args, 5, "config");
    // git
    let mut args = vec![
        "diff".to_string(),
        "--base-ref".into(),
        "no-such-ref".into(),
    ];
    args.extend(["--report".into(), report.to_string()]);
    check(args, 6, "git");
    // path-mismatch
    let elsewhere = fx.file(
        "elsewhere.lcov",
        "SF:nowhere/else.rs\nDA:1,1\nend_of_record\n",
    );
    let mut args = diff(&[]);
    args.extend(["--report".into(), path(&elsewhere).to_string()]);
    check(args, 7, "path-mismatch");
    // other
    let args = vec![
        "merge".to_string(),
        report.to_string(),
        "-o".into(),
        "no-such-dir/merged.lcov".into(),
    ];
    check(args, 8, "other");
}

#[test]
fn a_json_error_carries_the_causes() {
    let fx = Fixture::new();
    let missing = fx.root().join("missing.lcov");
    let output = fx.diff(&["--report", path(&missing), "--error-format", "json"]);
    let value = report(&output);
    assert!(
        value["message"]
            .as_str()
            .unwrap()
            .starts_with("could not read coverage report "),
        "{value}"
    );
    assert!(
        value["chain"][0].as_str().unwrap().contains("os error 2"),
        "{value}"
    );
}

#[test]
fn a_gate_failure_still_prints_the_report_to_stdout() {
    let fx = Fixture::new();
    let report = fx.report();
    let output = fx.diff(&[
        "--report",
        path(&report),
        "--fail-under-patch",
        "80",
        "--error-format=json",
    ]);
    assert_eq!(code(&output), 1);
    assert!(!output.stdout.is_empty());
    assert!(self::report(&output)["message"]
        .as_str()
        .unwrap()
        .contains("--fail-under-patch"));
}

/// The data behind the message: every failed gate with its threshold and measured
/// value, so a wrapper does not have to parse the text.
#[test]
fn a_json_gate_error_has_the_gates_as_data() {
    let fx = Fixture::new();
    let lcov = fx.report();
    let output = fx.diff(&[
        "--report",
        path(&lcov),
        "--fail-under-patch",
        "80",
        "--fail-under-lines",
        "75.5",
        "--fail-on-unmeasured",
        "b.rs",
        "--error-format",
        "json",
    ]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    let value = report(&output);
    assert_eq!(
        value["gates"],
        serde_json::json!([
            {"gate": "fail-under-patch", "threshold": 80.0, "measured": 50.0},
            {"gate": "fail-under-lines", "threshold": 75.5, "measured": 50.0},
            {"gate": "fail-on-unmeasured", "files": ["b.rs"]},
        ]),
        "{value}"
    );
    // The message is the same text the default format prints, built from the data.
    let text = fx.diff(&[
        "--report",
        path(&lcov),
        "--fail-under-patch",
        "80",
        "--fail-under-lines",
        "75.5",
        "--fail-on-unmeasured",
        "b.rs",
    ]);
    assert_eq!(
        stderr(&text),
        format!("Error: {}\n", value["message"].as_str().unwrap())
    );
    assert!(value["chain"].as_array().unwrap().is_empty());
}

#[test]
fn an_unmeasurable_line_total_is_a_null_measured_value() {
    let fx = Fixture::new();
    let lcov = fx.report();
    let output = fx.diff(&[
        "--report",
        path(&lcov),
        "--ignore-filename-regex",
        r"a\.rs",
        "--fail-under-lines",
        "1",
        "--error-format",
        "json",
    ]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert_eq!(
        report(&output)["gates"],
        serde_json::json!([{"gate": "fail-under-lines", "threshold": 1.0, "measured": null}])
    );
}

#[test]
fn only_the_gates_that_failed_are_listed() {
    let fx = Fixture::new();
    let lcov = fx.report();
    let output = fx.diff(&[
        "--report",
        path(&lcov),
        "--fail-under-patch",
        "50",
        "--fail-under-lines",
        "60",
        "--error-format",
        "json",
    ]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    let gates = report(&output)["gates"].clone();
    assert_eq!(
        gates,
        serde_json::json!([{"gate": "fail-under-lines", "threshold": 60.0, "measured": 50.0}])
    );
}

#[test]
fn the_environment_variable_sets_the_format_and_the_flag_wins() {
    let fx = Fixture::new();
    let missing = fx.root().join("missing.lcov");
    let args = ["diff", "--report", path(&missing)];
    let with_env = |value: &str, extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_patchcov"))
            .env("PATCHCOV_ERROR_FORMAT", value)
            .env("GIT_CEILING_DIRECTORIES", fx.root().parent().unwrap())
            .arg("-C")
            .arg(fx.root())
            .args(args)
            .args(extra)
            .output()
            .unwrap()
    };

    let output = with_env("json", &[]);
    assert_eq!(code(&output), 3);
    assert_eq!(report(&output)["kind"], "report");

    let output = with_env("json", &["--error-format", "text"]);
    assert!(
        stderr(&output).starts_with("Error: "),
        "{}",
        stderr(&output)
    );

    // A value that is neither is ignored with a warning, not a failure of its own.
    let output = with_env("xml", &[]);
    assert_eq!(code(&output), 3);
    let text = stderr(&output);
    assert!(
        text.contains("warning: ignoring PATCHCOV_ERROR_FORMAT=xml"),
        "{text}"
    );
    assert!(
        text.lines().last().unwrap().starts_with("Error: "),
        "{text}"
    );

    // Empty counts as unset.
    let output = with_env("", &[]);
    assert_eq!(code(&output), 3);
    assert!(
        stderr(&output).starts_with("Error: "),
        "{}",
        stderr(&output)
    );
}

#[test]
fn the_flag_is_accepted_before_the_subcommand() {
    let fx = Fixture::new();
    let missing = fx.root().join("missing.lcov");
    let output = run(
        fx.root(),
        &["--error-format=json", "diff", "--report", path(&missing)],
        &[],
    );
    assert_eq!(report(&output)["kind"], "report");
}

#[test]
fn argument_parser_errors_are_json_too() {
    let fx = Fixture::new();
    let report_missing = fx.diff(&["--error-format", "json"]);
    assert_eq!(code(&report_missing), 2);
    let value = report(&report_missing);
    assert_eq!(value["kind"], "usage");
    assert_eq!(value["chain"].as_array().unwrap().len(), 0);
    let message = value["message"].as_str().unwrap();
    assert!(message.contains("--report"), "{message}");
    assert!(!message.contains("Usage:"), "{message}");
    assert_eq!(stderr(&report_missing).lines().count(), 1);

    // An unknown subcommand, with the format from the flag in either spelling
    // and from the environment.
    for args in [
        &["nonsense", "--error-format=json"][..],
        &["--error-format", "json", "nonsense"][..],
    ] {
        let output = run(fx.root(), args, &[]);
        assert_eq!(code(&output), 2);
        assert_eq!(report(&output)["kind"], "usage", "{args:?}");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_patchcov"))
        .env("PATCHCOV_ERROR_FORMAT", "json")
        .arg("nonsense")
        .output()
        .unwrap();
    assert_eq!(code(&output), 2);
    assert_eq!(report(&output)["kind"], "usage");

    // A bad value for the flag itself is a plain parser error.
    let output = run(fx.root(), &["diff", "--error-format", "xml"], &[]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).starts_with("error: "),
        "{}",
        stderr(&output)
    );
}

#[test]
fn help_and_version_are_not_errors() {
    let fx = Fixture::new();
    for flag in ["--help", "--version"] {
        let output = run(fx.root(), &[flag, "--error-format=json"], &[]);
        assert_eq!(code(&output), 0, "{flag}");
        assert!(!output.stdout.is_empty(), "{flag}");
        assert!(output.stderr.is_empty(), "{flag}: {}", stderr(&output));
    }
}

// ── warnings ─────────────────────────────────────────────────────

/// The `warning: ...` lines of a run in the default format, without the prefix.
fn text_warnings(output: &Output) -> Vec<String> {
    stderr(output)
        .lines()
        .filter_map(|line| line.strip_prefix("warning: "))
        .map(str::to_owned)
        .collect()
}

/// The JSON warning objects of a run, which must be the only `warning` lines.
fn json_warnings(output: &Output) -> Vec<serde_json::Value> {
    let text = stderr(output);
    assert!(!text.contains("warning: "), "a text warning: {text}");
    text.lines()
        .filter(|line| line.starts_with('{'))
        .map(|line| serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}: {line}")))
        .filter(|object: &serde_json::Value| object["level"] == "warning")
        .collect()
}

/// Runs `head` and `args` in both formats and checks that the single warning is
/// the same message, of the given `kind`, with exactly the `fields` after
/// `level`, `kind` and `message`, and that the run succeeds.
fn assert_json_warning(
    fx: &Fixture,
    head: &[&str],
    args: &[&str],
    kind: &str,
    fields: serde_json::Value,
) {
    let plain = run(fx.root(), head, args);
    let mut flagged: Vec<&str> = head.to_vec();
    flagged.extend(["--error-format", "json"]);
    let json = run(fx.root(), &flagged, args);

    assert_eq!(code(&plain), 0, "{head:?}: {}", stderr(&plain));
    assert_eq!(code(&json), 0, "{head:?}: {}", stderr(&json));
    let text = text_warnings(&plain);
    let objects = json_warnings(&json);
    assert_eq!(text.len(), 1, "{head:?}: {}", stderr(&plain));
    assert_eq!(objects.len(), 1, "{head:?}: {}", stderr(&json));
    assert_eq!(objects[0]["kind"], kind, "{head:?}");
    assert_eq!(objects[0]["message"], text[0], "{head:?}");
    let mut expected = serde_json::json!({
        "level": "warning",
        "kind": kind,
        "message": text[0],
    });
    expected
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    assert_eq!(objects[0], expected, "{head:?}");
}

#[test]
fn an_allowed_path_mismatch_is_a_json_warning() {
    let fx = Fixture::new();
    let report = fx.file(
        "elsewhere.lcov",
        "SF:nowhere/else.rs\nDA:1,1\nend_of_record\n",
    );
    assert_json_warning(
        &fx,
        &["diff", "--base-ref", &fx.base],
        &["--report", path(&report), "--allow-path-mismatch"],
        "path-mismatch",
        serde_json::json!({
            "report": path(&report),
            "file_count": 1,
            "unmatched": ["nowhere/else.rs"],
        }),
    );
}

#[test]
fn a_deprecated_flag_is_a_json_warning() {
    let fx = Fixture::new();
    let report = fx.report();
    let cases = [
        (
            &["--format", "markdown"][..],
            serde_json::json!({"flag": "--format", "replacement": "-o/--output"}),
        ),
        (
            &["--fail-on-path-mismatch"],
            serde_json::json!({"flag": "--fail-on-path-mismatch", "replacement": null}),
        ),
    ];
    for (flag, fields) in cases {
        let mut args = vec!["--report", path(&report)];
        args.extend(flag);
        assert_json_warning(
            &fx,
            &["diff", "--base-ref", &fx.base],
            &args,
            "deprecated",
            fields,
        );
    }
}

#[test]
fn a_shard_under_another_root_is_a_json_warning() {
    let fx = Fixture::new();
    let shard = fx.file(
        "shard.lcov",
        "SF:/other/runner/a.rs\nDA:1,1\nend_of_record\n",
    );
    assert_json_warning(
        &fx,
        &[
            "merge",
            "--strip-prefix",
            "/ci/workspace",
            "-o",
            "merged.lcov",
        ],
        &[path(&shard)],
        "shard-root",
        serde_json::json!({
            "shard": path(&shard),
            "strip_prefix": "/ci/workspace",
            "absolute_paths": 1,
        }),
    );
}

#[test]
fn a_glob_that_matches_nothing_is_a_json_warning() {
    let fx = Fixture::new();
    assert_json_warning(
        &fx,
        &["lint-markers", "--include", "**/*.nomatch"],
        &[],
        "glob-no-match",
        serde_json::json!({"globs": ["**/*.nomatch"], "origin": "--include"}),
    );
}

#[test]
fn globs_from_the_config_say_so() {
    let fx = Fixture::new();
    fx.file(
        ".patchcov/config.yaml",
        "lint-markers:\n  include:\n    - \"**/*.nomatch\"\n    - \"*.also\"\n",
    );
    assert_json_warning(
        &fx,
        &["lint-markers"],
        &[],
        "glob-no-match",
        serde_json::json!({
            "globs": ["**/*.nomatch", "*.also"],
            "origin": "lint-markers.include",
        }),
    );
}

#[test]
fn the_error_follows_the_warnings_and_is_told_apart_by_level() {
    let fx = Fixture::new();
    let measured = fx.report();
    // Two deprecated flags warn; the gate then fails at 50%.
    let output = fx.diff(&[
        "--report",
        path(&measured),
        "--format",
        "markdown",
        "--fail-on-path-mismatch",
        "--fail-under-patch",
        "80",
        "--error-format",
        "json",
    ]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    let text = stderr(&output);
    let levels: Vec<_> = text
        .lines()
        .map(|line| {
            let object: serde_json::Value =
                serde_json::from_str(line).unwrap_or_else(|err| panic!("not JSON ({err}): {line}"));
            object["level"].clone()
        })
        .collect();
    assert_eq!(levels, ["warning", "warning", "error"], "{text}");
    let error = report(&output);
    assert_eq!(error["kind"], "gate");
}

#[test]
fn the_environment_variable_turns_warnings_into_json_too() {
    let fx = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_patchcov"))
        .env_remove("PATCHCOV_CONFIG_DIR")
        .env("PATCHCOV_ERROR_FORMAT", "json")
        .env("GIT_CEILING_DIRECTORIES", fx.root().parent().unwrap())
        .args([
            "-C",
            path(fx.root()),
            "lint-markers",
            "--include",
            "*.nomatch",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(json_warnings(&output).len(), 1, "{}", stderr(&output));
}

#[test]
fn an_invalid_format_variable_warns_in_text() {
    let fx = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_patchcov"))
        .env_remove("PATCHCOV_CONFIG_DIR")
        .env("PATCHCOV_ERROR_FORMAT", "xml")
        .env("GIT_CEILING_DIRECTORIES", fx.root().parent().unwrap())
        .args(["-C", path(fx.root()), "lint-markers"])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stderr(&output).starts_with("warning: ignoring PATCHCOV_ERROR_FORMAT=xml"),
        "{}",
        stderr(&output)
    );
}

/// Under `--error-format json` the `merge` summary is an `info` object with the
/// counts as fields, and the default format still prints the sentence.
#[test]
fn the_merge_summary_is_a_json_info_line() {
    let fx = Fixture::new();
    let report = fx.report();
    let out = fx.root().join("merged.lcov");

    let output = run(
        fx.root(),
        &["merge", "--error-format", "json", "-o", path(&out)],
        &[path(&report)],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stderr(&output);
    assert_eq!(text.lines().count(), 1, "{text}");
    let summary: serde_json::Value = serde_json::from_str(text.trim_end()).unwrap();
    assert_eq!(summary["level"], "info");
    assert_eq!(summary["kind"], "merge-summary");
    assert_eq!(summary["inputs"], 1);
    assert_eq!(summary["files"], 1);
    assert_eq!(summary["total_lines"], 4);
    assert_eq!(summary["covered_lines"], 2);
    assert_eq!(summary["percent"], 50.0);
    assert!(summary["output"].as_str().unwrap().ends_with("merged.lcov"));

    let plain = run(fx.root(), &["merge", "-o", path(&out)], &[path(&report)]);
    let line = stderr(&plain);
    assert!(
        line.starts_with("merged 1 report(s) into ") && line.contains("2 of 4 lines covered (50"),
        "{line}"
    );
    assert_eq!(
        line.trim_end(),
        summary["message"].as_str().unwrap(),
        "the object's message is the text line"
    );
}
