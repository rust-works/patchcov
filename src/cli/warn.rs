//! Warnings and other diagnostic lines on stderr, in the format `--error-format`
//! selects.
//!
//! Every non-fatal problem patchcov reports goes through [`warn`], so a wrapper
//! that asked for `--error-format json` gets one JSON object per warning instead
//! of a `warning: ...` line. The object is described in
//! `docs/reference.md#warnings`. The other lines a command prints (the findings
//! of `lint-markers`, the summary of `merge`) go through [`emit`], which does the
//! same for an object of the command's own.
//!
//! A [`Warning`] holds the data that identifies its subject, and its
//! [`Display`](fmt::Display) renders the message from it, so the text and the
//! JSON fields cannot drift apart.

use std::fmt;
use std::sync::OnceLock;

use serde::ser::SerializeMap;
use serde::Serialize;

use super::ErrorFormat;
use crate::merge::PrefixMismatch;

/// The format warnings are printed in, set once by `main`.
static FORMAT: OnceLock<ErrorFormat> = OnceLock::new();

/// Sets the format [`warn`] prints in. Only the first call has an effect; without
/// one, warnings are text.
pub fn set_format(format: ErrorFormat) {
    let _ = FORMAT.set(format);
}

/// A deprecated flag that still parses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeprecatedFlag {
    /// `diff --format`, replaced by `-o/--output`.
    Format,
    /// `diff --fail-on-path-mismatch`, which a path mismatch no longer needs.
    FailOnPathMismatch,
}

impl DeprecatedFlag {
    /// The flag, with its dashes.
    const fn name(self) -> &'static str {
        match self {
            Self::Format => "--format",
            Self::FailOnPathMismatch => "--fail-on-path-mismatch",
        }
    }

    /// What replaces the flag, if anything.
    const fn replacement(self) -> Option<&'static str> {
        match self {
            Self::Format => Some("-o/--output"),
            Self::FailOnPathMismatch => None,
        }
    }
}

/// The fields of a `deprecated` warning, both derived from the flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deprecation(DeprecatedFlag);

impl Serialize for Deprecation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("flag", self.0.name())?;
        map.serialize_entry("replacement", &self.0.replacement())?;
        map.end()
    }
}

/// Where the globs of `lint-markers` came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum GlobOrigin {
    /// The `--include` flag.
    #[serde(rename = "--include")]
    IncludeFlag,
    /// `lint-markers.include` in `config.yaml`.
    #[serde(rename = "lint-markers.include")]
    ConfigInclude,
}

impl GlobOrigin {
    /// How a message names the origin; longer than the name in the JSON object.
    pub const fn described(self) -> &'static str {
        match self {
            Self::IncludeFlag => "--include",
            Self::ConfigInclude => "lint-markers.include in config.yaml",
        }
    }
}

/// A non-fatal problem, with the fields `--error-format json` adds to the
/// warning object (documented in `docs/reference.md#warnings`).
///
/// Serializing a warning gives only its fields; [`Warning::kind`] and the
/// message are the other keys of the object.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum Warning {
    /// A deprecated flag was used.
    Deprecated(Deprecation),
    /// A report's paths match no tracked file, and `--allow-path-mismatch` let the
    /// run continue.
    PathMismatch {
        /// The coverage report, as the message names it.
        report: String,
        /// How many file paths the report has.
        file_count: usize,
        /// The first few of its normalized paths, which the message samples.
        unmatched: Vec<String>,
    },
    /// A shard was measured under a different workspace root (`merge`).
    ShardRoot(PrefixMismatch),
    /// The globs of `lint-markers` matched no tracked file.
    GlobNoMatch {
        /// The globs, as given.
        globs: Vec<String>,
        /// Where the globs came from.
        origin: GlobOrigin,
    },
}

impl Warning {
    /// A warning about the deprecated `flag`.
    pub const fn deprecated(flag: DeprecatedFlag) -> Self {
        Self::Deprecated(Deprecation(flag))
    }

    /// The stable name of this class of warning, `kind` in the JSON warning
    /// object, listed in `docs/reference.md#warnings`.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Deprecated(_) => "deprecated",
            Self::PathMismatch { .. } => "path-mismatch",
            Self::ShardRoot(_) => "shard-root",
            Self::GlobNoMatch { .. } => "glob-no-match",
        }
    }
}

impl fmt::Display for Warning {
    /// The message: what follows `warning: ` in the default format.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deprecated(Deprecation(flag)) => match flag.replacement() {
                Some(replacement) => {
                    write!(f, "{} is deprecated; use {replacement} instead", flag.name())
                }
                None => write!(
                    f,
                    "{} is deprecated and has no effect; a path mismatch is an error unless --allow-path-mismatch or diff.allow-path-mismatch is set",
                    flag.name()
                ),
            },
            Self::PathMismatch {
                report,
                file_count,
                unmatched,
            } => {
                let sample = unmatched
                    .iter()
                    .map(|p| format!("`{p}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(
                    f,
                    "coverage report {report}: none of its {file_count} file path(s) matches a tracked file in the repository; unmatched normalized paths: {sample}; use --strip-prefix or diff.path-mappings to make paths repo-relative"
                )
            }
            Self::ShardRoot(mismatch) => mismatch.fmt(f),
            Self::GlobNoMatch { origin, .. } => write!(
                f,
                "no tracked file matches the globs in {}; no coverage markers were checked",
                origin.described()
            ),
        }
    }
}

/// The `level` of a warning object; an [`ErrorReport`](super::exit::ErrorReport)
/// has `"error"`.
const LEVEL: &str = "warning";

/// The JSON warning object, with its fields in the documented order.
#[derive(Serialize)]
struct WarningReport<'a> {
    level: &'static str,
    kind: &'static str,
    message: String,
    #[serde(flatten)]
    fields: &'a Warning,
}

/// Prints a warning to stderr, as a `warning: ...` line or a JSON object.
pub fn warn(warning: &Warning) {
    eprintln!(
        "{}",
        render(FORMAT.get().copied().unwrap_or_default(), warning)
    );
}

/// Prints a line to stderr: `text` in the default format, `record` as one JSON
/// object under `--error-format json`.
pub fn emit(text: impl AsRef<str>, record: &impl Serialize) {
    eprintln!(
        "{}",
        render_line(
            FORMAT.get().copied().unwrap_or_default(),
            text.as_ref(),
            record
        )
    );
}

/// The line `emit` prints.
fn render_line(format: ErrorFormat, text: &str, record: &impl Serialize) -> String {
    match format {
        ErrorFormat::Text => text.to_string(),
        // Records hold strings, numbers and options, which always serialize.
        ErrorFormat::Json => serde_json::to_string(record).unwrap_or_default(),
    }
}

/// The line `warn` prints.
fn render(format: ErrorFormat, warning: &Warning) -> String {
    match format {
        ErrorFormat::Text => format!("warning: {warning}"),
        ErrorFormat::Json => serde_json::to_string(&WarningReport {
            level: LEVEL,
            kind: warning.kind(),
            message: warning.to_string(),
            fields: warning,
        })
        // Strings, numbers and lists of strings always serialize.
        .unwrap_or_default(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::cli::doc_fields::{
        example_keys, first_table_fields, json_keys, kind_rows, reference_section,
    };

    fn path_mismatch() -> Warning {
        Warning::PathMismatch {
            report: "head.lcov".to_owned(),
            file_count: 4,
            unmatched: vec!["a/x.rs".to_owned(), "a/y.rs".to_owned()],
        }
    }

    fn glob_no_match(origin: GlobOrigin) -> Warning {
        Warning::GlobNoMatch {
            globs: vec!["**/*.nomatch".to_owned(), "docs/*.x".to_owned()],
            origin,
        }
    }

    fn shard_root() -> Warning {
        Warning::ShardRoot(PrefixMismatch {
            shard: "two.lcov".to_owned(),
            strip_prefix: "/ci/workspace".to_owned(),
            absolute_paths: 3,
        })
    }

    #[test]
    fn text_is_the_warning_line() {
        let line = render(
            ErrorFormat::Text,
            &Warning::deprecated(DeprecatedFlag::Format),
        );
        assert_eq!(
            line,
            "warning: --format is deprecated; use -o/--output instead"
        );
    }

    #[derive(Serialize)]
    struct Record {
        level: &'static str,
        n: Option<u32>,
        message: &'static str,
    }

    #[test]
    fn emit_text_is_the_text_untouched() {
        let record = Record {
            level: "info",
            n: None,
            message: "ignored",
        };
        assert_eq!(
            render_line(ErrorFormat::Text, "as is: 1", &record),
            "as is: 1"
        );
    }

    /// One line, the record's fields in declaration order, `None` as `null`.
    #[test]
    fn emit_json_is_the_record_on_one_line() {
        let record = Record {
            level: "info",
            n: None,
            message: "two\nlines",
        };
        let line = render_line(ErrorFormat::Json, "unused", &record);
        assert_eq!(line, r#"{"level":"info","n":null,"message":"two\nlines"}"#);
    }

    /// The messages are unchanged from before the warnings carried fields.
    #[test]
    fn messages_are_unchanged() {
        assert_eq!(
            Warning::deprecated(DeprecatedFlag::FailOnPathMismatch).to_string(),
            "--fail-on-path-mismatch is deprecated and has no effect; a path mismatch is an error unless --allow-path-mismatch or diff.allow-path-mismatch is set"
        );
        assert_eq!(
            path_mismatch().to_string(),
            "coverage report head.lcov: none of its 4 file path(s) matches a tracked file in the repository; unmatched normalized paths: `a/x.rs`, `a/y.rs`; use --strip-prefix or diff.path-mappings to make paths repo-relative"
        );
        assert!(shard_root().to_string().starts_with(
            "coverage shard two.lcov: none of its 3 absolute file path(s) is under `/ci/workspace`, so "
        ));
        assert_eq!(
            glob_no_match(GlobOrigin::IncludeFlag).to_string(),
            "no tracked file matches the globs in --include; no coverage markers were checked"
        );
        assert_eq!(
            glob_no_match(GlobOrigin::ConfigInclude).to_string(),
            "no tracked file matches the globs in lint-markers.include in config.yaml; no coverage markers were checked"
        );
    }

    /// One line, `level`, `kind` and `message` first, quotes and newlines escaped.
    #[test]
    fn json_is_one_line_in_field_order() {
        let warning = Warning::PathMismatch {
            report: "it said \"no\"\nreally".to_owned(),
            file_count: 1,
            unmatched: vec!["x".to_owned()],
        };
        let line = render(ErrorFormat::Json, &warning);
        assert!(!line.contains('\n'), "{line}");
        assert!(
            line.starts_with(r#"{"level":"warning","kind":"path-mismatch","message":"coverage report it said \"no\"\nreally:"#),
            "{line}"
        );
        assert!(
            line.ends_with(
                r#""report":"it said \"no\"\nreally","file_count":1,"unmatched":["x"]}"#
            ),
            "{line}"
        );
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["report"], "it said \"no\"\nreally");
    }

    /// The fields of each kind are a contract with wrappers; this is the table in
    /// the docs.
    #[test]
    fn each_kind_has_its_fields() {
        let cases = [
            (
                Warning::deprecated(DeprecatedFlag::Format),
                "deprecated",
                r#"{"flag":"--format","replacement":"-o/--output"}"#,
            ),
            (
                Warning::deprecated(DeprecatedFlag::FailOnPathMismatch),
                "deprecated",
                r#"{"flag":"--fail-on-path-mismatch","replacement":null}"#,
            ),
            (
                path_mismatch(),
                "path-mismatch",
                r#"{"report":"head.lcov","file_count":4,"unmatched":["a/x.rs","a/y.rs"]}"#,
            ),
            (
                shard_root(),
                "shard-root",
                r#"{"shard":"two.lcov","strip_prefix":"/ci/workspace","absolute_paths":3}"#,
            ),
            (
                glob_no_match(GlobOrigin::IncludeFlag),
                "glob-no-match",
                r#"{"globs":["**/*.nomatch","docs/*.x"],"origin":"--include"}"#,
            ),
            (
                glob_no_match(GlobOrigin::ConfigInclude),
                "glob-no-match",
                r#"{"globs":["**/*.nomatch","docs/*.x"],"origin":"lint-markers.include"}"#,
            ),
        ];
        for (warning, kind, fields) in cases {
            assert_eq!(warning.kind(), kind, "{warning:?}");
            assert_eq!(serde_json::to_string(&warning).unwrap(), fields);
            // The object is `level`, `kind`, `message`, then exactly these fields.
            let object: serde_json::Value =
                serde_json::from_str(&render(ErrorFormat::Json, &warning)).unwrap();
            let expected: serde_json::Value = serde_json::from_str(fields).unwrap();
            let mut keys: Vec<_> = object.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            let mut want: Vec<_> = expected.as_object().unwrap().keys().cloned().collect();
            want.extend(["level", "kind", "message"].map(String::from));
            want.sort();
            assert_eq!(keys, want, "{warning:?}");
            assert_eq!(object["level"], "warning");
            assert_eq!(object["kind"], kind);
            assert_eq!(object["message"], warning.to_string());
            for (key, value) in expected.as_object().unwrap() {
                assert_eq!(&object[key], value, "{key}");
            }
        }
    }

    /// Every kind has the fields the "Warnings" tables in `docs/reference.md` list,
    /// in that order, and the docs list no kind the code does not have.
    #[test]
    fn each_kind_matches_the_docs() {
        let section = reference_section("Warnings");
        let common = ["level", "kind", "message"];
        assert_eq!(first_table_fields(section), common);

        let warnings = [
            Warning::deprecated(DeprecatedFlag::Format),
            Warning::deprecated(DeprecatedFlag::FailOnPathMismatch),
            path_mismatch(),
            shard_root(),
            glob_no_match(GlobOrigin::IncludeFlag),
            glob_no_match(GlobOrigin::ConfigInclude),
        ];
        let documented = kind_rows(section);
        let mut kinds: Vec<_> = warnings.iter().map(Warning::kind).collect();
        kinds.dedup();
        let listed: Vec<_> = documented.iter().map(|(kind, _)| kind.as_str()).collect();
        assert_eq!(listed, kinds, "the kinds in the docs table");

        let mut examples = 0;
        for warning in &warnings {
            let kind = warning.kind();
            let keys = json_keys(&render(ErrorFormat::Json, warning));
            let fields = &documented.iter().find(|(k, _)| k == kind).unwrap().1;
            let want: Vec<_> = common
                .iter()
                .map(ToString::to_string)
                .chain(fields.iter().cloned())
                .collect();
            assert_eq!(keys, want, "{kind}: the docs table");
            // The docs show an example for some kinds only.
            if let Some(example) = example_keys(section, kind) {
                examples += 1;
                assert_eq!(example, want, "{kind}: the docs example");
            }
        }
        assert!(examples > 0, "the docs show no example warning line");
    }
}
