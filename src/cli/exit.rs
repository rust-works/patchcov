//! Process exit codes: one per class of failure.
//!
//! A command fails with an [`anyhow::Error`]. The CLI layer tags the failures it
//! can tell apart with an [`ExitKind`] (see [`Classify`]), and `main` turns the tag
//! into the process exit code with [`code`]. The tag is invisible in the message,
//! so what is printed to stderr does not depend on it by default. With
//! `--error-format json` the same tag is printed as an [`ErrorReport`].

use std::error::Error as StdError;
use std::fmt;

use serde::Serialize;

use super::warn::json_line;

/// A class of failure, and the exit code it ends the process with.
///
/// `2` is also what the argument parser exits with, so it is the code of every
/// usage error. The values are part of the command-line contract and listed in
/// `docs/reference.md#exit-codes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[non_exhaustive]
pub enum ExitKind {
    /// A gate failed: `--fail-under-patch`, `--fail-under-lines` or
    /// `--fail-on-unmeasured`.
    Gate = 1,
    /// A usage error the argument parser cannot see, such as a flag combination
    /// that makes no sense.
    Usage = 2,
    /// A coverage report is unreadable, empty or unparseable.
    Report = 3,
    /// A coverage marker in the source is malformed.
    Marker = 4,
    /// The config file, a path mapping, an ignore regex or a glob is bad.
    Config = 5,
    /// Git could not answer: no repository, an unresolvable ref, no merge base.
    Git = 6,
    /// A report's paths match no tracked file, unless `--allow-path-mismatch` is set.
    PathMismatch = 7,
    /// Any other runtime failure, such as an I/O error writing the output.
    Other = 8,
}

impl ExitKind {
    /// The process exit code for this class of failure.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// The stable name of this class in the JSON error report, listed in
    /// `docs/reference.md#error-output`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gate => "gate",
            Self::Usage => "usage",
            Self::Report => "report",
            Self::Marker => "marker",
            Self::Config => "config",
            Self::Git => "git",
            Self::PathMismatch => "path-mismatch",
            Self::Other => "other",
        }
    }

    /// A new error with this class and `message`, for a check that fails outright.
    pub fn error(
        self,
        message: impl fmt::Display + fmt::Debug + Send + Sync + 'static,
    ) -> anyhow::Error {
        anyhow::Error::new(ExitError {
            kind: self,
            source: anyhow::Error::msg(message),
            gates: Vec::new(),
        })
    }
}

/// An error tagged with the [`ExitKind`] that decides the exit code.
///
/// It displays as the error it wraps and continues that error's chain, so
/// `{err:#}` prints the same text with or without the tag. The wrapped error
/// itself is no longer reachable by downcast, only its causes are.
#[derive(Debug)]
pub struct ExitError {
    kind: ExitKind,
    source: anyhow::Error,
    gates: Vec<GateFailure>,
}

impl ExitError {
    /// The class of failure.
    pub const fn kind(&self) -> ExitKind {
        self.kind
    }

    /// The gates that failed, for a [`ExitKind::Gate`] error built with
    /// [`gate_error`]; empty for every other error.
    pub fn gates(&self) -> &[GateFailure] {
        &self.gates
    }
}

impl fmt::Display for ExitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.source, f)
    }
}

impl StdError for ExitError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        // `anyhow::Error` derefs to `dyn Error`; skip it so it is not named twice.
        let inner: &(dyn StdError + Send + Sync + 'static) = self.source.as_ref();
        inner.source()
    }
}

/// The outermost tag on `err`, if any.
fn tag_of(err: &anyhow::Error) -> Option<&ExitError> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<ExitError>())
}

/// The class `err` was tagged with, if any.
fn kind_of(err: &anyhow::Error) -> Option<ExitKind> {
    tag_of(err).map(ExitError::kind)
}

/// The class of `err`: its tag, or [`ExitKind::Other`] when it has none.
pub fn kind(err: &anyhow::Error) -> ExitKind {
    kind_of(err).unwrap_or(ExitKind::Other)
}

/// The exit code for `err`: its tag, or [`ExitKind::Other`] when it has none.
pub fn code(err: &anyhow::Error) -> u8 {
    kind(err).code()
}

/// The `level` of an [`ErrorReport`].
const LEVEL: &str = "error";

/// One failed coverage gate, with the numbers behind it.
///
/// The [`Display`](fmt::Display) form is the message printed in the default
/// error format; the serialized form is an element of `gates` in the JSON error
/// report, documented in `docs/reference.md#error-output`. The measured values
/// are not rounded, so comparing one with its threshold gives the answer the
/// gate gave; only the message rounds them, to two decimal places.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "gate", rename_all = "kebab-case")]
pub enum GateFailure {
    /// `--fail-under-patch`: patch coverage is below the threshold.
    FailUnderPatch {
        /// The `--fail-under-patch` threshold.
        threshold: f64,
        /// The measured patch coverage.
        measured: f64,
    },
    /// `--fail-under-lines`: overall line coverage is below the threshold, or
    /// the report has no executable lines.
    FailUnderLines {
        /// The `--fail-under-lines` threshold.
        threshold: f64,
        /// The measured line coverage; `None` (`null`) when no executable line
        /// is left to measure.
        measured: Option<f64>,
    },
    /// `--fail-on-unmeasured` / `diff.require-measured`: touched files that match
    /// the policy are absent from every coverage report.
    FailOnUnmeasured {
        /// The matching touched files, in path order.
        files: Vec<String>,
    },
}

impl fmt::Display for GateFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FailUnderPatch {
                threshold,
                measured,
            } => write!(
                f,
                "patch coverage {measured:.2}% is below the --fail-under-patch threshold of {threshold:.2}%"
            ),
            Self::FailUnderLines {
                threshold,
                measured: Some(pct),
            } => write!(
                f,
                "line coverage {pct:.2}% is below the --fail-under-lines threshold of {threshold:.2}%"
            ),
            Self::FailUnderLines {
                threshold,
                measured: None,
            } => write!(
                f,
                "the report has no executable lines, so the --fail-under-lines threshold of {threshold:.2}% cannot be met"
            ),
            Self::FailOnUnmeasured { files } => write!(
                f,
                "touched files absent from every coverage report (--fail-on-unmeasured / diff.require-measured): {}",
                files.join(", ")
            ),
        }
    }
}

/// A [`ExitKind::Gate`] error for `failures`, which must not be empty.
///
/// The message names every failed gate, joined by `; `. The failures stay on the
/// error, for the `gates` field of the JSON report.
pub fn gate_error(failures: Vec<GateFailure>) -> anyhow::Error {
    debug_assert!(!failures.is_empty(), "a gate error names a failed gate");
    let message = failures
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    anyhow::Error::new(ExitError {
        kind: ExitKind::Gate,
        source: anyhow::Error::msg(message),
        gates: failures,
    })
}

/// A failure as one machine-readable object, printed to stderr by
/// `--error-format json`. The fields are part of the command-line contract and
/// documented in `docs/reference.md#error-output`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ErrorReport {
    /// Always `"error"`, so a consumer can tell this object from a warning line.
    pub level: &'static str,
    /// The process exit code.
    pub code: u8,
    /// The stable name of the class of failure, see [`ExitKind::name`].
    pub kind: &'static str,
    /// The outermost error message.
    pub message: String,
    /// The causes beneath `message`, outermost first. `message` followed by
    /// these, joined with `: `, is the text after `Error: ` in the default format.
    pub chain: Vec<String>,
    /// For a failed gate, the gates that failed, each with its threshold and
    /// measured value. Absent for every other class of failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gates: Option<Vec<GateFailure>>,
}

impl ErrorReport {
    /// The report for `err`.
    pub fn new(err: &anyhow::Error) -> Self {
        let kind = kind(err);
        let mut messages = err.chain().map(ToString::to_string);
        Self {
            level: LEVEL,
            code: kind.code(),
            kind: kind.name(),
            message: messages.next().unwrap_or_default(),
            chain: messages.collect(),
            gates: tag_of(err)
                .map(ExitError::gates)
                .filter(|gates| !gates.is_empty())
                .map(<[GateFailure]>::to_vec),
        }
    }

    /// The report for an error the argument parser found, which has no chain.
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            level: LEVEL,
            code: ExitKind::Usage.code(),
            kind: ExitKind::Usage.name(),
            message: message.into(),
            chain: Vec::new(),
            gates: None,
        }
    }

    /// The report as a single line of JSON.
    pub fn to_json(&self) -> String {
        json_line(self, || self.fallback())
    }

    /// The report without `chain` and `gates`, for a line that cannot be
    /// serialized in full: the exit code, class and message still reach a wrapper.
    fn fallback(&self) -> serde_json::Value {
        serde_json::json!({
            "level": self.level,
            "code": self.code,
            "kind": self.kind,
            "message": self.message,
        })
    }
}

/// Tags a failure with its [`ExitKind`].
pub trait Classify<T> {
    /// Tags the error, unless something deeper already did: the innermost class is
    /// the most specific one, so a caller can wrap broadly without overwriting it.
    fn classify(self, kind: ExitKind) -> anyhow::Result<T>;
}

impl<T, E> Classify<T> for Result<T, E>
where
    E: Into<anyhow::Error>,
{
    fn classify(self, kind: ExitKind) -> anyhow::Result<T> {
        self.map_err(|err| {
            let err = err.into();
            if kind_of(&err).is_some() {
                err
            } else {
                anyhow::Error::new(ExitError {
                    kind,
                    source: err,
                    gates: Vec::new(),
                })
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use anyhow::{anyhow, Context};

    use super::*;

    #[test]
    fn error_builds_a_tagged_error() {
        let err = ExitKind::Gate.error("it failed");
        assert_eq!(code(&err), 1);
        assert_eq!(format!("{err:#}"), "it failed");
    }

    fn fail(kind: ExitKind) -> anyhow::Error {
        Err::<(), _>(anyhow!("boom")).classify(kind).unwrap_err()
    }

    /// The numbers are a contract with scripts; this is the table in the docs.
    #[test]
    fn codes_are_stable() {
        let expected = [
            (ExitKind::Gate, 1),
            (ExitKind::Usage, 2),
            (ExitKind::Report, 3),
            (ExitKind::Marker, 4),
            (ExitKind::Config, 5),
            (ExitKind::Git, 6),
            (ExitKind::PathMismatch, 7),
            (ExitKind::Other, 8),
        ];
        for (kind, number) in expected {
            assert_eq!(kind.code(), number, "{kind:?}");
            assert_eq!(code(&fail(kind)), number, "{kind:?}");
        }
    }

    /// The names are a contract with scripts; this is the table in the docs.
    #[test]
    fn names_are_stable() {
        let expected = [
            (ExitKind::Gate, "gate"),
            (ExitKind::Usage, "usage"),
            (ExitKind::Report, "report"),
            (ExitKind::Marker, "marker"),
            (ExitKind::Config, "config"),
            (ExitKind::Git, "git"),
            (ExitKind::PathMismatch, "path-mismatch"),
            (ExitKind::Other, "other"),
        ];
        for (kind, name) in expected {
            assert_eq!(kind.name(), name, "{kind:?}");
        }
    }

    #[test]
    fn a_report_names_the_class_message_and_causes() {
        let err = Err::<(), _>(std::io::Error::other("disk on fire"))
            .context("could not read it")
            .classify(ExitKind::Report)
            .unwrap_err();
        let err = Err::<(), _>(err).context("outer").unwrap_err();
        let report = ErrorReport::new(&err);
        assert_eq!(report.code, 3);
        assert_eq!(report.kind, "report");
        assert_eq!(report.message, "outer");
        assert_eq!(report.chain, ["could not read it", "disk on fire"]);
        // The text format is the same messages joined.
        let mut all = vec![report.message.clone()];
        all.extend(report.chain);
        assert_eq!(all.join(": "), format!("{err:#}"));
    }

    #[test]
    fn a_report_of_an_untagged_error_is_other_with_no_chain() {
        let report = ErrorReport::new(&anyhow!("boom"));
        assert_eq!(report.code, 8);
        assert_eq!(report.kind, "other");
        assert_eq!(report.message, "boom");
        assert!(report.chain.is_empty());
    }

    #[test]
    fn a_usage_report_has_no_chain() {
        let report = ErrorReport::usage("bad flag");
        assert_eq!((report.code, report.kind), (2, "usage"));
        assert!(report.chain.is_empty());
    }

    /// One line, fields in the documented order, quotes and newlines escaped.
    #[test]
    fn json_is_one_line_in_field_order() {
        let err = ExitKind::Gate.error("it said \"no\"\nreally");
        let json = ErrorReport::new(&err).to_json();
        assert!(!json.contains('\n'), "{json}");
        assert_eq!(
            json,
            r#"{"level":"error","code":1,"kind":"gate","message":"it said \"no\"\nreally","chain":[]}"#
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["message"], "it said \"no\"\nreally");
    }

    fn failures() -> Vec<GateFailure> {
        vec![
            GateFailure::FailUnderPatch {
                threshold: 80.0,
                measured: 66.5,
            },
            GateFailure::FailUnderLines {
                threshold: 90.0,
                measured: Some(40.0),
            },
            GateFailure::FailUnderLines {
                threshold: 50.0,
                measured: None,
            },
            GateFailure::FailOnUnmeasured {
                files: vec!["a.rs".into(), "b.rs".into()],
            },
        ]
    }

    /// The default text is a contract too: these are the messages in the docs.
    #[test]
    fn a_gate_failure_displays_as_its_message() {
        let messages: Vec<String> = failures().iter().map(ToString::to_string).collect();
        assert_eq!(
            messages,
            [
                "patch coverage 66.50% is below the --fail-under-patch threshold of 80.00%",
                "line coverage 40.00% is below the --fail-under-lines threshold of 90.00%",
                "the report has no executable lines, so the --fail-under-lines threshold of 50.00% cannot be met",
                "touched files absent from every coverage report (--fail-on-unmeasured / diff.require-measured): a.rs, b.rs",
            ]
        );
    }

    #[test]
    fn a_gate_error_joins_the_messages_and_keeps_the_data() {
        let err = gate_error(failures());
        assert_eq!(code(&err), 1);
        let text = format!("{err:#}");
        assert_eq!(text.matches("; ").count(), 3, "{text}");
        assert!(text.starts_with("patch coverage 66.50%"), "{text}");
        assert_eq!(tag_of(&err).unwrap().gates(), failures());
    }

    /// Fields in the documented order; the numbers are exactly as measured.
    #[test]
    fn a_gate_report_lists_the_gates() {
        let report = ErrorReport::new(&gate_error(failures()));
        let json: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
        assert_eq!(
            json["gates"],
            serde_json::json!([
                {"gate": "fail-under-patch", "threshold": 80.0, "measured": 66.5},
                {"gate": "fail-under-lines", "threshold": 90.0, "measured": 40.0},
                {"gate": "fail-under-lines", "threshold": 50.0, "measured": null},
                {"gate": "fail-on-unmeasured", "files": ["a.rs", "b.rs"]},
            ])
        );
        let line = report.to_json();
        assert!(
            line.contains(
                r#""gates":[{"gate":"fail-under-patch","threshold":80.0,"measured":66.5}"#
            ),
            "{line}"
        );
        assert!(!line.contains('\n'), "{line}");
    }

    /// A value just under the threshold must not round up to it, or comparing the
    /// two would say the gate passed.
    #[test]
    fn measured_values_are_not_rounded() {
        let err = gate_error(vec![GateFailure::FailUnderPatch {
            threshold: 80.0,
            measured: 79.996,
        }]);
        let json: serde_json::Value =
            serde_json::from_str(&ErrorReport::new(&err).to_json()).unwrap();
        assert_eq!(json["gates"][0]["measured"], 79.996);
        assert!(err.to_string().contains("80.00% is below"), "{err}");
    }

    /// `gates` is for a gate that carries data and nothing else.
    #[test]
    fn only_a_gate_with_data_has_gates() {
        for err in [
            ExitKind::Gate.error("no data"),
            fail(ExitKind::Report),
            anyhow!("boom"),
        ] {
            let report = ErrorReport::new(&err);
            assert_eq!(report.gates, None);
            assert!(!report.to_json().contains("gates"));
        }
        assert_eq!(ErrorReport::usage("bad flag").gates, None);
    }

    /// Context added after the gate error does not hide its data.
    #[test]
    fn the_gates_survive_added_context() {
        let err = Err::<(), _>(gate_error(failures()))
            .context("outer")
            .classify(ExitKind::Other)
            .unwrap_err();
        let report = ErrorReport::new(&err);
        assert_eq!(report.kind, "gate");
        assert_eq!(report.gates, Some(failures()));
    }

    #[test]
    fn an_untagged_error_is_other() {
        assert_eq!(code(&anyhow!("boom")), ExitKind::Other.code());
    }

    /// Context added after the tag (as `?` through callers does) hides nothing.
    #[test]
    fn the_tag_survives_added_context() {
        let err = Err::<(), _>(fail(ExitKind::Git))
            .context("while resolving")
            .unwrap_err();
        assert_eq!(code(&err), ExitKind::Git.code());
    }

    #[test]
    fn the_innermost_class_wins() {
        let err = Err::<(), _>(fail(ExitKind::Report))
            .classify(ExitKind::Other)
            .unwrap_err();
        assert_eq!(code(&err), ExitKind::Report.code());
        let err = Err::<(), _>(fail(ExitKind::Report))
            .context("outer")
            .classify(ExitKind::Config)
            .unwrap_err();
        assert_eq!(code(&err), ExitKind::Report.code());
    }

    /// The tag must not change a character of what is printed.
    #[test]
    fn the_message_and_chain_are_unchanged() {
        let build = || {
            Err::<(), _>(std::io::Error::other("disk on fire"))
                .context("could not read it")
                .context("outer")
                .unwrap_err()
        };
        let plain = build();
        let tagged = Err::<(), _>(build())
            .classify(ExitKind::Report)
            .unwrap_err();
        assert_eq!(format!("{plain:#}"), format!("{tagged:#}"));
        assert_eq!(format!("{plain}"), format!("{tagged}"));
        assert_eq!(plain.chain().count(), tagged.chain().count());
    }

    /// A plain `std` error converts as well as an `anyhow` one.
    #[test]
    fn a_std_error_can_be_classified() {
        let err = Err::<(), _>(std::io::Error::other("x"))
            .classify(ExitKind::Other)
            .unwrap_err();
        assert_eq!(code(&err), ExitKind::Other.code());
        assert_eq!(err.to_string(), "x");
    }

    /// What survives when the full report cannot be serialized.
    #[test]
    fn fallback_keeps_level_code_kind_and_message() {
        let err = ExitKind::Config.error("bad config");
        assert_eq!(
            ErrorReport::new(&err).fallback().to_string(),
            r#"{"code":5,"kind":"config","level":"error","message":"bad config"}"#
        );
        assert_eq!(
            ErrorReport::usage("bad flag").fallback().to_string(),
            r#"{"code":2,"kind":"usage","level":"error","message":"bad flag"}"#
        );
    }
}
