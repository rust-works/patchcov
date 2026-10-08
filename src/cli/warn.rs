//! Warnings on stderr, in the format `--error-format` selects.
//!
//! Every non-fatal problem patchcov reports goes through [`warn`], so a wrapper
//! that asked for `--error-format json` gets one JSON object per warning instead
//! of a `warning: ...` line. The object is described in
//! `docs/reference.md#error-output`.

use std::sync::OnceLock;

use serde::Serialize;

use super::ErrorFormat;

/// The format warnings are printed in, set once by `main`.
static FORMAT: OnceLock<ErrorFormat> = OnceLock::new();

/// Sets the format [`warn`] prints in. Only the first call has an effect; without
/// one, warnings are text.
pub fn set_format(format: ErrorFormat) {
    let _ = FORMAT.set(format);
}

/// A class of warning, named by `kind` in the JSON warning object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WarningKind {
    /// A deprecated flag was used.
    Deprecated,
    /// A report's paths match no tracked file, and `--allow-path-mismatch` let the
    /// run continue.
    PathMismatch,
    /// A shard was measured under a different workspace root (`merge`).
    ShardRoot,
    /// The globs of `lint-markers` matched no tracked file.
    GlobNoMatch,
}

impl WarningKind {
    /// The stable name of this class in the JSON warning object, listed in
    /// `docs/reference.md#error-output`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Deprecated => "deprecated",
            Self::PathMismatch => "path-mismatch",
            Self::ShardRoot => "shard-root",
            Self::GlobNoMatch => "glob-no-match",
        }
    }
}

/// The JSON warning object, with its fields in the documented order.
#[derive(Serialize)]
struct WarningReport<'a> {
    level: &'static str,
    kind: &'static str,
    message: &'a str,
}

/// Prints a warning to stderr, as a `warning: ...` line or a JSON object.
pub fn warn(kind: WarningKind, message: impl AsRef<str>) {
    eprintln!(
        "{}",
        render(
            FORMAT.get().copied().unwrap_or_default(),
            kind,
            message.as_ref()
        )
    );
}

/// The line `warn` prints.
fn render(format: ErrorFormat, kind: WarningKind, message: &str) -> String {
    match format {
        ErrorFormat::Text => format!("warning: {message}"),
        ErrorFormat::Json => serde_json::to_string(&WarningReport {
            level: "warning",
            kind: kind.name(),
            message,
        })
        // Plain strings always serialize.
        .unwrap_or_default(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn text_is_the_warning_line() {
        let line = render(ErrorFormat::Text, WarningKind::Deprecated, "x is old");
        assert_eq!(line, "warning: x is old");
    }

    /// One line, fields in the documented order, quotes and newlines escaped.
    #[test]
    fn json_is_one_line_in_field_order() {
        let line = render(
            ErrorFormat::Json,
            WarningKind::PathMismatch,
            "it said \"no\"\nreally",
        );
        assert!(!line.contains('\n'), "{line}");
        assert_eq!(
            line,
            r#"{"level":"warning","kind":"path-mismatch","message":"it said \"no\"\nreally"}"#
        );
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["message"], "it said \"no\"\nreally");
    }

    /// The names are a contract with wrappers; this is the table in the docs.
    #[test]
    fn names_are_stable() {
        let expected = [
            (WarningKind::Deprecated, "deprecated"),
            (WarningKind::PathMismatch, "path-mismatch"),
            (WarningKind::ShardRoot, "shard-root"),
            (WarningKind::GlobNoMatch, "glob-no-match"),
        ];
        for (kind, name) in expected {
            assert_eq!(kind.name(), name, "{kind:?}");
        }
    }
}
